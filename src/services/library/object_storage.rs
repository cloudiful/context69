use std::{path::Path, time::SystemTime};

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::TryStreamExt;
use opendal::{Operator, services};
use uuid::Uuid;

use crate::config::FileLibraryConfig;
use crate::domain_errors::DomainError;

use super::dependency_runtime::bounded_s3_operation;
use super::dependency_storage::bounded_s3_attempt;

#[derive(Clone)]
pub(crate) struct LibraryObjectStorage {
    operator: Operator,
    backend: &'static str,
}

impl LibraryObjectStorage {
    pub(super) fn from_config(config: &FileLibraryConfig) -> Result<Self> {
        if let Some(s3) = &config.s3 {
            let mut builder = services::S3::default()
                .endpoint(&s3.endpoint)
                .region(&s3.region)
                .bucket(&s3.bucket)
                .root(&s3.prefix)
                .access_key_id(&s3.access_key)
                .secret_access_key(&s3.secret_key);
            if !s3.path_style {
                builder = builder.enable_virtual_host_style();
            }
            return Ok(Self {
                operator: Operator::new(builder)?,
                backend: "s3",
            });
        }

        std::fs::create_dir_all(&config.storage_root).with_context(|| {
            DomainError::internal(format!(
                "failed to create storage root {}",
                config.storage_root.display()
            ))
        })?;
        let builder = services::Fs::default().root(path_text(&config.storage_root)?);
        Ok(Self {
            operator: Operator::new(builder)?,
            backend: "local",
        })
    }

    pub(crate) fn from_s3(config: &crate::config::S3StorageConfig) -> Result<Self> {
        let wrapper = FileLibraryConfig {
            storage_root: "./data/library".into(),
            max_upload_size_mb: 1,
            max_upload_request_size_mb: 1,
            ingest_concurrency: 1,
            url_import_concurrency: 1,
            url_import_min_interval_ms: 1000,
            trusted_proxy_enabled: false,
            s3: Some(config.clone()),
        };
        Self::from_config(&wrapper)
    }

    pub(crate) async fn check(&self) -> Result<()> {
        if self.backend == "s3" {
            bounded_s3_operation("check", || self.operator.check())
                .await
                .context(DomainError::internal("S3 connection check failed"))
        } else {
            self.operator
                .check()
                .await
                .context(DomainError::internal("storage connection check failed"))
        }
    }

