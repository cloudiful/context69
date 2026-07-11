use std::path::Path;

use anyhow::{Context, Result};
use bytes::Bytes;
use opendal::{Operator, services};

use crate::config::FileLibraryConfig;

#[derive(Clone)]
pub(super) struct LibraryObjectStorage {
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

    pub(super) fn backend(&self) -> &'static str {
        self.backend
    }

    pub(super) async fn write(&self, key: &str, bytes: Bytes) -> Result<()> {
        self.operator
            .write(key, bytes)
            .await
            .with_context(|| format!("failed to write stored object {key}"))?;
        Ok(())
    }

    pub(super) async fn read(&self, key: &str) -> Result<Option<Bytes>> {
        match self.operator.read(key).await {
            Ok(buffer) => Ok(Some(Bytes::from(buffer.to_vec()))),
            Err(error) if error.kind() == opendal::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("failed to read stored object {key}")),
        }
    }

    pub(super) async fn exists(&self, key: &str) -> Result<bool> {
        self.operator
            .exists(key)
            .await
            .with_context(|| format!("failed to inspect stored object {key}"))
    }

    pub(super) async fn delete(&self, key: &str) -> Result<()> {
        self.operator
            .delete(key)
            .await
            .with_context(|| format!("failed to delete stored object {key}"))
    }
}

pub(super) fn content_object_key(group_id: i64, sha256: &str) -> String {
    format!("objects/{group_id}/{sha256}")
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("storage root is not valid UTF-8: {}", path.display()))
}
