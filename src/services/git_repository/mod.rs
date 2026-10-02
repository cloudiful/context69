//! Provider-neutral Git acquisition foundation (issue #681 phase 3A).
//!
//! This module exposes the validated inputs, hard bounds, injectable
//! transport, and the [`RepositoryAcquirer`] surface later source, task, and
//! index phases build their scheduling and persistence on. The only concrete
//! provider here is a strictly bounded public GitHub client: it resolves a
//! ref, lists one recursive tree, and fetches individual blobs. It never
//! executes repository content, invokes Git, follows redirects, accepts
//! credentials, or exposes provider response bodies through errors.
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
//! [`limits`], its orchestration in [`index_generation`].

mod bounds;
mod classify;
mod code_text;
pub(crate) mod connection_readiness;
mod github_client;
mod github_schemas;
mod github_transport;
mod github_url;
pub(crate) mod index_generation;
pub(crate) mod limits;
mod model;
mod ref_path_safety;
mod resolve;

// Content preparation is reached through its own modules (and by the snapshot
// indexer in [`index_generation`]); re-exporting it here would only add an
// unused import.
pub(crate) use github_client::{GitHubAcquirer, RepositoryAcquirer};
pub(crate) use github_transport::{GitHttpTransport, GitHubApiTransport, TransportResponse};
pub(crate) use github_url::{GitHubRepoCoordinates, parse_canonical_github_url};
pub(crate) use model::{BlobContent, GitAcquisitionLimits, TreeFileEntry, TreeListing};
pub(crate) use ref_path_safety::{SafeRef, SafeSha, SafeTreePath};

#[cfg(test)]
mod module_tests;

/// In-memory provider doubles and the database fixture shared by the snapshot
/// indexing tests in `index_generation`. They live beside the module tests so
/// the indexer and its limit file stay inside their size budgets.
#[cfg(test)]
pub(crate) mod support {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use bytes::Bytes;
    use sqlx::Row;
    use uuid::Uuid;

    use crate::contracts::sources::{
        GitIndexProfile, GitIndexStatus, GitProviderKind, GitRefreshPolicy,
    };
    use crate::db::{
        Database, GitCheckpointUpdate, GitGenerationCoverage, NewGitRepositoryGeneration,
        NewGitRepositorySource, StoredGitGenerationChunk, StoredGitGenerationFile,
        StoredGitRepositoryGeneration, StoredGitRepositorySource,
    };
    use crate::domain_errors::DomainError;

    use super::github_client::RepositoryAcquirer;
    use super::github_transport::{GitHttpTransport, TransportResponse};
    use super::index_generation::GitSnapshotProvider;
    use super::model::{BlobContent, GitAcquisitionLimits, TreeFileEntry, TreeListing};
    use super::ref_path_safety::{SafeRef, SafeSha, SafeTreePath};

    pub(crate) fn sha(seed: u8) -> String {
        format!("{seed:02x}").repeat(20)
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

    /// A source change performed mid-walk, so the pre-activation re-read sees a
    /// target that moved under the attempt.
    #[derive(Clone)]
    pub(crate) struct SourceMutation {
        pub(crate) db: Database,
        pub(crate) group_id: i64,
        pub(crate) repository_key: Uuid,
        pub(crate) commit_sha: String,
    }

    #[derive(Clone)]
    pub(crate) struct FakeAcquirerSpec {
        pub(crate) commit: String,
        /// (path, blob sha, exact content)
        pub(crate) files: Vec<(&'static str, String, &'static [u8])>,
        pub(crate) excluded: usize,
        pub(crate) fail_blob: Option<String>,
        pub(crate) mutate: Option<SourceMutation>,
    }

    struct FakeAcquirer {
        spec: FakeAcquirerSpec,
    }

    #[async_trait]
    impl RepositoryAcquirer for FakeAcquirer {
        async fn resolve_ref(&self, _safe_ref: &SafeRef) -> Result<SafeSha> {
            SafeSha::parse(&self.spec.commit)
        }

        async fn list_tree(&self, commit_sha: &SafeSha) -> Result<TreeListing> {
            if let Some(mutation) = &self.spec.mutate {
                mutation
                    .db
                    .update_git_repository_checkpoint(
                        mutation.group_id,
                        mutation.repository_key,
                        &GitCheckpointUpdate {
                            target_commit_sha: Some(mutation.commit_sha.clone()),
                            indexed_commit_sha: None,
                            index_status: GitIndexStatus::Indexing,
                        },
                    )
                    .await?;
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
            })
        }

        async fn fetch_blob(&self, blob_sha: &SafeSha) -> Result<BlobContent> {
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
                blob_sha: SafeSha::parse(blob_sha.as_str())?,
                declared_size: content.len() as u64,
                bytes: Bytes::from(content),
            })
        }
    }

    pub(crate) struct FakeProvider {
        spec: FakeAcquirerSpec,
        pub(crate) created: Arc<AtomicUsize>,
        pub(crate) seen_url: Mutex<Option<String>>,
    }

    impl FakeProvider {
        pub(crate) fn new(spec: FakeAcquirerSpec) -> Self {
            Self {
                spec,
                created: Arc::new(AtomicUsize::new(0)),
                seen_url: Mutex::new(None),
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
    }

    pub(crate) struct Fixture {
        db: Database,
        pub(crate) group_id: i64,
        pub(crate) repository_key: Uuid,
    }

    impl Fixture {
        pub(crate) async fn setup() -> Option<Self> {
            let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
            let db = Database::connect(&url)
                .await
                .expect("connect test database");
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
                        target_commit_sha: None,
                        index_profile: GitIndexProfile::Lexical,
                        refresh_policy: GitRefreshPolicy::Manual,
                    },
                )
                .await
                .expect("seed repository source")
                .repository_key;
            Some(Self {
                db,
                group_id,
                repository_key,
            })
        }

        pub(crate) fn db(&self) -> &Database {
            &self.db
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
            Reads {
                active,
                source,
                generations,
                files,
                chunks,
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
            fail_blob: None,
            mutate: None,
        }
    }
}