    pub(super) fn backend(&self) -> &'static str {
        self.backend
    }

    pub(super) async fn write(&self, key: &str, bytes: Bytes) -> Result<()> {
        if self.backend == "s3" {
            let key = key.to_string();
            bounded_s3_operation("write", || {
                let bytes = bytes.clone();
                let key = key.clone();
                async move { self.operator.write(&key, bytes).await }
            })
            .await
            .with_context(|| {
                DomainError::internal(format!("failed to write stored object {key}"))
            })?;
        } else {
            self.operator.write(key, bytes).await.with_context(|| {
                DomainError::internal(format!("failed to write stored object {key}"))
            })?;
        }
        Ok(())
    }

    /// Open an incremental writer for streamed source materialization.
    pub(super) async fn open_stream_writer(
        &self,
        key: &str,
    ) -> Result<super::streaming::StorageStreamWriter> {
        let writer = if self.backend == "s3" {
            let key = key.to_string();
            bounded_s3_attempt("writer_open", || self.operator.writer(&key))
                .await
                .with_context(|| {
                    DomainError::internal(format!("failed to open stored object writer {key}"))
                })?
        } else {
            self.operator.writer(key).await.with_context(|| {
                DomainError::internal(format!("failed to open stored object writer {key}"))
            })?
        };
        Ok(super::streaming::StorageStreamWriter::new(
            writer,
            self.backend,
        ))
    }

    pub(super) async fn read(&self, key: &str) -> Result<Option<Bytes>> {
        let result = if self.backend == "s3" {
            let key = key.to_string();
            bounded_s3_operation("read", || {
                let key = key.clone();
                async move { self.operator.read(&key).await }
            })
            .await
        } else {
            self.operator.read(key).await.map_err(anyhow::Error::from)
        };
        match result {
            Ok(buffer) => Ok(Some(Bytes::from(buffer.to_vec()))),
            Err(error) if is_not_found_error(&error) => Ok(None),
            Err(error) => Err(error).with_context(|| {
                DomainError::internal(format!("failed to read stored object {key}"))
            }),
        }
    }

    pub(super) async fn exists(&self, key: &str) -> Result<bool> {
        if self.backend == "s3" {
            let key = key.to_string();
            bounded_s3_operation("exists", || {
                let key = key.clone();
                async move { self.operator.exists(&key).await }
            })
            .await
            .with_context(|| {
                DomainError::internal(format!("failed to inspect stored object {key}"))
            })
        } else {
            self.operator.exists(key).await.with_context(|| {
                DomainError::internal(format!("failed to inspect stored object {key}"))
            })
        }
    }

    pub(super) async fn delete(&self, key: &str) -> Result<()> {
        if self.backend == "s3" {
            let key = key.to_string();
            bounded_s3_operation("delete", || {
                let key = key.clone();
                async move { self.operator.delete(&key).await }
            })
            .await
            .with_context(|| DomainError::internal(format!("failed to delete stored object {key}")))
        } else {
            self.operator.delete(key).await.with_context(|| {
                DomainError::internal(format!("failed to delete stored object {key}"))
            })
        }
    }

    /// Copy one stored object to another key, overwriting the destination.
    pub(super) async fn copy(&self, from: &str, to: &str) -> Result<()> {
        let result = if self.backend == "s3" {
            let from_key = from.to_string();
            let to_key = to.to_string();
            bounded_s3_operation("copy", || {
                let from_key = from_key.clone();
                let to_key = to_key.clone();
                async move { self.operator.copy(&from_key, &to_key).await }
            })
            .await
        } else {
            self.operator
                .copy(from, to)
                .await
                .map_err(anyhow::Error::from)
        };
        result
            .with_context(|| {
                DomainError::internal(format!("failed to copy stored object {from} to {to}"))
            })
            .map(|_| ())
    }

    /// One bounded page of `staging/`; `start_after` continues from the last key.
    pub(super) async fn list_staging_page(
        &self,
        start_after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StagingEntry>> {
        let list = || async {
            let mut request = self
                .operator
                .lister_with(STAGING_KEY_PREFIX)
                .recursive(true)
                .limit(limit);
            if let Some(start_after) = start_after {
                request = request.start_after(start_after);
            }
            let mut lister = request.await?;
            let mut page = Vec::new();
            while page.len() < limit {
                let Some(entry) = lister.try_next().await? else {
                    break;
                };
                page.push(StagingEntry {
                    key: entry.path().to_string(),
                    size_bytes: entry.metadata().content_length(),
                    modified: entry.metadata().last_modified().map(SystemTime::from),
                });
            }
            Ok::<_, opendal::Error>(page)
        };
        if self.backend == "s3" {
            bounded_s3_operation("list", list).await
        } else {
            list().await.map_err(anyhow::Error::from)
        }
    }
}

pub(super) fn content_object_key(group_id: i64, sha256: &str) -> String {
    format!("objects/{group_id}/{sha256}")
}

/// Physical key for bytes that are still being materialized.
///
/// The content-addressed key needs the digest, which is only known once the
/// whole stream has been read, so a streamed upload lands here first and is
/// copied to [`content_object_key`] after hashing. A staging key is never
/// recorded as a catalog `object_key`; it only ever names in-flight bytes that
/// the materializer deletes on both the success and the failure path.
pub(super) fn staging_object_key(group_id: i64, token: Uuid) -> String {
    format!("staging/{group_id}/{token}")
}

/// Root prefix of every staging key (the rowless sweep lists it directly).
pub(super) const STAGING_KEY_PREFIX: &str = "staging/";

#[derive(Debug, Clone)]
pub(super) struct StagingEntry {
    pub key: String,
    pub size_bytes: u64,
    pub modified: Option<SystemTime>,
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str().with_context(|| {
        DomainError::internal(format!(
            "storage root is not valid UTF-8: {}",
            path.display()
        ))
    })
}

