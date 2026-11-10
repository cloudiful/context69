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
#[cfg(test)]
pub(crate) use model::GitAcquisitionLimits;
pub(crate) use provider_url::ForgejoOrigin;
pub(crate) use ref_path_safety::{SafeRef, SafeSha, SafeTreePath};

#[cfg(test)]
mod module_tests;

/// In-memory provider doubles and the database fixture shared by the snapshot
/// indexing tests in `indexing`. They live beside the module tests so
/// the indexer and its limit file stay inside their size budgets.
#[cfg(test)]
pub(crate) mod support {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use bytes::Bytes;
    use sqlx::Row;
    use uuid::Uuid;

    use crate::code_embedding_runtime::CodeEmbeddingSnapshot;
    use crate::contracts::sources::{
        GitConnectionMode, GitIndexProfile, GitIndexStatus, GitProviderKind,
    };
    use crate::db::{
        Database, GitCheckpointUpdate, GitGenerationCoverage, GitRepositorySettingsUpdate,
        NewGitProviderConnection, NewGitRepositoryGeneration, NewGitRepositorySource,
        StoredGitGenerationChunk, StoredGitGenerationCodeVectors, StoredGitGenerationFile,
        StoredGitRepositoryGeneration, StoredGitRepositorySource,
    };
    use crate::domain_errors::DomainError;
    use crate::embedding::EmbeddingProvider;
    use crate::qdrant_index::code_vectors::CodeVectorPoint;

    use super::code_vectors::CodeVectorSink;
    use super::github_client::RepositoryAcquirer;
    use super::http::{GitHttpTransport, TransportResponse};
    use super::indexing::GitSnapshotProvider;
    use super::model::{BlobContent, GitAcquisitionLimits, TreeFileEntry, TreeListing};
    use super::ref_path_safety::{SafeRef, SafeSha, SafeTreePath};

    pub(crate) fn sha(seed: u8) -> String {
        format!("{seed:02x}").repeat(20)
    }

    /// A successful JSON response, as every scripted provider transport returns
    /// one.
    pub(crate) fn ok_json(body: String) -> Result<TransportResponse> {
        Ok(TransportResponse {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.into_bytes().into(),
        })
    }

    /// Standard base64, matching the provider blob encoding.
    pub(crate) fn encoded(content: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(content)
    }

    /// Transport that fails every call, so a test can build an acquirer and
    /// assert the provider needs no credential without touching the network.
    pub(crate) struct NeverTransport;

    #[async_trait]
    impl GitHttpTransport for NeverTransport {
        async fn get(&self, _url: &str, _max_body_bytes: usize) -> Result<TransportResponse> {
            Err(anyhow!("the test transport never calls the network"))
        }
    }

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

    #[async_trait]
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

    /// Records every acknowledged page, optionally failing instead.
    pub(crate) struct RecordingCodeSink {
        pub(crate) writes: Mutex<Vec<usize>>,
        pub(crate) points: Mutex<Vec<CodeVectorPoint>>,
        pub(crate) vectors: Mutex<HashMap<Uuid, Vec<f32>>>,
        pub(crate) fail: bool,
    }

    impl RecordingCodeSink {
        pub(crate) fn new(fail: bool) -> Arc<Self> {
            Arc::new(Self {
                writes: Mutex::new(Vec::new()),
                points: Mutex::new(Vec::new()),
                vectors: Mutex::new(HashMap::new()),
                fail,
            })
        }

        pub(crate) fn write_count(&self) -> usize {
            self.writes.lock().unwrap().len()
        }

        pub(crate) fn point_count(&self) -> usize {
            self.points.lock().unwrap().len()
        }

        pub(crate) fn forget_vectors(&self) {
            self.vectors.lock().unwrap().clear();
        }
    }

    #[async_trait]
    impl CodeVectorSink for RecordingCodeSink {
        async fn write(&self, dimensions: usize, points: &[CodeVectorPoint]) -> Result<()> {
            if self.fail {
                return Err(anyhow!("git_code_sink_failed"));
            }
            self.writes.lock().unwrap().push(dimensions);
            let mut stored = self.vectors.lock().unwrap();
            for point in points {
                stored.insert(point.chunk_key, point.vector.clone());
            }
            self.points.lock().unwrap().extend(points.iter().cloned());
            Ok(())
        }

