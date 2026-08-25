use std::path::Path;

use anyhow::{Context, Result};
use bytes::Bytes;
use opendal::{Operator, services};

use crate::config::FileLibraryConfig;

use super::dependency_runtime::bounded_s3_operation;

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
                operator: Operator::new(builder)?.finish(),
                backend: "s3",
            });
        }

        std::fs::create_dir_all(&config.storage_root).with_context(|| {
            format!(
                "failed to create storage root {}",
                config.storage_root.display()
            )
        })?;
        let builder = services::Fs::default().root(path_text(&config.storage_root)?);
        Ok(Self {
            operator: Operator::new(builder)?.finish(),
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
                .context("S3 connection check failed")
        } else {
            self.operator
                .check()
                .await
                .context("storage connection check failed")
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
            .with_context(|| format!("failed to write stored object {key}"))?;
        } else {
            self.operator
                .write(key, bytes)
                .await
                .with_context(|| format!("failed to write stored object {key}"))?;
        }
        Ok(())
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
            Err(error) => Err(error).with_context(|| format!("failed to read stored object {key}")),
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
            .with_context(|| format!("failed to inspect stored object {key}"))
        } else {
            self.operator
                .exists(key)
                .await
                .with_context(|| format!("failed to inspect stored object {key}"))
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
            .with_context(|| format!("failed to delete stored object {key}"))
        } else {
            self.operator
                .delete(key)
                .await
                .with_context(|| format!("failed to delete stored object {key}"))
        }
    }
}

pub(super) fn content_object_key(group_id: i64, sha256: &str) -> String {
    format!("objects/{group_id}/{sha256}")
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("storage root is not valid UTF-8: {}", path.display()))
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

    use super::{LibraryObjectStorage, content_object_key};

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
