//! Provider-neutral Git acquisition foundation (issue #681 phase 3A).
//!
//! This module exposes the validated inputs, hard bounds, injectable
//! transport, and the [`RepositoryAcquirer`] surface later source, task, and
//! index phases build their scheduling and persistence on. Two concrete
//! providers implement it: a strictly bounded GitHub client and an
//! origin-bound Forgejo client. The Forgejo client paginates its recursive tree
//! while the provider reports truncation and fails closed when the listing
//! cannot be completed. Neither executes repository content, invokes Git,
//! follows redirects, or exposes provider response bodies through errors. The
//! Forgejo transport accepts an explicit credential seam, applied only as a
//! bearer header.
//!
//! It also carries the pure content preparation the lexical index is built on
//! (issue #681 work unit 3B2): path-based language classification, and code text
//! that is validated and cut into verbatim, line-anchored chunks. Both are
//! lexical and deterministic, so a stored manifest and its chunks are
//! reproducible from a tree listing and its blobs alone.
//!
//! The bounded initial snapshot indexer (issue #681 work unit 3B3) composes all
//! of it: it walks one pinned tree under explicit eligible-file and Blob-fetch
//! counts and persists a complete, classified generation. Its limits live in
//! [`limits`], its orchestration in [`indexing`].

mod bounds;
mod classify;
pub(crate) mod code_search;
mod code_text;
pub(crate) mod code_vectors;
pub(crate) mod connection_readiness;
mod forgejo_client;
mod forgejo_schemas;
mod github_client;
mod github_schemas;
mod github_transport;
mod github_url;
mod http;
pub(crate) mod incremental;
pub(crate) mod indexing;
pub(crate) mod limits;
mod model;
pub(crate) mod provider_credentials;
pub(crate) mod provider_transport;
pub(crate) mod provider_url;
mod ref_path_safety;
pub(crate) mod refresh;
mod resolve;
mod selection;
mod structural_chunking;
pub(crate) mod vector_reuse;

// Request parsing and validated identities are the only Git capabilities
// composed outside their owning modules.
pub(crate) use github_transport::GitHubApiTransport;
pub(crate) use github_url::parse_canonical_github_url;
pub(crate) use provider_url::ForgejoOrigin;
pub(crate) use ref_path_safety::{SafeRef, SafeSha, SafeTreePath};

/// In-memory embedding fixture shared by the code-search and code-vector
/// unit tests.
#[cfg(test)]
pub(crate) mod support {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::{Result, anyhow};

    use crate::code_embedding_runtime::CodeEmbeddingSnapshot;
    use crate::embedding::EmbeddingProvider;

    /// Deterministic code provider used by the code-vector orchestration tests.
    pub(crate) struct FakeCodeProvider {
        pub(crate) dimensions: usize,
        pub(crate) calls: Arc<AtomicUsize>,
        pub(crate) fail: bool,
    }

    impl FakeCodeProvider {
        pub(crate) fn new(dimensions: usize, fail: bool) -> Arc<Self> {
            Arc::new(Self {
                dimensions,
                calls: Arc::new(AtomicUsize::new(0)),
                fail,
            })
        }
    }

    #[async_trait::async_trait]
    impl EmbeddingProvider for FakeCodeProvider {
        async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(anyhow!("git_code_embedding_failed"));
            }
            Ok(texts
                .iter()
                .map(|_| vec![0.25_f32; self.dimensions])
                .collect())
        }
    }

    /// Builds an immutable code snapshot around a test provider.
    pub(crate) fn code_snapshot(
        provider: Arc<dyn EmbeddingProvider>,
        dimensions: usize,
        fingerprint: &str,
    ) -> Arc<CodeEmbeddingSnapshot> {
        Arc::new(CodeEmbeddingSnapshot {
            provider,
            config: crate::config::EmbeddingConfig {
                base_url: "http://code.invalid/v1".to_string(),
                api_key: None,
                model: "code-model".to_string(),
                query_model: None,
                dimensions,
                timeout: std::time::Duration::from_secs(30),
            },
            fingerprint: fingerprint.to_string(),
        })
    }
}
