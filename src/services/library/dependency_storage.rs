use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use tokio::time::timeout;
use uuid::Uuid;

use super::dependency_errors::{
    is_configuration_error, is_s3_attempt_retryable, is_s3_transient_error,
};
use super::{LibraryDependency, LibraryService};

const S3_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const S3_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
const S3_RETRY_LIMIT: usize = 2;

impl LibraryService {
    async fn ensure_active_storage_ready_for(&self, lease_token: Option<Uuid>) -> Result<()> {
        if self.storage.backend() != "s3" {
            return Ok(());
        }
        let gate = self
            .store
            .list_dependency_gates()
            .await?
            .into_iter()
            .find(|gate| gate.dependency_key == LibraryDependency::S3.as_str())
            .ok_or_else(|| anyhow!("s3 dependency unavailable: dependency gate is missing"))?;
        let probe_owned = gate.state == "half_open"
            && lease_token.is_some()
            && gate.probe_lease_token == lease_token;
        if gate.state != "closed" && !probe_owned {
            return Err(anyhow!(
                "s3 dependency unavailable: state={}{}",
                gate.state,
                gate.last_error
                    .as_deref()
                    .map(|error| format!("; last_error={error}"))
                    .unwrap_or_default()
            ));
        }
        Ok(())
    }

    pub(super) async fn write_active_storage(&self, key: &str, bytes: Bytes) -> Result<()> {
        self.write_active_storage_with_lease(key, bytes, None).await
    }

    pub(super) async fn write_active_storage_for_lease(
        &self,
        key: &str,
        bytes: Bytes,
        lease_token: Uuid,
    ) -> Result<()> {
        self.write_active_storage_with_lease(key, bytes, Some(lease_token))
            .await
    }

