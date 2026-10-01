//! LIKE metacharacters in a code query must stay literal (issue #681 work unit
//! 3B2, checkpoint repair 2).
//!
//! Every `LIKE` pattern the lexical statement builds — the chunk branch and the
//! path branch alike — is built from the escaped phrase, and the raw query text
//! is used only for the exact path comparison, so a `%`, `_`, or `\` in a query
//! looks for that character instead of acting as a wildcard.
//!
//! A query matches in one of three ways, and these tests keep them apart: the
//! whole phrase in a chunk (`chunk_phrase`), the whole phrase in a path
//! (`path_phrase`), or every identifier token of the query present in a chunk
//! (`chunk_terms`). Digits and `_` are identifier characters, so a query like
//! `100%` still tokenises to `100` and legitimately matches code containing it;
//! what it must never do is admit a path that merely holds those characters as a
//! `path_phrase`, which is what an unescaped wildcard would do. Every assertion
//! below therefore checks the path *and* the reported reason, and the first test
//! proves each fixture row is reachable so an empty result is a property of the
//! query rather than of an empty index.
//!
//! They run only when CONTEXT69_TEST_DATABASE_URL names a scratch database
//! (migrations are applied automatically), and are skipped otherwise.

use context69::contracts::sources::{
    GitCodeMatchKind, GitIndexProfile, GitProviderKind, GitRefreshPolicy,
};
use context69::db::{
    Database, GitGenerationCoverage, GitLexicalCodeSearch, NewGitGenerationChunk,
    NewGitGenerationFile, NewGitRepositoryGeneration, NewGitRepositorySource,
};
use sqlx::Row;
use uuid::Uuid;

/// Provider blob id for test content: the database deduplicates on ids, so
/// equal bytes must always produce an equal one.
fn blob_sha(content: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:040x}")
}

fn manifest_entry(path: &str, content: &str) -> NewGitGenerationFile {
    NewGitGenerationFile {
        path: path.to_string(),
        language: "rust".to_string(),
        provider_blob_sha: blob_sha(content),
        content: content.as_bytes().to_vec(),
        line_count: content.split_inclusive('\n').count() as i64,
    }
}

/// One group, one repository, and the generations indexed into them.
struct Fixture {
    db: Database,
    group_id: i64,
    repository_key: Uuid,
}

impl Fixture {
    /// A group and a repository, or `None` when no scratch database is
    /// configured, which skips the calling test.
    async fn setup(visibility: &str) -> Option<Self> {
        let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
            eprintln!(
                "CONTEXT69_TEST_DATABASE_URL is not set; skipping git file search literal test"
            );
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

    /// Starts a generation, stores `files` as the whole manifest with one chunk
    /// each, and activates it so the lexical query can answer.
    async fn store_and_activate(&self, commit: &str, files: &[(&str, &str)]) {
        let generation_key = self
            .db
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
            .generation_key;
        self.db
            .replace_git_generation_files(
                self.group_id,
                self.repository_key,
                generation_key,
                &files
                    .iter()
                    .map(|(path, content)| manifest_entry(path, content))
                    .collect::<Vec<_>>(),
            )
            .await
            .expect("store the manifest");
        for (path, content) in files {
            self.db
                .replace_git_file_chunks(
                    self.group_id,
                    self.repository_key,
                    generation_key,
                    self.file_key(generation_key, path).await,
                    &[NewGitGenerationChunk {
                        chunk_index: 0,
                        start_line: 1,
                        end_line: 1,
                        text: (*content).to_string(),
                    }],
                )
                .await
                .expect("write chunks");
        }
        self.db
            .complete_and_activate_git_repository_generation(
                self.group_id,
                self.repository_key,
                generation_key,
                GitGenerationCoverage {
                    file_count: files.len() as i64,
                    excluded_file_count: 0,
                    total_bytes: 0,
                },
            )
            .await
            .expect("activate generation");
    }

    async fn file_key(&self, generation_key: Uuid, path: &str) -> Uuid {
        self.db
            .get_git_generation_file(self.group_id, self.repository_key, generation_key, path)
            .await
            .expect("read manifest entry")
            .unwrap_or_else(|| panic!("{path} must be in the manifest"))
            .file_key
    }

    /// The `(path, why it matched)` pairs a query returns, sorted by path.
    async fn search(&self, query: &str) -> Vec<(String, GitCodeMatchKind)> {
        let mut hits = self
            .db
            .lexical_search_git_generation_chunks(
                self.group_id,
                &GitLexicalCodeSearch {
                    repository_key: self.repository_key,
                    query: query.to_string(),
                    path_prefix: None,
                    language: None,
                    visible_group_ids: vec![self.group_id],
                    limit: 50,
                },
            )
            .await
            .expect("lexical search")
            .into_iter()
            .map(|hit| (hit.path, hit.matched))
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| left.0.cmp(&right.0));
        hits
    }

    async fn cleanup(self) {
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(self.group_id)
            .execute(self.db.pool())
            .await
            .expect("clean up test group");
    }
}