        async fn retrieve(&self, point_ids: &[Uuid]) -> Result<Vec<(Uuid, Vec<f32>)>> {
            let stored = self.vectors.lock().unwrap();
            Ok(point_ids
                .iter()
                .filter_map(|id| stored.get(id).cloned().map(|vector| (*id, vector)))
                .collect())
        }
    }

    /// A source change performed mid-walk, so the pre-activation re-read sees a
    /// source that moved under the attempt.
    #[derive(Clone)]
    pub(crate) struct SourceMutation {
        pub(crate) db: Database,
        pub(crate) group_id: i64,
        pub(crate) repository_key: Uuid,
        pub(crate) commit_sha: String,
        pub(crate) action: SourceMutationAction,
    }

    #[derive(Debug, Clone)]
    pub(crate) enum SourceMutationAction {
        /// Another attempt advances the observed target.
        ObservedTarget,
        /// A user edits the selection filters.
        ExcludePatterns(Vec<String>),
        /// A user disables the source.
        Disable,
        /// A user rewrites the target ref.
        TargetRef(String),
        /// A user changes the index profile.
        IndexProfile(GitIndexProfile),
        /// A connection is attached to the source while acquisition is paused.
        ConnectionAttach,
        /// The configured connection is disabled while acquisition is paused.
        ConnectionDisable(String),
        /// A rebuild request arrives while this run owns the repository.
        ForceRebuild,
    }

    impl SourceMutation {
        async fn apply(&self) -> Result<()> {
            match &self.action {
                SourceMutationAction::ObservedTarget => {
                    self.db
                        .update_git_repository_checkpoint(
                            self.group_id,
                            self.repository_key,
                            &crate::db::GitCheckpointUpdate {
                                target_commit_sha: Some(self.commit_sha.clone()),
                                indexed_commit_sha: None,
                                index_status: crate::contracts::sources::GitIndexStatus::Indexing,
                                pipeline_version: None,
                                config_fingerprint: None,
                                skip_summary: None,
                            },
                        )
                        .await?;
                }
                SourceMutationAction::ForceRebuild => {
                    self.db
                        .request_git_force_rebuild(self.group_id, self.repository_key)
                        .await?;
                }
                SourceMutationAction::ConnectionAttach => {
                    let connection_key = format!("mutation-{}", Uuid::new_v4());
                    self.db
                        .upsert_git_provider_connection(
                            self.group_id,
                            &NewGitProviderConnection {
                                connection_key: connection_key.clone(),
                                provider: GitProviderKind::GitHub,
                                mode: GitConnectionMode::Public,
                                display_name: "Mutation connection".to_string(),
                                base_url: "https://github.com".to_string(),
                                credential_secret_key: None,
                                webhook_secret_key: None,
                            },
                        )
                        .await?;
                    attach_connection(
                        &self.db,
                        self.group_id,
                        self.repository_key,
                        &connection_key,
                    )
                    .await?;
                }
                SourceMutationAction::ConnectionDisable(connection_key) => {
                    self.db
                        .disable_git_provider_connection(self.group_id, connection_key)
                        .await?;
                }
                action => {
                    let source = self
                        .db
                        .get_git_repository_source(self.group_id, self.repository_key)
                        .await?
                        .ok_or_else(|| anyhow!("mutation target source is missing"))?;
                    let target_ref = match action {
                        SourceMutationAction::TargetRef(target_ref) => Some(target_ref.clone()),
                        _ => None,
                    };
                    let index_profile = match action {
                        SourceMutationAction::IndexProfile(profile) => {
                            Some(profile.as_str().to_owned())
                        }
                        _ => None,
                    };
                    let exclude_patterns = match action {
                        SourceMutationAction::ExcludePatterns(patterns) => patterns.clone(),
                        _ => source.exclude_patterns.clone(),
                    };
                    let enabled = !matches!(action, SourceMutationAction::Disable);
                    self.db
                        .update_git_repository_source_settings(
                            self.group_id,
                            self.repository_key,
                            &GitRepositorySettingsUpdate {
                                target_ref: target_ref.as_deref(),
                                pinned_commit_action: "keep",
                                pinned_commit_value: None,
                                index_profile: index_profile.as_deref(),
                                include_patterns: &source.include_patterns,
                                exclude_patterns: &exclude_patterns,
                                interval_seconds: source.interval_seconds,
                                webhook_enabled: source.webhook_enabled,
                                enabled,
                                retention_count: source.retention_count,
                            },
                        )
                        .await?;
                }
            }
            Ok(())
        }
    }

