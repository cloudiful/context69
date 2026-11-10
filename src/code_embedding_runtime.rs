//! The process-wide handle to the optional Git code embedding provider.
//!
//! Code embedding is independent of the document embedding runtime: it has its
//! own configuration, credential, and immutable snapshot, and it never borrows
//! a document value. The handle publishes one complete snapshot at a time, so a
//! reader clones either the previous or the next provider/config/fingerprint
//! triple and can never mix fields from two installations.
//!
//! A generation that starts after an install keeps the snapshot it captured;
//! changing code settings never relabels vectors that a running generation is
//! already producing.

use std::sync::{Arc, RwLock};

use anyhow::Result;

use crate::{
    config::EmbeddingConfig,
    embedding::{EmbeddingProvider, build_provider},
};

/// The versioned code-specific input policy identity.
///
/// Code inputs are sent exactly as chunked, with no token cap, server prefix,
/// or special-token switch. This explicit marker keeps that fixed decision
/// inside the vector identity so code and document spaces never collide.
pub const CODE_INPUT_POLICY_IDENTITY: &str =
    "code-input-policy-v1:no-token-cap:no-server-prefix:no-special-tokens";

/// One immutable code embedding provider, its configuration, and its identity.
#[derive(Clone)]
pub struct CodeEmbeddingSnapshot {
    pub provider: Arc<dyn EmbeddingProvider>,
    pub config: EmbeddingConfig,
    pub fingerprint: String,
}

/// A shared handle to the code embedding snapshot currently in effect.
#[derive(Clone, Default)]
pub struct CodeEmbeddingRuntime {
    snapshot: Arc<RwLock<Option<Arc<CodeEmbeddingSnapshot>>>>,
}

impl CodeEmbeddingRuntime {
    /// An unconfigured handle. Code embedding is disabled by default.
    pub fn new() -> Self {
        Self::default()
    }

    /// The provider currently in effect, or `None` when code embedding is not
    /// configured.
    pub fn snapshot(&self) -> Option<Arc<CodeEmbeddingSnapshot>> {
        self.read().clone()
    }

    /// Atomically installs a new provider, configuration, and identity.
    pub fn install(
        &self,
        provider: Arc<dyn EmbeddingProvider>,
        config: EmbeddingConfig,
        fingerprint: String,
    ) {
        let snapshot = Arc::new(CodeEmbeddingSnapshot {
            provider,
            config,
            fingerprint,
        });
        *self.write() = Some(snapshot);
    }

    /// Atomically removes the current snapshot, leaving code embedding
    /// unconfigured. A generation that already captured its snapshot keeps it.
    pub fn disable(&self) {
        *self.write() = None;
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Option<Arc<CodeEmbeddingSnapshot>>> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Option<Arc<CodeEmbeddingSnapshot>>> {
        self.snapshot
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Thin alias of the shared provider builder.
pub fn build_code_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    build_provider(config)
}

/// The deterministic identity of one code vector space.
///
/// It includes the endpoint, the code document model, the effective query
/// model, the dimensions, and the fixed code input-policy identity. The API
/// key, timeout, and batching limits are deliberately excluded: they do not
/// change the vectors a provider produces.
pub fn code_embedding_fingerprint(config: &EmbeddingConfig) -> String {
    use sha2::{Digest, Sha256};

    let query_model = config
        .query_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| config.model.trim());
    let parts = [
        config.base_url.trim().trim_end_matches('/').to_string(),
        config.model.trim().to_string(),
        query_model.to_string(),
        config.dimensions.to_string(),
        CODE_INPUT_POLICY_IDENTITY.to_string(),
    ];
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{CodeEmbeddingRuntime, code_embedding_fingerprint};

    fn config() -> crate::config::EmbeddingConfig {
        crate::config::EmbeddingConfig {
            base_url: "http://embedding.invalid/v1/".to_string(),
            api_key: Some("first-key".to_string()),
            model: "code-model".to_string(),
            query_model: Some("code-query".to_string()),
            dimensions: 1024,
            timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn fingerprint_ignores_credentials_and_timeouts() {
        let base = config();
        let mut changed = config();
        changed.api_key = Some("another-key".to_string());
        changed.timeout = Duration::from_secs(90);
        assert_eq!(
            code_embedding_fingerprint(&base),
            code_embedding_fingerprint(&changed)
        );
    }

    #[test]
    fn fingerprint_changes_with_the_vector_space_identity() {
        let base = config();
        let fingerprint = code_embedding_fingerprint(&base);
        let identity_changes: [fn(&mut crate::config::EmbeddingConfig); 4] = [
            |config| config.base_url = "http://other.invalid/v1".to_string(),
            |config| config.model = "other-code-model".to_string(),
            |config| config.query_model = Some("other-query".to_string()),
            |config| config.dimensions = 768,
        ];
        for change in identity_changes {
            let mut changed = config();
            change(&mut changed);
            assert_ne!(fingerprint, code_embedding_fingerprint(&changed));
        }
    }

    #[test]
    fn an_absent_query_model_uses_the_code_model_identity() {
        let mut absent = config();
        absent.query_model = None;
        let mut explicit = config();
        explicit.query_model = Some("code-model".to_string());
        assert_eq!(
            code_embedding_fingerprint(&absent),
            code_embedding_fingerprint(&explicit)
        );
    }

    #[test]
    fn disable_removes_only_the_current_snapshot() {
        let runtime = CodeEmbeddingRuntime::new();
        assert!(runtime.snapshot().is_none());
        runtime.disable();
        assert!(runtime.snapshot().is_none());
    }
}
