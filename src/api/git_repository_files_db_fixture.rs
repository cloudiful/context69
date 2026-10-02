//! Disposable-schema fixture for the manifest listing round trips (issue #681
//! phase 5A).
//!
//! The fixture owns everything a test needs to exist: one group and repository,
//! an activated generation with a stored manifest, and a page read that goes
//! through the route's own composition. It exists only under `cfg(test)`, skips
//! when no scratch database is configured, and removes every group it created.
//! Nothing here reads a secret: the manifest stores repository content, never a
//! credential or a store reference.

use sqlx::Row;
use uuid::Uuid;

use crate::{
    contracts::{
        CursorPageQuery,
        sources::{
            GitIndexProfile, GitIndexStatus, GitProviderKind, GitRefreshPolicy,
            GitRepositoryFileListResponse,
        },
    },
    db::{
        Database, GitCheckpointUpdate, GitGenerationCoverage, NewGitGenerationFile,
        NewGitRepositoryGeneration, NewGitRepositorySource, StoredGitGenerationFile,
        StoredGitRepositorySource,
    },
};

use super::git_repository_file_paging::requested_page;
use super::git_repository_files::{manifest_page, serving_generation};

/// The commit the indexed generation covers.
pub(super) const INDEXED_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
/// The commit the repository ref has since moved to.
pub(super) const TARGET_COMMIT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
/// How many entries [`unsorted_manifest`] stores.
pub(super) const MANIFEST_ENTRIES: usize = 5;

/// One group, one repository, and the generations indexed into them.
pub(super) struct Fixture {
    pub(super) db: Database,
    pub(super) group_id: i64,
    pub(super) repository_key: Uuid,
}

impl Fixture {
    /// A group and a repository, or `None` when no scratch database is
    /// configured, which skips the calling test.
    pub(super) async fn setup() -> Option<Self> {
        let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
            eprintln!(
                "CONTEXT69_TEST_DATABASE_URL is not set; skipping git manifest page round trip"
            );
            return None;
        };
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        let group_id = seed_group(&db).await;
        let repository_key = seed_repository(&db, group_id).await;
        Some(Self {
            db,
            group_id,
            repository_key,
        })
    }

    /// A second group, used to prove the reads are group-confined.
    pub(super) async fn foreign_group(&self) -> i64 {
        seed_group(&self.db).await
    }

    /// Registers a repository with the given group.
    pub(super) async fn repository_for(&self, group_id: i64) -> Uuid {
        seed_repository(&self.db, group_id).await
    }

    /// The standard manifest, indexed and activated: the starting point for
    /// every test that needs a repository serving a generation.
    pub(super) async fn indexed(&self) -> Uuid {
        self.activate(&unsorted_manifest(), coverage(MANIFEST_ENTRIES as i64))
            .await
    }

    /// Starts a generation and leaves it building, for the reads that must
    /// refuse a generation the repository does not yet advertise.
    pub(super) async fn start_building(&self) -> Uuid {
        self.start_generation().await
    }

    /// Stores the standard manifest into a building generation.
    pub(super) async fn store_manifest(&self, generation_key: Uuid) {
        self.replace_manifest(generation_key, &unsorted_manifest())
            .await
    }

    /// One exact manifest entry of this fixture's repository, or `None`.
    pub(super) async fn file(
        &self,
        generation_key: Uuid,
        path: &str,
    ) -> Option<StoredGitGenerationFile> {
        self.file_in(self.group_id, self.repository_key, generation_key, path)
            .await
    }

    /// One exact manifest entry as the given group would read it. A group that
    /// does not own the repository matches no row, which is what makes the
    /// confinement assertions meaningful.
    pub(super) async fn file_in(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        path: &str,
    ) -> Option<StoredGitGenerationFile> {
        self.db
            .get_git_generation_file(group_id, repository_key, generation_key, path)
            .await
            .expect("read one exact manifest entry")
    }

    /// Indexes one generation, stores `files` as its manifest, activates it, and
    /// advances the source checkpoint the way the snapshot orchestration does,
    /// so the page has real provenance and freshness to report.
    async fn activate(
        &self,
        files: &[(&str, &str, &str)],
        coverage: GitGenerationCoverage,
    ) -> Uuid {
        let generation_key = self.start_generation().await;
        self.replace_manifest(generation_key, files).await;
        self.db
            .complete_and_activate_git_repository_generation(
                self.group_id,
                self.repository_key,
                generation_key,
                coverage,
            )
            .await
            .expect("activate the generation");
        self.db
            .update_git_repository_checkpoint(
                self.group_id,
                self.repository_key,
                &GitCheckpointUpdate {
                    target_commit_sha: Some(TARGET_COMMIT.to_string()),
                    indexed_commit_sha: Some(INDEXED_COMMIT.to_string()),
                    index_status: GitIndexStatus::Ready,
                },
            )
            .await
            .expect("advance the source checkpoint")
            .expect("the repository is owned by this group");
        generation_key
    }

    /// Opens the next generation of this repository, pinned to the indexed
    /// commit.
    async fn start_generation(&self) -> Uuid {
        self.db
            .start_git_repository_generation(
                self.group_id,
                self.repository_key,
                &NewGitRepositoryGeneration {
                    ref_name: "refs/heads/main".to_string(),
                    commit_sha: INDEXED_COMMIT.to_string(),
                    index_profile: GitIndexProfile::Lexical,
                },
            )
            .await
            .expect("start a generation")
            .generation_key
    }

    /// Stores a whole manifest into a building generation.
    async fn replace_manifest(&self, generation_key: Uuid, files: &[(&str, &str, &str)]) {
        self.db
            .replace_git_generation_files(
                self.group_id,
                self.repository_key,
                generation_key,
                &files
                    .iter()
                    .map(|(path, content, language)| manifest_entry(path, content, language))
                    .collect::<Vec<_>>(),
            )
            .await
            .expect("store the manifest");
    }

    /// The stored source, as the route reads it.
    pub(super) async fn source(&self) -> StoredGitRepositorySource {
        self.db
            .get_git_repository_source(self.group_id, self.repository_key)
            .await
            .expect("read the source")
            .expect("the repository is owned by this group")
    }

    /// Reads one page the way the route does: the stored source, the serving
    /// generation, the bounded manifest window, and the composed response.
    pub(super) async fn page(
        &self,
        limit: u32,
        cursor: Option<&str>,
    ) -> GitRepositoryFileListResponse {
        let page = requested_page(&CursorPageQuery {
            limit,
            cursor: cursor.map(ToOwned::to_owned),
        })
        .expect("a bounded page request");
        let source = self.source().await;
        let generation = serving_generation(&self.db, &source)
            .await
            .expect("resolve the serving generation")
            .expect("the repository serves an active generation");
        let mut rows = self
            .db
            .list_git_generation_files(
                self.group_id,
                self.repository_key,
                generation.generation_key,
                page.fetch_limit(),
                page.offset,
            )
            .await
            .expect("list the manifest window");
        let has_more = rows.len() as i64 > page.limit();
        if has_more {
            rows.truncate(page.limit() as usize);
        }
        manifest_page(&source, &generation, rows, &page, has_more)
    }

    /// Removes every group this fixture created, taking its repositories,
    /// generations, and manifest rows with it.
    pub(super) async fn cleanup(&self, group_ids: &[i64]) {
        for group_id in group_ids {
            sqlx::query("DELETE FROM context69.groups WHERE id = $1")
                .bind(group_id)
                .execute(self.db.pool())
                .await
                .expect("clean up a test group");
        }
    }
}