    #[derive(Clone)]
    pub(crate) struct FakeAcquirerSpec {
        pub(crate) commit: String,
        /// (path, blob sha, exact content)
        pub(crate) files: Vec<(&'static str, String, &'static [u8])>,
        pub(crate) excluded: usize,
        /// Acquired entries the provider declared beyond the per-blob budget.
        pub(crate) too_large: usize,
        pub(crate) fail_blob: Option<String>,
        pub(crate) mutate: Option<SourceMutation>,
    }

    struct FakeAcquirer {
        spec: FakeAcquirerSpec,
        fetched: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl RepositoryAcquirer for FakeAcquirer {
        async fn resolve_ref(&self, _safe_ref: &SafeRef) -> Result<SafeSha> {
            SafeSha::parse(&self.spec.commit)
        }

        async fn list_tree(&self, commit_sha: &SafeSha) -> Result<TreeListing> {
            if let Some(mutation) = &self.spec.mutate {
                mutation.apply().await?;
            }
            let mut files = Vec::with_capacity(self.spec.files.len());
            for (path, blob, _) in &self.spec.files {
                files.push(TreeFileEntry {
                    path: SafeTreePath::parse(path).map_err(|_| anyhow!("bad test path {path}"))?,
                    blob_sha: SafeSha::parse(blob)?,
                    size: None,
                });
            }
            Ok(TreeListing {
                commit_sha: commit_sha.clone(),
                files,
                excluded_files: self.spec.excluded,
                too_large_files: self.spec.too_large,
            })
        }

        async fn fetch_blob(&self, blob_sha: &SafeSha) -> Result<BlobContent> {
            self.fetched
                .lock()
                .unwrap()
                .push(blob_sha.as_str().to_owned());
            if self.spec.fail_blob.as_deref() == Some(blob_sha.as_str()) {
                return Err(anyhow!(DomainError::upstream_error("git_http_status_500")));
            }
            let content = self
                .spec
                .files
                .iter()
                .find(|(_, blob, _)| blob == blob_sha.as_str())
                .map(|(_, _, content)| content.to_vec())
                .ok_or_else(|| anyhow!(DomainError::internal("unexpected test blob")))?;
            Ok(BlobContent {
                declared_size: content.len() as u64,
                bytes: Bytes::from(content),
            })
        }
    }

    pub(crate) struct FakeProvider {
        spec: FakeAcquirerSpec,
        pub(crate) created: Arc<AtomicUsize>,
        pub(crate) seen_url: Mutex<Option<String>>,
        pub(crate) fetched: Arc<Mutex<Vec<String>>>,
    }