/// Five files, so a wildcard would have something to widen to: one carries a
/// literal percent sign, one a literal underscore, one holds only the digits
/// `100`, and two carry none of them.
const FILES: [(&str, &str); 5] = [
    ("src/percent.rs", "let done = 100%;\n"),
    ("src/under_score.rs", "let done = 100_0;\n"),
    ("src/percent100.rs", "fn unrelated() {}\n"),
    ("src/plain.rs", "fn plain() {}\n"),
    ("src/other.rs", "fn other() {}\n"),
];

/// The exact rows a query returns, so a wildcard false positive is visibly
/// different from a legitimate phrase or token hit.
fn assert_hits(
    query: &str,
    hits: &[(String, GitCodeMatchKind)],
    expected: &[(&str, GitCodeMatchKind)],
) {
    let expected = expected
        .iter()
        .map(|(path, matched)| ((*path).to_string(), *matched))
        .collect::<Vec<_>>();
    assert_eq!(hits, &expected, "`{query}` returned the wrong rows");
}

/// Every fixture row answers a control query, so no assertion below can pass for
/// the wrong reason: an unreadable snapshot answers nothing at all.
#[tokio::test]
async fn every_fixture_row_is_searchable() {
    let Some(fixture) = Fixture::setup("public").await else {
        return;
    };
    fixture.store_and_activate(&"6".repeat(40), &FILES).await;

    // The control words are chosen to appear in chunk text and in no path, so
    // each control is a plain `chunk_phrase` with nothing to interpret.
    assert_hits(
        "unrelated",
        &fixture.search("unrelated").await,
        &[("src/percent100.rs", GitCodeMatchKind::ChunkPhrase)],
    );
    assert_hits(
        "done",
        &fixture.search("done").await,
        &[
            ("src/percent.rs", GitCodeMatchKind::ChunkPhrase),
            ("src/under_score.rs", GitCodeMatchKind::ChunkPhrase),
        ],
    );
    // `fn` is in three of the chunks and in no path, which covers the remaining
    // rows and shows a multi-row result as well.
    assert_hits(
        "fn",
        &fixture.search("fn").await,
        &[
            ("src/other.rs", GitCodeMatchKind::ChunkPhrase),
            ("src/percent100.rs", GitCodeMatchKind::ChunkPhrase),
            ("src/plain.rs", GitCodeMatchKind::ChunkPhrase),
        ],
    );

    fixture.cleanup().await;
}

/// A `%` in the query looks for a percent sign. The path that holds only the
/// digits is absent, which is the whole regression: unescaped, it would be
/// admitted as a `path_phrase` wildcard hit.
#[tokio::test]
async fn a_percent_sign_never_reaches_the_path_branch_as_a_wildcard() {
    let Some(fixture) = Fixture::setup("public").await else {
        return;
    };
    fixture.store_and_activate(&"7".repeat(40), &FILES).await;

    assert_hits(
        "100%",
        &fixture.search("100%").await,
        &[
            // The literal phrase `100%`, in the chunk that really contains it.
            ("src/percent.rs", GitCodeMatchKind::ChunkPhrase),
            // The identifier token `100`, which legitimately matches this code
            // even though its phrase does not.
            ("src/under_score.rs", GitCodeMatchKind::ChunkTerms),
        ],
    );
    assert_hits(
        "100_0",
        &fixture.search("100_0").await,
        &[("src/under_score.rs", GitCodeMatchKind::ChunkPhrase)],
    );

    // A bare percent sign matches the one row that contains one, not the four
    // rows an unescaped `LIKE '%%%'` would admit as `path_phrase`.
    assert_hits(
        "%",
        &fixture.search("%").await,
        &[("src/percent.rs", GitCodeMatchKind::ChunkPhrase)],
    );

    fixture.cleanup().await;
}

/// The phrase and the identifier tokens are separate matching paths, and a
/// metacharacter in either stays a literal character.
#[tokio::test]
async fn literal_metacharacters_and_identifier_tokens_stay_separate() {
    let Some(fixture) = Fixture::setup("public").await else {
        return;
    };
    fixture.store_and_activate(&"8".repeat(40), &FILES).await;

    // An underscore is an identifier character, so it matches the path that
    // really contains one: a valid literal `path_phrase`, not a wildcard.
    assert_hits(
        "_",
        &fixture.search("_").await,
        &[("src/under_score.rs", GitCodeMatchKind::PathPhrase)],
    );
    // `100%%` has no literal occurrence anywhere; only its `100` token matches.
    assert_hits(
        "100%%",
        &fixture.search("100%%").await,
        &[
            ("src/percent.rs", GitCodeMatchKind::ChunkTerms),
            ("src/under_score.rs", GitCodeMatchKind::ChunkTerms),
        ],
    );
    // `%_%` tokenises to the escaped underscore, so the same row answers on the
    // token while the literal `%_` phrase matches nothing.
    assert_hits(
        "%_%",
        &fixture.search("%_%").await,
        &[("src/under_score.rs", GitCodeMatchKind::ChunkTerms)],
    );
    // A backslash is neither an identifier character nor present in any row, so
    // there is neither a phrase hit nor a token to fall back on.
    assert_hits("\\", &fixture.search("\\").await, &[]);

    fixture.cleanup().await;
}
