//! Generation-scoped storage of exact Git code content (issue #681 work unit
//! 3B2).
//!
//! These exercise what only a database can answer about storing code: byte
//! deduplication per generation, retirement of what a new manifest drops, the
//! refusal of an inconsistent manifest, the byte ceilings, verbatim chunks with
//! inclusive line ranges, and writable-only-while-building. Reading code back is
//! covered in `tests/git_file_search.rs`, and both run only when
//! CONTEXT69_TEST_DATABASE_URL names a scratch database.

use context69::contracts::sources::{GitIndexProfile, GitProviderKind, GitRefreshPolicy};
use context69::db::{
    Database, GitGenerationCoverage, NewGitGenerationChunk, NewGitGenerationFile,
    NewGitRepositoryGeneration, NewGitRepositorySource, StoredGitGenerationFile,
};
use sqlx::Row;
use uuid::Uuid;

const MIB: usize = 1024 * 1024;
const COUNT_FILES: &str =
    "SELECT count(*) FROM context69.git_generation_files WHERE generation_key = $1";
const SUM_BLOB_BYTES: &str = "SELECT COALESCE(SUM(byte_count), 0)::bigint FROM context69.git_generation_blobs WHERE generation_key = $1";

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
    /// A group and a repository, or `None` when no scratch database is
    /// configured, which skips the calling test.
    async fn setup(visibility: &str) -> Option<Self> {
        let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping git file test");
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

    /// Stores a whole manifest of (path, content, language) triples.
    async fn store_manifest(
        &self,
        generation_key: Uuid,
        files: &[(&str, &str, &str)],
    ) -> anyhow::Result<context69::db::GitManifestReplacement> {
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
    }

    async fn file(&self, generation_key: Uuid, path: &str) -> StoredGitGenerationFile {
        self.db
            .get_git_generation_file(self.group_id, self.repository_key, generation_key, path)
            .await
            .expect("read manifest entry")
            .unwrap_or_else(|| panic!("{path} must be in the manifest"))
    }

    async fn count(&self, sql: &'static str, generation_key: Uuid) -> i64 {
        sqlx::query(sql)
            .bind(generation_key)
            .fetch_one(self.db.pool())
            .await
            .expect("count stored rows")
            .get::<i64, _>(0)
    }

    async fn cleanup(self) {
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(self.group_id)
            .execute(self.db.pool())
            .await
            .expect("clean up test group");
    }
}

#[tokio::test]
async fn manifest_replacement_deduplicates_bytes_and_retires_what_it_drops() {
    let Some(fixture) = Fixture::setup("public").await else {
        return;
    };
    let generation_key = fixture.start_generation(&"b".repeat(40)).await;
    let shared = "fn shared() {}\n";

    // The same bytes at two paths cost one row, and the declared size is the
    // stored size.
    let reported = fixture
        .store_manifest(
            generation_key,
            &[
                ("src/one.rs", shared, "rust"),
                ("src/two.rs", shared, "rust"),
                ("src/dropped.rs", "fn dropped() {}\n", "rust"),
            ],
        )
        .await
        .expect("store the first manifest");
    assert_eq!(
        reported.stored_blob_count, 2,
        "identical bytes are stored once per generation, not once per path"
    );
    let stored = fixture.file(generation_key, "src/one.rs").await;
    assert_eq!(stored.provider_blob_sha, blob_sha(shared));
    assert_eq!(stored.byte_count, shared.len() as i64);
    let dropped = fixture.file(generation_key, "src/dropped.rs").await;

    // Re-sending the unchanged file while another path changes content: the
    // unchanged row keeps its identity, and the changed one is retired whole
    // with the chunks cut from it.
    let reported = fixture
        .store_manifest(
            generation_key,
            &[
                ("src/one.rs", shared, "rust"),
                ("src/dropped.rs", "fn changed() {}\n", "rust"),
            ],
        )
        .await
        .expect("store the second manifest");
    assert_eq!(reported.stored_blob_count, 1, "only new bytes are stored");
    assert_eq!(
        reported.retired_file_count, 2,
        "the omitted and changed paths both retire"
    );
    assert_eq!(
        reported.retired_blob_count, 1,
        "its unreferenced bytes leave"
    );
    assert_eq!(
        fixture.file(generation_key, "src/one.rs").await.file_key,
        stored.file_key,
        "an unchanged entry keeps its identity"
    );
    assert_ne!(
        fixture
            .file(generation_key, "src/dropped.rs")
            .await
            .file_key,
        dropped.file_key
    );
    assert_eq!(
        fixture.count(COUNT_FILES, generation_key).await,
        2,
        "a path the manifest no longer describes is gone"
    );

    // A manifest that cannot describe one snapshot is refused and stores
    // nothing: one content-addressed id, src/one.rs's, mapped to two byte strings.
    let mut conflicting = manifest_entry("src/three.rs", "fn different() {}\n", "rust");
    conflicting.provider_blob_sha = blob_sha(shared);
    assert!(
        fixture
            .db
            .replace_git_generation_files(
                fixture.group_id,
                fixture.repository_key,
                generation_key,
                &[manifest_entry("src/one.rs", shared, "rust"), conflicting,],
            )
            .await
            .is_err(),
        "an inconsistent manifest must be refused"
    );
    assert_eq!(fixture.count(COUNT_FILES, generation_key).await, 2);

    fixture.cleanup().await;
}

