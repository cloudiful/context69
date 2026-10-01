//! Reading stored Git code back: verbatim chunks and bounded lexical search
//! (issue #681 work unit 3B2).
//!
//! These exercise what only a database can answer about retrieving code: a
//! lexical query that serves the activated generation alone with
//! repository/ref/commit/path/line provenance, identifiers and a percent sign
//! kept intact, and the visibility and group isolation of a private repository.
//! Storing content is in `tests/git_file_chunks.rs` and literal metacharacters in
//! `tests/git_file_search_literals.rs`; all three need CONTEXT69_TEST_DATABASE_URL.

use context69::contracts::sources::{
    GitCodeLexicalHit, GitCodeMatchKind, GitIndexProfile, GitProviderKind, GitRefreshPolicy,
};
use context69::db::{
    Database, GitGenerationCoverage, GitLexicalCodeSearch, NewGitGenerationChunk,
    NewGitGenerationFile, NewGitRepositoryGeneration, NewGitRepositorySource,
    StoredGitGenerationFile,
};
use sqlx::Row;
use uuid::Uuid;

/// Provider blob id for test content: the database deduplicates on
/// content-addressed ids, so equal bytes must always produce an equal id.
fn blob_sha(content: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:040x}")
}

fn manifest_entry(path: &str, content: &str, language: &str) -> NewGitGenerationFile {
    NewGitGenerationFile {
        path: path.to_string(),
        language: language.to_string(),
        provider_blob_sha: blob_sha(content),
        content: content.as_bytes().to_vec(),
        line_count: content.split_inclusive('\n').count() as i64,
    }
}

fn chunk(index: i32, start_line: i32, end_line: i32, text: &str) -> NewGitGenerationChunk {
    NewGitGenerationChunk {
        chunk_index: index,
        start_line,
        end_line,
        text: text.to_string(),
    }
}

/// One group, one repository, and the generations indexed into them.
struct Fixture {
    db: Database,
    group_id: i64,
    repository_key: Uuid,
}

impl Fixture {
    /// A group and a repository, or `None` when no scratch database is configured.
    async fn setup(visibility: &str) -> Option<Self> {
        let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping git file search test");
            return None;
        };
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        let key = format!("git-content-{}", Uuid::new_v4());
        let group_id = sqlx::query("INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1, $2, $3, 'personal', $4) RETURNING id")
            .bind(&key)
            .bind("Git Content Test Group")
            .bind(visibility)
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
                    provider: GitProviderKind::Generic,
                    canonical_url: format!("https://example.invalid/{nonce}.git"),
                    owner: "content".to_string(),
                    name: nonce.to_string(),
                    default_branch: "main".to_string(),
                    target_ref: "refs/heads/main".to_string(),
                    target_commit_sha: Some("a".repeat(40)),
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

    async fn start_generation(&self, commit: &str) -> Uuid {
        self.db
            .start_git_repository_generation(
                self.group_id,
                self.repository_key,
                &NewGitRepositoryGeneration {
                    ref_name: "refs/heads/main".to_string(),
                    commit_sha: commit.to_string(),
                    index_profile: GitIndexProfile::Lexical,
                },
            )
            .await
            .expect("start generation")
            .generation_key
    }

    /// Stores a whole manifest, one chunk per file covering all of its lines.
    async fn store_and_chunk(&self, generation_key: Uuid, files: &[(&str, &str, &str)]) {
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
        for (path, content, _) in files {
            let file = self.file(generation_key, path).await;
            self.db
                .replace_git_file_chunks(
                    self.group_id,
                    self.repository_key,
                    generation_key,
                    file.file_key,
                    &[chunk(0, 1, file.line_count as i32, content)],
                )
                .await
                .expect("write chunks");
        }
    }

    async fn activate(&self, generation_key: Uuid, file_count: i64) {
        self.db
            .complete_and_activate_git_repository_generation(
                self.group_id,
                self.repository_key,
                generation_key,
                GitGenerationCoverage {
                    file_count,
                    excluded_file_count: 0,
                    total_bytes: 0,
                },
            )
            .await
            .expect("activate generation");
    }

    async fn file(&self, generation_key: Uuid, path: &str) -> StoredGitGenerationFile {
        self.db
            .get_git_generation_file(self.group_id, self.repository_key, generation_key, path)
            .await
            .expect("read manifest entry")
            .unwrap_or_else(|| panic!("{path} must be in the manifest"))
    }

    async fn search(
        &self,
        query: &str,
        path_prefix: Option<&str>,
        language: Option<&str>,
        visible_group_ids: Vec<i64>,
    ) -> Vec<GitCodeLexicalHit> {
        self.db
            .lexical_search_git_generation_chunks(
                self.group_id,
                &GitLexicalCodeSearch {
                    repository_key: self.repository_key,
                    query: query.to_string(),
                    path_prefix: path_prefix.map(str::to_string),
                    language: language.map(str::to_string),
                    visible_group_ids,
                    limit: 20,
                },
            )
            .await
            .expect("lexical search")
    }

    async fn cleanup(self) {
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(self.group_id)
            .execute(self.db.pool())
            .await
            .expect("clean up test group");
    }
}

/// The scratch database, or `None` when none is configured.
async fn test_database() -> Option<Database> {
    let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping git file test");
        return None;
    };
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