    impl FakeProvider {
        pub(crate) fn new(spec: FakeAcquirerSpec) -> Self {
            Self {
                spec,
                created: Arc::new(AtomicUsize::new(0)),
                seen_url: Mutex::new(None),
                fetched: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl GitSnapshotProvider for FakeProvider {
        fn acquirer(
            &self,
            canonical_url: &str,
            _limits: GitAcquisitionLimits,
        ) -> Result<Box<dyn RepositoryAcquirer>> {
            self.created.fetch_add(1, Ordering::SeqCst);
            *self.seen_url.lock().unwrap() = Some(canonical_url.to_owned());
            Ok(Box::new(FakeAcquirer {
                spec: self.spec.clone(),
                fetched: Arc::clone(&self.fetched),
            }))
        }
    }

    /// State a test asserts on, read before cleanup so a failing assertion
    /// leaves no rows behind.
    pub(crate) struct Reads {
        pub(crate) active: Option<Uuid>,
        pub(crate) source: Option<StoredGitRepositorySource>,
        pub(crate) generations: Vec<StoredGitRepositoryGeneration>,
        pub(crate) files: Vec<StoredGitGenerationFile>,
        pub(crate) chunks: Vec<StoredGitGenerationChunk>,
        pub(crate) code_vectors: Option<StoredGitGenerationCodeVectors>,
    }

    /// Attach a connection reference through the production upsert, copying
    /// identity, pin, profile, and both policies from the stored source so only
    /// the reference changes.
    pub(crate) async fn attach_connection(
        db: &Database,
        group_id: i64,
        repository_key: Uuid,
        connection_key: &str,
    ) -> Result<()> {
        let source = db
            .get_git_repository_source(group_id, repository_key)
            .await?
            .ok_or_else(|| anyhow!("the connection target source is missing"))?;
        db.upsert_git_repository_source(
            group_id,
            &NewGitRepositorySource {
                connection_key: Some(connection_key.to_owned()),
                provider: source.provider,
                canonical_url: source.canonical_url.clone(),
                owner: source.owner.clone(),
                name: source.name.clone(),
                default_branch: source.default_branch.clone(),
                target_ref: source.version.ref_name.clone(),
                pinned_commit_sha: source.version.commit_sha.clone(),
                index_profile: source.index_profile,
                include_patterns: source.include_patterns.clone(),
                exclude_patterns: source.exclude_patterns.clone(),
                interval_seconds: source.interval_seconds,
                webhook_enabled: source.webhook_enabled,
                enabled: source.enabled,
                retention_count: source.retention_count,
                created_by_user_id: source.created_by_user_id,
            },
        )
        .await?;
        Ok(())
    }

    pub(crate) struct Fixture {
        db: Database,
        pub(crate) group_id: i64,
        pub(crate) repository_key: Uuid,
    }

    impl Fixture {
        /// The scratch fixture for the database-backed cases.
        ///
        /// The target comes from the shared resolver, so no bespoke URL handling
        /// is left here, and a missing target fails instead of skipping.
        pub(crate) async fn setup() -> Self {
            Self::setup_with_profile(GitIndexProfile::Lexical).await
        }

        /// The same fixture registered with an explicit index profile, so code
        /// vector cases exercise the non-lexical path without a second setup.
        pub(crate) async fn setup_with_profile(profile: GitIndexProfile) -> Self {
            let url = context69_test_support::accessor::database_url();
            let db = Database::connect(&url)
                .await
                .expect("connect test database");
            Self::seeded(db, profile).await
        }

        async fn seeded(db: Database, profile: GitIndexProfile) -> Self {
            let key = format!("git-index-{}", Uuid::new_v4());
            let group_id = sqlx::query(
                "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
                 VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
            )
            .bind(&key)
            .bind("Git Index Test Group")
            .bind(format!("/{key}"))
            .fetch_one(db.pool())
            .await
            .expect("seed test group")
            .get("id");
            let nonce = Uuid::new_v4();
            let repository_key = db
                .upsert_git_repository_source(
                    group_id,
                    &NewGitRepositorySource {
                        connection_key: None,
                        provider: GitProviderKind::GitHub,
                        canonical_url: format!("https://github.com/octo/{nonce}"),
                        owner: "octo".to_string(),
                        name: nonce.to_string(),
                        default_branch: "main".to_string(),
                        target_ref: "refs/heads/main".to_string(),
                        pinned_commit_sha: None,
                        index_profile: profile,
                        include_patterns: Vec::new(),
                        exclude_patterns: Vec::new(),
                        interval_seconds: None,
                        webhook_enabled: false,
                        enabled: true,
                        retention_count: 2,
                        created_by_user_id: None,
                    },
                )
                .await
                .expect("seed repository source")
                .repository_key;
            Self {
                db,
                group_id,
                repository_key,
            }
        }

        pub(crate) fn db(&self) -> &Database {
            &self.db
        }

        /// A disposable task user, created through the checked task-execution
        /// seed statement.
        pub(crate) async fn seed_user(&self) -> i64 {
            sqlx::query_file_scalar!(
                "tests/sql/task_execution/seed_user.sql",
                format!("git-fixture-{}", Uuid::new_v4()),
                "Git Fixture Test",
                "x"
            )
            .fetch_one(self.db.pool())
            .await
            .expect("seed the fixture user")
        }

        pub(crate) async fn attach_connection(&self, connection_key: &str) -> Result<()> {
            attach_connection(&self.db, self.group_id, self.repository_key, connection_key).await
        }

        pub(crate) async fn start_prior_generation(&self) -> Uuid {
            let prior = self
                .db
                .start_git_repository_generation(
                    self.group_id,
                    self.repository_key,
                    &NewGitRepositoryGeneration {
                        ref_name: "refs/heads/main".to_string(),
                        commit_sha: sha(0x10),
                        index_profile: GitIndexProfile::Lexical,
                    },
                )
                .await
                .expect("start prior generation");
            self.db
                .complete_and_activate_git_repository_generation(
                    self.group_id,
                    self.repository_key,
                    prior.generation_key,
                    GitGenerationCoverage {
                        file_count: 1,
                        excluded_file_count: 0,
                        total_bytes: 5,
                    },
                )
                .await
                .expect("activate prior generation");
            prior.generation_key
        }

        pub(crate) async fn observe_target(&self, commit_sha: &str) {
            let source = self
                .db
                .get_git_repository_source(self.group_id, self.repository_key)
                .await
                .expect("read source")
                .expect("the fixture owns the source");
            self.db
                .update_git_repository_checkpoint(
                    self.group_id,
                    self.repository_key,
                    &GitCheckpointUpdate {
                        target_commit_sha: Some(commit_sha.to_owned()),
                        indexed_commit_sha: source.checkpoint.indexed_commit_sha.clone(),
                        index_status: GitIndexStatus::Indexing,
                        pipeline_version: None,
                        config_fingerprint: None,
                        skip_summary: None,
                    },
                )
                .await
                .expect("observe the new target")
                .expect("the fixture owns the source");
        }

        pub(crate) async fn reads(&self, generation_key: Option<Uuid>) -> Reads {
            let active = self
                .db
                .get_git_active_generation(self.group_id, self.repository_key)
                .await
                .ok()
                .flatten()
                .map(|active| active.generation_key);
            let source = self
                .db
                .get_git_repository_source(self.group_id, self.repository_key)
                .await
                .ok()
                .flatten();
            let generations = self
                .db
                .list_git_repository_generations(self.group_id, self.repository_key)
                .await
                .unwrap_or_default();
            let files = match generation_key {
                Some(key) => self
                    .db
                    .list_git_generation_files(self.group_id, self.repository_key, key, 100, 0)
                    .await
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            let chunks = match (
                generation_key,
                files.iter().find(|file| file.path == "src/main.rs"),
            ) {
                (Some(key), Some(file)) => self
                    .db
                    .list_git_generation_chunks(
                        self.group_id,
                        self.repository_key,
                        key,
                        file.file_key,
                        100,
                        0,
                    )
                    .await
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            let code_vectors = match generation_key {
                Some(key) => self
                    .db
                    .get_git_generation_code_vectors(self.group_id, self.repository_key, key)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            Reads {
                active,
                source,
                generations,
                files,
                chunks,
                code_vectors,
            }
        }

        pub(crate) async fn cleanup(self) {
            sqlx::query("DELETE FROM context69.groups WHERE id = $1")
                .bind(self.group_id)
                .execute(self.db.pool())
                .await
                .expect("clean up test group");
        }
    }

    /// A snapshot whose listing excludes two entries and whose third file is
    /// binary.
    pub(crate) fn binary_spec(commit: &str) -> FakeAcquirerSpec {
        FakeAcquirerSpec {
            commit: commit.to_owned(),
            files: vec![
                ("src/main.rs", sha(0xb1), b"fn main() {}\n"),
                ("README.md", sha(0xb2), b"# hello\n"),
                ("assets/logo.bin", sha(0xb3), &[0xff, 0xfe, 0x00]),
            ],
            excluded: 2,
            too_large: 0,
            fail_blob: None,
            mutate: None,
        }
    }
}
