use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use super::dependency_runtime::{log_dependency_transition, probe_lease_ttl_secs};
use super::url_import_helpers::url_import_uses_dependency;
use super::url_import_runtime::{
    URL_IMPORT_HEARTBEAT_INTERVAL, URL_IMPORT_LEASE_TTL_SECS, UrlImportRuntime,
};
use super::*;

pub(super) const URL_IMPORT_PENDING_REQUEUE_SECS: i64 = 5;
pub(super) const URL_IMPORT_TRANSIENT_REQUEUE_SECS: i64 = 30;

pub(super) enum UrlImportProgress {
    Succeeded,
    WaitingForIngest,
    Requeue,
}

pub(super) enum UrlImportOutcome {
    Succeeded,
    Requeued,
}

impl LibraryService {
    pub(super) async fn run_url_import_worker(&self, runtime: UrlImportRuntime, worker_id: usize) {
        loop {
            if let Err(error) = self.store.recover_expired_url_import_jobs().await {
                warn!(worker_id, %error, "failed to recover expired URL import leases");
            }
            let lease_token = Uuid::new_v4();
            let reserved = self.reserve_url_dependency_probes(lease_token).await;
            let job = match self
                .store
                .claim_next_url_import_job(
                    lease_token,
                    URL_IMPORT_LEASE_TTL_SECS,
                    self.storage.backend(),
                )
                .await
            {
                Ok(Some(job)) => job,
                Ok(None) => {
                    self.abandon_dependency_probes(&reserved, lease_token).await;
                    tokio::select! {
                        _ = runtime.wait_for_work() => {}
                        _ = tokio::time::sleep(UrlImportRuntime::poll_interval()) => {}
                    }
                    continue;
                }
                Err(error) => {
                    self.abandon_dependency_probes(&reserved, lease_token).await;
                    warn!(worker_id, %error, "failed to claim URL import job");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            let used_probes = reserved
                .iter()
                .copied()
                .filter(|dependency| {
                    url_import_uses_dependency(*dependency, &job, self.storage.backend())
                })
                .collect::<Vec<_>>();
            for dependency in &reserved {
                if !used_probes.contains(dependency) {
                    if let Ok(Some(transition)) = self
                        .store
                        .abandon_dependency_probe(dependency.as_str(), lease_token)
                        .await
                    {
                        log_dependency_transition(&transition);
                    }
                }
            }

            let queue_wait_ms = Utc::now()
                .signed_duration_since(job.created_at)
                .num_milliseconds()
                .max(0);
            info!(
                worker_id,
                job_id = %job.id,
                queue_wait_ms,
                "URL import job claimed"
            );

            let heartbeat =
                self.spawn_url_import_heartbeat(job.id, lease_token, used_probes.clone());
            let result = self.run_url_import(&job, lease_token).await;
            heartbeat.abort();
            match result {
                Ok(UrlImportOutcome::Succeeded) => {
                    self.release_dependency_probes(&used_probes, lease_token)
                        .await;
                }
                Ok(UrlImportOutcome::Requeued) => {
                    self.release_dependency_probes(&used_probes, lease_token)
                        .await;
                }
                Err(error) => {
                    self.release_dependency_probes(&used_probes, lease_token)
                        .await;
                    warn!(worker_id, job_id = %job.id, %error, "URL import failed");
                }
            }
        }
    }

    async fn reserve_url_dependency_probes(&self, lease_token: Uuid) -> Vec<LibraryDependency> {
        let mut reserved = Vec::new();
        for dependency in super::url_import_helpers::url_probe_dependencies(self.storage.backend())
        {
            match self
                .store
                .reserve_dependency_probe(dependency.as_str(), lease_token, probe_lease_ttl_secs())
                .await
            {
                Ok(Some(transition)) => {
                    log_dependency_transition(&transition);
                    reserved.push(dependency);
                }
                Ok(None) => {}
                Err(error) => {
                    warn!(dependency = dependency.as_str(), %error, "failed to reserve URL import dependency probe");
                }
            }
        }
        reserved
    }

    async fn abandon_dependency_probes(
        &self,
        dependencies: &[LibraryDependency],
        lease_token: Uuid,
    ) {
        for dependency in dependencies {
            if let Ok(Some(transition)) = self
                .store
                .abandon_dependency_probe(dependency.as_str(), lease_token)
                .await
            {
                log_dependency_transition(&transition);
            }
        }
    }

    async fn release_dependency_probes(
        &self,
        dependencies: &[LibraryDependency],
        lease_token: Uuid,
    ) {
        for dependency in dependencies {
            if let Ok(Some(transition)) = self
                .store
                .release_dependency_probe(dependency.as_str(), lease_token)
                .await
            {
                log_dependency_transition(&transition);
            }
        }
    }

    fn spawn_url_import_heartbeat(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        probe_dependencies: Vec<LibraryDependency>,
    ) -> JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(URL_IMPORT_HEARTBEAT_INTERVAL);
            loop {
                interval.tick().await;
                match service
                    .store
                    .heartbeat_url_import_job(job_id, lease_token, URL_IMPORT_LEASE_TTL_SECS)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        warn!(%job_id, %error, "failed to heartbeat URL import job");
                    }
                }
                for dependency in &probe_dependencies {
                    if let Err(error) = service
                        .store
                        .renew_dependency_probe(
                            dependency.as_str(),
                            lease_token,
                            probe_lease_ttl_secs(),
                        )
                        .await
                    {
                        warn!(
                            %job_id,
                            dependency = dependency.as_str(),
                            %error,
                            "failed to heartbeat URL import dependency probe"
                        );
                    }
                }
            }
        })
    }
}