#[tokio::test]
async fn lexical_search_serves_the_active_generation_with_provenance() {
    let Some(fixture) = Fixture::setup("public").await else {
        return;
    };
    let visible = vec![fixture.group_id];
    let source = "fn parse_ref(raw: &str) -> SafeRef {\n    SafeRef::parse(raw)\n}\n";
    let first = fixture.start_generation(&"2".repeat(40)).await;
    fixture
        .store_and_chunk(first, &[("src/ref.rs", source, "rust")])
        .await;
    fixture.activate(first, 1).await;

    let hits = fixture
        .search("parse_ref", None, None, visible.clone())
        .await;
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.repository_key, fixture.repository_key);
    assert_eq!(hit.generation_key, first);
    assert_eq!(hit.generation_number, 1);
    assert_eq!(hit.ref_name, "refs/heads/main");
    assert_eq!(hit.commit_sha, "2".repeat(40));
    assert_eq!(
        (hit.path.as_str(), hit.language.as_str()),
        ("src/ref.rs", "rust")
    );
    assert_eq!((hit.start_line, hit.end_line), (1, 3));
    assert_eq!(hit.text, source);
    assert_eq!(hit.matched, GitCodeMatchKind::ChunkPhrase);
    assert!(hit.score > 0.0);

    // A newer building generation is not served, so a hit always describes the
    // snapshot a reader would actually be given.
    let second = fixture.start_generation(&"3".repeat(40)).await;
    fixture
        .store_and_chunk(
            second,
            &[("src/ref.rs", "fn parse_ref_unchecked() {}\n", "rust")],
        )
        .await;
    let hits = fixture
        .search("parse_ref", None, None, visible.clone())
        .await;
    assert_eq!(
        hits[0].generation_key, first,
        "a building generation is not served"
    );

    fixture.activate(second, 1).await;
    let hits = fixture.search("parse_ref", None, None, visible).await;
    assert_eq!(
        (hits.len(), hits[0].generation_key),
        (1, second),
        "the newly activated snapshot takes over"
    );

    fixture.cleanup().await;
}

#[tokio::test]
async fn lexical_search_keeps_identifiers_whole_and_escapes_wildcards() {
    let Some(fixture) = Fixture::setup("public").await else {
        return;
    };
    let generation_key = fixture.start_generation(&"4".repeat(40)).await;
    fixture
        .store_and_chunk(
            generation_key,
            &[
                ("src/a.rs", "fn parse_ref(raw: &str) {}\n", "rust"),
                ("src/b.rs", "let done = 100%;\n", "rust"),
                // A path that merely contains the digits of the query: an
                // unescaped `%` in the query would admit this row.
                ("src/percent100.rs", "fn unrelated() {}\n", "rust"),
                // Both halves of the identifier, never the identifier itself.
                ("src/d.rs", "let parse = 1;\nlet ref = 2;\n", "rust"),
            ],
        )
        .await;
    fixture.activate(generation_key, 4).await;
    let visible = vec![fixture.group_id];

    // An identifier is one token: it must not be shredded into `parse` and
    // `ref`, which would also match a chunk that merely mentions both.
    let hits = fixture
        .search("parse_ref", None, None, visible.clone())
        .await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "src/a.rs");

    // `%` in a query is a literal character, so a path holding only the digits
    // is not a match.
    let hits = fixture.search("100%", None, None, visible.clone()).await;
    assert_eq!(hits.len(), 1, "a literal percent sign matches one chunk");
    assert_eq!(hits[0].path, "src/b.rs");
    assert_eq!(
        hits[0].matched,
        GitCodeMatchKind::ChunkPhrase,
        "matched on text"
    );

    // A path that is the whole query is the strongest match. Literal
    // metacharacter queries are covered in tests/git_file_search_literals.rs.
    let hits = fixture
        .search("src/a.rs", None, None, visible.clone())
        .await;
    assert_eq!(hits[0].matched, GitCodeMatchKind::PathExact);

    fixture.cleanup().await;
}

#[tokio::test]
async fn lexical_search_honours_visibility_filters_and_group_isolation() {
    let Some(fixture) = Fixture::setup("private").await else {
        return;
    };
    let generation_key = fixture.start_generation(&"5".repeat(40)).await;
    fixture
        .store_and_chunk(
            generation_key,
            &[
                ("src/scoped.rs", "fn scoped() {}\n", "rust"),
                ("src/other.py", "def scoped():\n    pass\n", "python"),
            ],
        )
        .await;
    fixture.activate(generation_key, 2).await;

    assert!(
        fixture
            .search("scoped", None, None, vec![])
            .await
            .is_empty(),
        "a caller without the owning group must not see a private repository"
    );
    let visible = vec![fixture.group_id];
    assert_eq!(
        fixture
            .search("scoped", None, None, visible.clone())
            .await
            .len(),
        2
    );
    let hits = fixture
        .search("scoped", Some("src/o"), Some("python"), visible)
        .await;
    assert_eq!(hits.len(), 1, "path and language filters narrow the result");
    assert_eq!(hits[0].path, "src/other.py");

    // Another group can neither find this repository nor write into its generations.
    let other = Fixture::setup("public")
        .await
        .expect("scratch database is configured");
    assert!(
        other
            .search("scoped", None, None, vec![other.group_id])
            .await
            .is_empty()
    );
    assert!(
        fixture
            .db
            .replace_git_generation_files(
                other.group_id,
                fixture.repository_key,
                generation_key,
                &[manifest_entry(
                    "src/injected.rs",
                    "fn injected() {}\n",
                    "rust"
                )],
            )
            .await
            .is_err(),
        "another group must not write into this repository's generations"
    );

    other.cleanup().await;
    fixture.cleanup().await;
}
