use super::*;
use crate::contracts::ImportLibraryFileFromUrlRequest;
use anyhow::Result;
use bytes::Bytes;
use rate_limiter::RateLimiter;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

use super::streaming::{ByteSink, transfer};

/// What a streamed materialization must fetch from the network.
struct RemoteSourceSpec<'a> {
    url: &'a str,
    filename: Option<&'a str>,
    media_type: Option<&'a str>,
    trusted_proxy_enabled: bool,
    limiter: Arc<dyn RateLimiter>,
}

/// A [`ByteSink`] decorator that records whether the object store failed a write.
///
/// Attribution for the S3 dependency gate is by provenance, never by error text:
/// a remote download error embeds the user-supplied URL and the upstream status
/// code, and `401`/`403`/`5xx`/`429` classify as storage configuration or
/// transient failures, so offering them to the gate would let an ordinary remote
/// auth error latch the platform-wide gate. Only a write that failed inside this
/// sink is a storage event.
struct StorageFailureSink<'a> {
    inner: &'a mut dyn ByteSink,
    storage_failed: bool,
}

impl<'a> StorageFailureSink<'a> {
    fn new(inner: &'a mut dyn ByteSink) -> Self {
        Self {
            inner,
            storage_failed: false,
        }
    }
}

#[async_trait::async_trait]
impl ByteSink for StorageFailureSink<'_> {
    async fn write_chunk(&mut self, chunk: Bytes) -> Result<()> {
        match self.inner.write_chunk(chunk).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.storage_failed = true;
                Err(error)
            }
        }
    }
}

/// What the streamed transfer learned, plus the resolved source identity the
/// task payload keeps for a re-claim.
struct StreamedSourceBody {
    source_url: String,
    filename: String,
    media_type: String,
    sha256: String,
    size_bytes: i64,
}

/// Result of streaming one remote URL into a staged, content-addressed object.
///
/// `source_url`/`filename`/`media_type` are the values resolved from the
/// response (redirect target, `Content-Disposition`, `Content-Type`), which a
/// re-claim cannot re-derive without re-downloading, so they are what the task
/// payload keeps alongside `input_storage_object_id`.
pub(crate) struct MaterializedUrlSource {
    pub source_url: String,
    pub filename: String,
    pub media_type: String,
    pub sha256: String,
    pub object_id: Uuid,
}

/// Whether a failed streamed materialization counts as a storage failure.
///
/// `storage_write_failed` is [`StorageFailureSink`]'s provenance flag: only a
/// write the object store rejected is a storage event. A remote download, SSRF
/// or URL-validation, size-limit, or rate-limiter failure is never attributed,
/// because those messages are built from user-supplied input (the URL and the
/// upstream HTTP status) and the shared classifiers read `401`/`403` as
/// configuration errors and `429`/`5xx` as transient ones.
fn attributes_stream_failure(
    backend: &str,
    storage_write_failed: bool,
    error: &anyhow::Error,
) -> bool {
    storage_write_failed && dependency_storage::is_storage_gate_failure(backend, error)
}