fn is_not_found_error(error: &anyhow::Error) -> bool {
    // Match on the structured opendal error kind first; the string fallback
    // keeps older message formats working.
    let has_not_found_kind = error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<opendal::Error>())
        .any(|error| error.kind() == opendal::ErrorKind::NotFound);
    if has_not_found_kind {
        return true;
    }
    let message = error.to_string().to_ascii_lowercase();
    message.contains("kind=notfound") || message.contains("kind=not_found")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bytes::Bytes;
    use uuid::Uuid;

    use crate::config::FileLibraryConfig;

    use super::super::streaming::ByteSink;
    use super::{LibraryObjectStorage, content_object_key, staging_object_key};

    #[tokio::test]
    async fn local_backend_round_trips_objects() {
        let root = std::env::temp_dir().join(format!("context69-storage-{}", Uuid::new_v4()));
        let storage = LibraryObjectStorage::from_config(&config(root.clone())).unwrap();
        let key = content_object_key(42, &"a".repeat(64));

        storage
            .write(&key, Bytes::from_static(b"content"))
            .await
            .unwrap();
        assert!(storage.exists(&key).await.unwrap());
        assert_eq!(
            storage.read(&key).await.unwrap(),
            Some(Bytes::from_static(b"content"))
        );
        storage.delete(&key).await.unwrap();
        assert!(!storage.exists(&key).await.unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    /// Issue 667 Phase 1: the streamed writer lands the bytes, and finalizing
    /// copies them to the content-addressed key so the temporary staging key is
    /// never what the catalog records.
    #[tokio::test]
    async fn streamed_staging_writer_finalizes_into_the_content_key() {
        let root = std::env::temp_dir().join(format!("context69-storage-{}", Uuid::new_v4()));
        let storage = LibraryObjectStorage::from_config(&config(root.clone())).unwrap();
        let temp_key = staging_object_key(42, Uuid::new_v4());
        let key = content_object_key(42, &"c".repeat(64));
        assert_ne!(temp_key, key);

        let mut writer = storage.open_stream_writer(&temp_key).await.unwrap();
        writer
            .write_chunk(Bytes::from_static(b"stream"))
            .await
            .unwrap();
        writer.write_chunk(Bytes::from_static(b"ed")).await.unwrap();
        writer.close().await.unwrap();
        assert_eq!(
            storage.read(&temp_key).await.unwrap(),
            Some(Bytes::from_static(b"streamed"))
        );

        storage.copy(&temp_key, &key).await.unwrap();
        assert_eq!(
            storage.read(&key).await.unwrap(),
            Some(Bytes::from_static(b"streamed"))
        );
        storage.delete(&temp_key).await.unwrap();
        assert!(!storage.exists(&temp_key).await.unwrap());
        assert!(storage.exists(&key).await.unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    /// A failed stream is aborted and the caller deletes the temporary key:
    /// the local filesystem backend writes in place and cannot abort, so the
    /// delete is what guarantees the failure path leaves no staging bytes.
    #[tokio::test]
    async fn aborted_streamed_write_is_reclaimed_by_the_staging_delete() {
        let root = std::env::temp_dir().join(format!("context69-storage-{}", Uuid::new_v4()));
        let storage = LibraryObjectStorage::from_config(&config(root.clone())).unwrap();
        let temp_key = staging_object_key(42, Uuid::new_v4());

        let mut writer = storage.open_stream_writer(&temp_key).await.unwrap();
        writer
            .write_chunk(Bytes::from_static(b"half"))
            .await
            .unwrap();
        writer.abort(&temp_key).await;
        drop(writer);
        storage.delete(&temp_key).await.unwrap();
        assert!(!storage.exists(&temp_key).await.unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn content_keys_are_group_scoped() {
        let hash = "b".repeat(64);
        assert_eq!(content_object_key(7, &hash), format!("objects/7/{hash}"));
        assert_ne!(content_object_key(7, &hash), content_object_key(8, &hash));
    }

    fn config(storage_root: PathBuf) -> FileLibraryConfig {
        FileLibraryConfig {
            storage_root,
            max_upload_size_mb: 1,
            max_upload_request_size_mb: 1,
            ingest_concurrency: 1,
            url_import_concurrency: 1,
            url_import_min_interval_ms: 1000,
            trusted_proxy_enabled: false,
            s3: None,
        }
    }
}