#[tokio::test]
async fn the_database_refuses_content_beyond_the_stored_byte_ceilings() {
    let Some(fixture) = Fixture::setup("public").await else {
        return;
    };
    let generation_key = fixture.start_generation(&"e".repeat(40)).await;
    // A raw blob insert bypasses the storage layer, so the ceilings under test
    // are the database's own.
    let insert = |sha: String, bytes: usize| {
        sqlx::query("INSERT INTO context69.git_generation_blobs (generation_key, provider_blob_sha, byte_count, content) VALUES ($1, $2, $3, decode(repeat('61', $3), 'hex'))")
            .bind(generation_key)
            .bind(sha)
            .bind(bytes as i32)
            .execute(fixture.db.pool())
    };
    assert!(
        insert(format!("{:040x}", 1), 8 * MIB + 1).await.is_err(),
        "one byte over the per-file ceiling must be refused"
    );
    // Eight 8 MiB blobs fill the 64 MiB per-generation ceiling exactly.
    for index in 2..=9_u64 {
        insert(format!("{index:040x}"), 8 * MIB)
            .await
            .unwrap_or_else(|error| panic!("blob {index} must be accepted: {error}"));
    }
    assert!(
        insert(format!("{:040x}", 10), 8 * MIB).await.is_err(),
        "the ninth 8 MiB blob crosses the per-generation ceiling"
    );
    assert_eq!(
        fixture.count(SUM_BLOB_BYTES, generation_key).await,
        64 * MIB as i64,
        "storage stops at the ceiling"
    );

    fixture.cleanup().await;
}

#[tokio::test]
async fn chunks_round_trip_verbatim_until_the_generation_is_activated() {
    let Some(fixture) = Fixture::setup("public").await else {
        return;
    };
    let generation_key = fixture.start_generation(&"1".repeat(40)).await;
    // Mixed indentation, trailing spaces, a CRLF line, and a final line without
    // a newline: all of it must survive storage unchanged.
    let source = "fn main() {\r\n\tlet x = 1;   \n\tprintln!(\"{x}\");\n}";
    fixture
        .store_manifest(generation_key, &[("src/main.rs", source, "rust")])
        .await
        .expect("a building generation accepts content");
    let file = fixture.file(generation_key, "src/main.rs").await;
    let written = vec![
        chunk(0, 1, 2, "fn main() {\r\n\tlet x = 1;   \n"),
        chunk(1, 3, 4, "\tprintln!(\"{x}\");\n}"),
    ];
    fixture
        .db
        .replace_git_file_chunks(
            fixture.group_id,
            fixture.repository_key,
            generation_key,
            file.file_key,
            &written,
        )
        .await
        .expect("write chunks");
    let stored = fixture
        .db
        .list_git_generation_chunks(
            fixture.group_id,
            fixture.repository_key,
            generation_key,
            file.file_key,
            100,
            0,
        )
        .await
        .expect("list chunks");
    assert_eq!(stored.len(), written.len());
    for (position, stored_chunk) in stored.iter().enumerate() {
        assert_eq!(stored_chunk.chunk_index, position as i32);
        assert_eq!(
            (stored_chunk.start_line, stored_chunk.end_line),
            (written[position].start_line, written[position].end_line),
            "each chunk keeps its inclusive line range"
        );
    }
    assert_eq!(
        stored
            .iter()
            .map(|stored_chunk| stored_chunk.text.as_str())
            .collect::<String>(),
        source,
        "concatenating the stored chunks reproduces the file byte for byte"
    );

    // An activated generation is immutable by construction, and the write path
    // refuses one; the next snapshot still starts empty and writable.
    fixture
        .db
        .complete_and_activate_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            generation_key,
            GitGenerationCoverage {
                file_count: 1,
                excluded_file_count: 0,
                total_bytes: 0,
            },
        )
        .await
        .expect("activate generation");
    assert!(
        fixture
            .db
            .replace_git_file_chunks(
                fixture.group_id,
                fixture.repository_key,
                generation_key,
                file.file_key,
                &[chunk(0, 1, 1, "fn late() {}\n")],
            )
            .await
            .is_err(),
        "an activated generation accepts no further content"
    );
    let next = fixture.start_generation(&"0".repeat(40)).await;
    assert!(
        fixture
            .store_manifest(next, &[("src/main.rs", source, "rust")])
            .await
            .is_ok(),
        "a new generation starts empty and writable"
    );

    fixture.cleanup().await;
}