/// A group row keyed by a fresh key, so a rerun never collides.
async fn seed_group(db: &Database) -> i64 {
    let key = format!("git-manifest-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1, $2, 'private', 'personal', $3) RETURNING id")
        .bind(&key)
        .bind("Git Manifest Test Group")
        .bind(format!("/{key}"))
        .fetch_one(db.pool())
        .await
        .expect("seed a test group")
        .get("id")
}

/// A repository source in the given group, pinned to a ref and a target commit
/// that is not yet the indexed one, so freshness stays visible.
async fn seed_repository(db: &Database, group_id: i64) -> Uuid {
    let nonce = Uuid::new_v4();
    db.upsert_git_repository_source(
        group_id,
        &NewGitRepositorySource {
            connection_key: None,
            provider: GitProviderKind::Generic,
            canonical_url: format!("https://example.invalid/{nonce}.git"),
            owner: "manifest".to_string(),
            name: nonce.to_string(),
            default_branch: "main".to_string(),
            target_ref: "refs/heads/main".to_string(),
            target_commit_sha: Some(TARGET_COMMIT.to_string()),
            index_profile: GitIndexProfile::Lexical,
            refresh_policy: GitRefreshPolicy::Manual,
        },
    )
    .await
    .expect("seed a repository source")
    .repository_key
}

/// One manifest entry. Content is stored verbatim, so distinct paths with
/// distinct text keep distinct blobs and distinct byte counts.
fn manifest_entry(path: &str, content: &str, language: &str) -> NewGitGenerationFile {
    NewGitGenerationFile {
        path: path.to_string(),
        language: language.to_string(),
        provider_blob_sha: provider_blob_sha(content),
        content: content.as_bytes().to_vec(),
        line_count: content.split_inclusive('\n').count() as i64,
    }
}

/// A provider-shaped blob id derived from the content it addresses.
fn provider_blob_sha(content: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce8_4222_2325;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:040x}")
}

/// [`MANIFEST_ENTRIES`] in deliberately unsorted order, so page order can only
/// come from the read rather than from insertion order. The expected order is
/// never spelled out here: it is whatever one unpaginated read returns, which
/// keeps the tests independent of the database collation.
pub(super) fn unsorted_manifest() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("src/zeta.rs", "fn zeta() {}\n", "rust"),
        ("README.md", "# manifest\n", "markdown"),
        ("src/alpha.rs", "fn alpha() {}\n", "rust"),
        ("docs/guide.md", "guide\n", "markdown"),
        ("src/mid.rs", "fn mid() {}\n", "rust"),
    ]
}

/// Coverage counters to record for an activated generation.
pub(super) fn coverage(file_count: i64) -> GitGenerationCoverage {
    GitGenerationCoverage {
        file_count,
        excluded_file_count: 2,
        total_bytes: 4096,
    }
}

/// The manifest paths of one page, in the order the page served them.
pub(super) fn paths(page: &GitRepositoryFileListResponse) -> Vec<String> {
    page.files.iter().map(|file| file.path.clone()).collect()
}