impl LibraryService {
    /// Stream a URL source into the object store and register the bytes as
    /// this task input's staged object.
    ///
    /// The bytes never enter the task payload: the item records the resolved
    /// source metadata plus `input_storage_object_id`, and the storage stage
    /// consumes them through the existing FileBatch staged-object path
    /// (deduplication, finalization, and release guards included).
    pub(crate) async fn materialize_url_source_for_task(
        &self,
        group_id: i64,
        request: &ImportLibraryFileFromUrlRequest,
        lease_token: Uuid,
    ) -> Result<MaterializedUrlSource> {
        let url = remote_download::normalize_url(&request.url)?;
        if let Some(folder_id) = request.folder_id {
            self.store
                .get_folder_in_project(group_id, folder_id)
                .await?
                .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))
                .map_err(anyhow::Error::from)?;
        }

        let trusted_proxy_enabled = self.settings.trusted_proxy_enabled().await?;
        let limiter = self
            .url_import_runtime
            .limiter()
            .ok_or_else(|| DomainError::unavailable("URL import rate limiter is unavailable"))
            .map_err(anyhow::Error::from)?;
        let temp_key = object_storage::staging_object_key(group_id, Uuid::new_v4());
        let spec = RemoteSourceSpec {
            url: url.as_str(),
            filename: request.filename.as_deref(),
            media_type: request.media_type.as_deref(),
            trusted_proxy_enabled,
            limiter,
        };

        let streamed = match self
            .stream_remote_source(&temp_key, lease_token, spec)
            .await
        {
            Ok(streamed) => streamed,
            Err(error) => {
                self.discard_staging_key(&temp_key, lease_token).await;
                return Err(error);
            }
        };
        let object_id = match self
            .finalize_streamed_task_input(
                group_id,
                &temp_key,
                &streamed.sha256,
                streamed.size_bytes,
                lease_token,
            )
            .await
        {
            Ok(object_id) => object_id,
            Err(error) => {
                self.discard_staging_key(&temp_key, lease_token).await;
                return Err(error);
            }
        };
        // The staged bytes now live under the content-addressed key; the
        // temporary copy is never part of the catalog.
        self.discard_staging_key(&temp_key, lease_token).await;
        info!(
            group_id,
            object_id = %object_id,
            size_bytes = streamed.size_bytes,
            sha256 = %streamed.sha256,
            "streamed URL source into the object store"
        );
        Ok(MaterializedUrlSource {
            source_url: streamed.source_url,
            filename: streamed.filename,
            media_type: streamed.media_type,
            sha256: streamed.sha256,
            object_id,
        })
    }

    /// Stream one remote body into `temp_key` under the S3 dependency gate.
    ///
    /// The caller owns `temp_key` and deletes it on both the success and the
    /// failure path: a failure here aborts the incomplete object, so nothing
    /// half-written is ever finalized.
    ///
    /// Only object-store failures reach the S3 gate. A remote download, SSRF,
    /// limiter, or size-limit failure is reported as-is and never attributed to
    /// the storage dependency, however its message reads.
    async fn stream_remote_source(
        &self,
        temp_key: &str,
        lease_token: Uuid,
        spec: RemoteSourceSpec<'_>,
    ) -> Result<StreamedSourceBody> {
        self.ensure_active_storage_ready_for(Some(lease_token))
            .await?;
        let mut writer = match self.storage.open_stream_writer(temp_key).await {
            Ok(writer) => writer,
            Err(error) => {
                self.note_storage_error(&error, Some(lease_token)).await;
                return Err(error);
            }
        };
        let mut sink = StorageFailureSink::new(&mut writer);
        let attempt = async {
            let mut source = remote_download::open(
                spec.url,
                spec.filename,
                spec.media_type,
                self.max_upload_size_bytes,
                spec.trusted_proxy_enabled,
                spec.limiter.as_ref(),
            )
            .await?;
            let digest = transfer(&mut source, &mut sink, self.max_upload_size_bytes).await?;
            Ok(StreamedSourceBody {
                source_url: source.url.to_string(),
                filename: source.filename.clone(),
                media_type: source.media_type.clone(),
                sha256: digest.sha256,
                size_bytes: digest.size_bytes,
            })
        }
        .await;
        // The transfer stops at the first failure, so a sink write that failed
        // is necessarily the error carried here; a remote failure leaves the
        // flag false and its error never reaches the gate.
        let storage_failed = sink.storage_failed;
        match attempt {
            Ok(body) => match writer.close().await {
                Ok(()) => {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                    Ok(body)
                }
                Err(error) => {
                    self.note_storage_error(&error, Some(lease_token)).await;
                    Err(error)
                }
            },
            Err(error) => {
                writer.abort(temp_key).await;
                if attributes_stream_failure(self.storage.backend(), storage_failed, &error) {
                    self.note_storage_error(&error, Some(lease_token)).await;
                }
                Err(error)
            }
        }
    }

    /// Remove an in-flight staging key, best effort.
    ///
    /// The key never has a catalog row, so a lost delete only leaves bytes for
    /// the byte-store lifecycle policy to reclaim; it must never fail the
    /// materialization that just succeeded.
    async fn discard_staging_key(&self, temp_key: &str, lease_token: Uuid) {
        if let Err(error) = self
            .delete_active_storage_for_lease(temp_key, lease_token)
            .await
            && !matches!(
                crate::domain_errors::find_domain_error(&error),
                Some(DomainError::NotFound(_))
            )
        {
            tracing::warn!(
                key = temp_key,
                %error,
                "failed to delete streamed staging object"
            );
        }
    }
}

#[cfg(test)]
mod tests;
