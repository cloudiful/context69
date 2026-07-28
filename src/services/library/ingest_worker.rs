use std::time::Duration;

use anyhow::Result;
use tracing::{info, warn};
use uuid::Uuid;

use super::dependency_runtime::{
    ingest_lease_ttl_secs, log_dependency_transition, probe_lease_ttl_secs,
};
use super::{IngestClaimOutcome, LibraryDependency, LibraryService};
use crate::library_store::PendingIngestDependencies;

impl LibraryService {
    pub(super) async fn run_ingest_worker(&self, worker_id: usize) {
        loop {
            if let Err(error) = self.refresh_dependency_configuration().await {
                warn!(worker_id, %error, "failed to refresh library dependency configuration");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            if let Err(error) = self.store.recover_expired_ingest_jobs().await {
                warn!(worker_id, %error, "failed to recover expired library ingest leases");
            }

            let lease_token = Uuid::new_v4();
            let pending_dependencies = match self
                .store
                .pending_ingest_dependencies(self.storage.backend())
                .await
            {
                Ok(dependencies) => dependencies,
                Err(error) => {
                    warn!(worker_id, %error, "failed to inspect pending library ingest dependencies");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let mut reserved = Vec::new();
            for dependency in probe_dependencies(&pending_dependencies) {
                match self
                    .store
                    .reserve_dependency_probe(
                        dependency.as_str(),
                        lease_token,
                        probe_lease_ttl_secs(),
                    )
                    .await
                {
                    Ok(Some(transition)) => {
                        log_dependency_transition(&transition);
                        reserved.push(dependency);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        warn!(worker_id, dependency = dependency.as_str(), %error, "failed to reserve library dependency probe");
                    }
                }
            }

            let claim = match self
                .store
                .claim_next_ingest_job(lease_token, ingest_lease_ttl_secs(), self.storage.backend())
                .await
            {
                Ok(Some(claim)) => claim,
                Ok(None) => {
                    abandon_dependency_probes(self, &reserved, lease_token).await;
                    self.wait_for_ingest_work().await;
                    continue;
                }
                Err(error) => {
                    abandon_dependency_probes(self, &reserved, lease_token).await;
                    warn!(worker_id, %error, "failed to claim library ingest job");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            let used = used_dependencies(&claim);
            let active_probes = reserved
                .iter()
                .copied()
                .filter(|dependency| used.contains(dependency))
                .collect::<Vec<_>>();
            for dependency in reserved {
                if !used.contains(&dependency) {
                    if let Ok(Some(transition)) = self
                        .store
                        .abandon_dependency_probe(dependency.as_str(), lease_token)
                        .await
                    {
                        log_dependency_transition(&transition);
                    }
                }
            }

            let queue_wait_ms = chrono::Utc::now()
                .signed_duration_since(claim.created_at)
                .num_milliseconds()
                .max(0);
            info!(
                worker_id,
                job_id = %claim.job_id,
                queue_wait_ms,
                "library ingest job claimed"
            );

            let result = self.run_ingest_claim(claim, active_probes).await;
            match result {
                Ok(IngestClaimOutcome::Succeeded) => {
                    release_dependency_probes(self, &used, lease_token).await;
                }
                Ok(IngestClaimOutcome::Requeued) => {
                    // A dependency failure has already opened its gate. If the job was
                    // requeued for an unrelated transient error, close the unused probe
                    // instead of penalizing a healthy dependency.
                    release_dependency_probes(self, &used, lease_token).await;
                }
                Err(error) => {
                    // Permanent task errors are scoped to the document. They must not leave a
                    // healthy dependency open merely because the task was its half-open probe.
                    release_dependency_probes(self, &used, lease_token).await;
                    warn!(worker_id, %error, "library ingest job finished with a permanent failure");
                }
            }
        }
    }
}

fn probe_dependencies(pending: &PendingIngestDependencies) -> Vec<LibraryDependency> {
    let mut dependencies = Vec::new();
    if pending.has_pending {
        dependencies.push(LibraryDependency::EmbeddingVector);
    }
    if pending.requires_docling {
        dependencies.push(LibraryDependency::Docling);
    }
    if pending.requires_s3 {
        dependencies.push(LibraryDependency::S3);
    }
    dependencies
}

fn used_dependencies(claim: &crate::library_store::IngestClaim) -> Vec<LibraryDependency> {
    let mut dependencies = vec![LibraryDependency::EmbeddingVector];
    if claim.requires_docling {
        dependencies.push(LibraryDependency::Docling);
    }
    if claim.storage_backend == "s3" {
        dependencies.push(LibraryDependency::S3);
    }
    dependencies
}

async fn abandon_dependency_probes(
    service: &LibraryService,
    dependencies: &[LibraryDependency],
    lease_token: Uuid,
) {
    for dependency in dependencies {
        if let Ok(Some(transition)) = service
            .store
            .abandon_dependency_probe(dependency.as_str(), lease_token)
            .await
        {
            log_dependency_transition(&transition);
        }
    }
}

async fn release_dependency_probes(
    service: &LibraryService,
    dependencies: &[LibraryDependency],
    lease_token: Uuid,
) {
    for dependency in dependencies {
        if let Ok(Some(transition)) = service
            .store
            .release_dependency_probe(dependency.as_str(), lease_token)
            .await
        {
            log_dependency_transition(&transition);
        }
    }
}