    async fn write_active_storage_with_lease(
        &self,
        key: &str,
        bytes: Bytes,
        lease_token: Option<Uuid>,
    ) -> Result<()> {
        self.ensure_active_storage_ready_for(lease_token).await?;
        match self.storage.write(key, bytes).await {
            Ok(()) => {
                if let Some(lease_token) = lease_token {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                }
                Ok(())
            }
            Err(error)
                if self.storage.backend() == "s3"
                    && (is_s3_transient_error(&error) || is_configuration_error(&error)) =>
            {
                self.note_dependency_failure_with_lease(
                    LibraryDependency::S3,
                    lease_token.unwrap_or_else(Uuid::nil),
                    &error,
                )
                .await;
                Err(error.context("s3 dependency unavailable"))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn read_active_storage(&self, key: &str) -> Result<Option<Bytes>> {
        self.read_active_storage_with_lease(key, None).await
    }

    pub(super) async fn read_active_storage_for_lease(
        &self,
        key: &str,
        lease_token: Uuid,
    ) -> Result<Option<Bytes>> {
        self.read_active_storage_with_lease(key, Some(lease_token))
            .await
    }

    async fn read_active_storage_with_lease(
        &self,
        key: &str,
        lease_token: Option<Uuid>,
    ) -> Result<Option<Bytes>> {
        self.ensure_active_storage_ready_for(lease_token).await?;
        match self.storage.read(key).await {
            Ok(bytes) => {
                if let Some(lease_token) = lease_token {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                }
                Ok(bytes)
            }
            Err(error)
                if self.storage.backend() == "s3"
                    && (is_s3_transient_error(&error) || is_configuration_error(&error)) =>
            {
                self.note_dependency_failure_with_lease(
                    LibraryDependency::S3,
                    lease_token.unwrap_or_else(Uuid::nil),
                    &error,
                )
                .await;
                Err(error.context("s3 dependency unavailable"))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn exists_active_storage(&self, key: &str) -> Result<bool> {
        self.exists_active_storage_for_lease_context(key, None)
            .await
    }

    pub(super) async fn exists_active_storage_for_lease(
        &self,
        key: &str,
        lease_token: Uuid,
    ) -> Result<bool> {
        self.exists_active_storage_for_lease_context(key, Some(lease_token))
            .await
    }

    async fn exists_active_storage_for_lease_context(
        &self,
        key: &str,
        lease_token: Option<Uuid>,
    ) -> Result<bool> {
        self.ensure_active_storage_ready_for(lease_token).await?;
        match self.storage.exists(key).await {
            Ok(exists) => {
                if let Some(lease_token) = lease_token {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                }
                Ok(exists)
            }
            Err(error)
                if self.storage.backend() == "s3"
                    && (is_s3_transient_error(&error) || is_configuration_error(&error)) =>
            {
                self.note_dependency_failure_with_lease(
                    LibraryDependency::S3,
                    lease_token.unwrap_or_else(Uuid::nil),
                    &error,
                )
                .await;
                Err(error.context("s3 dependency unavailable"))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn delete_active_storage(&self, key: &str) -> Result<()> {
        self.delete_active_storage_with_lease(key, None).await
    }

    pub(super) async fn delete_active_storage_for_lease(
        &self,
        key: &str,
        lease_token: Uuid,
    ) -> Result<()> {
        self.delete_active_storage_with_lease(key, Some(lease_token))
            .await
    }

    async fn delete_active_storage_with_lease(
        &self,
        key: &str,
        lease_token: Option<Uuid>,
    ) -> Result<()> {
        self.ensure_active_storage_ready_for(lease_token).await?;
        match self.storage.delete(key).await {
            Ok(()) => {
                if let Some(lease_token) = lease_token {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                }
                Ok(())
            }
            Err(error)
                if self.storage.backend() == "s3"
                    && (is_s3_transient_error(&error) || is_configuration_error(&error)) =>
            {
                self.note_dependency_failure_with_lease(
                    LibraryDependency::S3,
                    lease_token.unwrap_or_else(Uuid::nil),
                    &error,
                )
                .await;
                Err(error.context("s3 dependency unavailable"))
            }
            Err(error) => Err(error),
        }
    }
}

pub(super) async fn bounded_s3_operation<T, F, Fut>(operation: &str, mut action: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, opendal::Error>>,
{
    let result = timeout(S3_OPERATION_TIMEOUT, async {
        let mut last_error = None;
        let mut retryable_error_seen = false;
        for attempt in 0..=S3_RETRY_LIMIT {
            let result = timeout(S3_ATTEMPT_TIMEOUT, action()).await;
            match result {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(error)) => {
                    if !is_s3_attempt_retryable(&error) {
                        return Err(anyhow!(
                            "s3 operation {operation} failed: kind={:?}: {error}",
                            error.kind()
                        ));
                    }
                    retryable_error_seen = true;
                    last_error = Some(format!("kind={:?}: {error}", error.kind()));
                }
                Err(_) => {
                    retryable_error_seen = true;
                    last_error = Some(format!(
                        "{operation} attempt timed out after {}s",
                        S3_ATTEMPT_TIMEOUT.as_secs()
                    ));
                }
            }
            if attempt < S3_RETRY_LIMIT {
                tokio::time::sleep(Duration::from_millis(200 * (attempt as u64 + 1))).await;
            }
        }

        Err(anyhow!(
            "s3 operation {operation} failed after {} attempts: {}{}",
            S3_RETRY_LIMIT + 1,
            last_error.unwrap_or_else(|| "unknown error".to_string()),
            if retryable_error_seen {
                "; s3 transient transport failure"
            } else {
                ""
            }
        ))
    })
    .await;

    result.unwrap_or_else(|_| {
        Err(anyhow!(
            "s3 operation {operation} timed out after {}s",
            S3_OPERATION_TIMEOUT.as_secs()
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::bounded_s3_operation;

    #[tokio::test]
    async fn does_not_retry_permanent_s3_errors() {
        let calls = Arc::new(AtomicUsize::new(0));
        let result = bounded_s3_operation("write", {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::Relaxed);
                async {
                    Err::<(), _>(opendal::Error::new(
                        opendal::ErrorKind::AlreadyExists,
                        "object already exists",
                    ))
                }
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn retries_temporary_s3_errors_within_the_attempt_budget() {
        let calls = Arc::new(AtomicUsize::new(0));
        let result = bounded_s3_operation("write", {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::Relaxed);
                async {
                    Err::<(), _>(
                        opendal::Error::new(opendal::ErrorKind::Unexpected, "upstream timeout")
                            .set_temporary(),
                    )
                }
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }
}
