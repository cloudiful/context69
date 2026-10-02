//! Tests for the `get_code` adapter (issue #681 phase 5I).
//!
//! The pure tests cover window assembly over synthetic stored chunks and the
//! argument-validation refusal surface, with no database and no provider. The
//! database-gated round trips below exercise the full guard chain against a
//! scratch database and skip unless `CONTEXT69_TEST_DATABASE_URL` is set.

use chrono::Utc;
use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;
use sqlx::Row;
use uuid::Uuid;

use super::{checked_get_args, get, window_page};
use crate::contracts::McpGetCodeRequest;
use crate::contracts::sources::{GitIndexProfile, GitProviderKind, GitRefreshPolicy};
use crate::contracts::{
    MCP_CODE_GET_CHUNK_LIMIT_DEFAULT, MCP_CODE_GET_CHUNK_LIMIT_MAX, McpGetCodeResponse,
};
use crate::db::{
    Database, GitGenerationCoverage, NewGitGenerationChunk, NewGitGenerationFile,
    NewGitRepositoryGeneration, NewGitRepositorySource, StoredGitGenerationChunk,
};
use crate::domain::AccessScope;

fn chunk(index: i32, start_line: i32, end_line: i32, text: &str) -> StoredGitGenerationChunk {
    StoredGitGenerationChunk {
        chunk_key: Uuid::new_v4(),
        generation_key: Uuid::new_v4(),
        file_key: Uuid::new_v4(),
        chunk_index: index,
        start_line,
        end_line,
        text: text.to_string(),
        created_at: Utc::now(),
    }
}

fn base_request() -> McpGetCodeRequest {
    McpGetCodeRequest {
        group_path: "research/context69".to_string(),
        repository_key: Uuid::new_v4(),
        path: "src/mcp/tools/code.rs".to_string(),
        start_line: 1,
        end_line: 20,
        chunk_limit: MCP_CODE_GET_CHUNK_LIMIT_DEFAULT,
    }
}

#[test]
fn window_page_concatenates_the_requested_lines_in_chunk_order() {
    let rows = vec![
        chunk(0, 1, 2, "line1\nline2\n"),
        chunk(1, 3, 4, "line3\nline4\n"),
    ];
    let page = window_page(&rows, 1, 3, 64);
    assert_eq!(page.text, "line1\nline2\nline3\n");
    assert_eq!(page.byte_count, page.text.len() as i64);
    assert!(!page.truncated, "the window fits the chunk and byte caps");
}

#[test]
fn window_page_preserves_crlf_trailing_whitespace_and_utf8() {
    let rows = vec![chunk(0, 1, 2, "café\r\nx \n")];
    let page = window_page(&rows, 1, 2, 64);
    assert_eq!(page.text, "café\r\nx \n", "bytes stay verbatim");
    assert_eq!(page.byte_count, page.text.len() as i64);
    assert!(!page.truncated);
}

#[test]
fn window_page_truncates_at_the_chunk_limit() {
    let rows = vec![
        chunk(0, 1, 1, "a"),
        chunk(1, 1, 1, "b"),
        chunk(2, 1, 1, "c"),
    ];
    let page = window_page(&rows, 1, 1, 2);
    assert_eq!(page.text, "ab");
    assert!(page.truncated, "the extra probe row reports the drop");
}

#[test]
fn window_page_truncates_at_the_byte_cap() {
    let piece = "x".repeat(16 * 1024);
    let rows: Vec<StoredGitGenerationChunk> =
        (0..5).map(|index| chunk(index, 1, 1, &piece)).collect();
    let page = window_page(&rows, 1, 1, 64);
    assert_eq!(page.byte_count, 64 * 1024, "four chunks fill the byte cap");
    assert_eq!(page.text.len(), 64 * 1024);
    assert!(page.truncated, "the fifth chunk crossed the byte cap");
}

#[test]
fn checked_get_args_accepts_a_bounded_safe_window() {
    assert!(checked_get_args(&base_request()).is_ok());

    let mut widest = base_request();
    widest.end_line = 400;
    assert!(checked_get_args(&widest).is_ok());
}

#[test]
fn checked_get_args_rejects_unsafe_paths_and_bad_windows() {
    let reject = |label: &str, mutate: fn(&mut McpGetCodeRequest)| {
        let mut request = base_request();
        mutate(&mut request);
        assert!(checked_get_args(&request).is_err(), "must reject {label}");
    };

    reject("blank group_path", |request| {
        request.group_path = "  ".to_string();
    });
    reject("blank path", |request| request.path = String::new());
    reject("absolute path", |request| {
        request.path = "/etc/passwd".to_string()
    });
    reject("traversing path", |request| {
        request.path = "src/../../etc".to_string()
    });
    reject("dot segment", |request| request.path = "./src".to_string());
    reject("double slash", |request| {
        request.path = "src//lib".to_string()
    });
    reject("backslash segment", |request| {
        request.path = "src\\lib".to_string()
    });
    reject("control character", |request| {
        request.path = "src/\u{1}lib".to_string()
    });
    reject("zero start_line", |request| request.start_line = 0);
    reject("zero end_line", |request| request.end_line = 0);
    reject("reversed window", |request| {
        request.start_line = 10;
        request.end_line = 5;
    });
    reject("window wider than 400 lines", |request| {
        request.start_line = 1;
        request.end_line = 401;
    });
    reject("chunk_limit below minimum", |request| {
        request.chunk_limit = 0
    });
    reject("chunk_limit above maximum", |request| {
        request.chunk_limit = MCP_CODE_GET_CHUNK_LIMIT_MAX + 1;
    });
}

// --- Disposable database round trips ---
// Skipped unless `CONTEXT69_TEST_DATABASE_URL` names a migrated scratch
// database. The test seeds synthetic rows through the public storage APIs and
// removes every group it created. Only synthetic paths, statuses, counts, and
// byte counts are asserted.

const DB_PATH: &str = "src/alpha.rs";

fn db_line(i: i32, s: i32, e: i32, text: &str) -> NewGitGenerationChunk {
    NewGitGenerationChunk {
        chunk_index: i,
        start_line: s,
        end_line: e,
        text: text.to_string(),
    }
}

// Three chunks over six lines: CRLF, trailing whitespace, and multi-byte UTF-8.
fn db_lines() -> Vec<NewGitGenerationChunk> {
    vec![
        db_line(0, 1, 2, "fn alpha() {\r\n    1.0;  \n"),
        db_line(1, 3, 4, "let café = \"é\";\n    mid();\n"),
        db_line(2, 5, 6, "fn tail() {}\n}\n"),
    ]
}

async fn scratch_db() -> Option<Database> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    let db = Database::connect(&url)
        .await
        .expect("connect scratch database");
    Some(db)
}

// A fresh private group plus one repository source.
async fn db_seed(db: &Database) -> (i64, Uuid) {
    let key = format!("mcp-get-code-{}", Uuid::new_v4());
    let group_id: i64 = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'private', 'personal', $3) RETURNING id",
    )
    .bind(&key)
    .bind("MCP Get Code Test Group")
    .bind(format!("/{key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed a test group")
    .get("id");
    let nonce = Uuid::new_v4();
    let repository_key = db
        .upsert_git_repository_source(
            group_id,
            &NewGitRepositorySource {
                connection_key: None,
                provider: GitProviderKind::Generic,
                canonical_url: format!("https://example.invalid/{nonce}.git"),
                owner: "mcp".to_string(),
                name: nonce.to_string(),
                default_branch: "main".to_string(),
                target_ref: "refs/heads/main".to_string(),
                target_commit_sha: Some("a".repeat(40)),
                index_profile: GitIndexProfile::Lexical,
                refresh_policy: GitRefreshPolicy::Manual,
            },
        )
        .await
        .expect("seed a repository source")
        .repository_key;
    (group_id, repository_key)
}

// Store the synthetic manifest and chunks in one generation and activate it when
// `activate` is set; without activation the repository serves nothing.
async fn db_index(db: &Database, group_id: i64, repository_key: Uuid, activate: bool) {
    let generation_key = db
        .start_git_repository_generation(
            group_id,
            repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: "a".repeat(40),
                index_profile: GitIndexProfile::Lexical,
            },
        )
        .await
        .expect("start a generation")
        .generation_key;
    let content: String = db_lines().iter().map(|c| c.text.as_str()).collect();
    db.replace_git_generation_files(
        group_id,
        repository_key,
        generation_key,
        &[NewGitGenerationFile {
            path: DB_PATH.to_string(),
            language: "rust".to_string(),
            provider_blob_sha: "0".repeat(40),
            content: content.as_bytes().to_vec(),
            line_count: 6,
        }],
    )
    .await
    .expect("store the manifest");
    let file = db
        .get_git_generation_file(group_id, repository_key, generation_key, DB_PATH)
        .await
        .expect("read the manifest entry")
        .expect("the manifest holds the path");
    db.replace_git_file_chunks(
        group_id,
        repository_key,
        generation_key,
        file.file_key,
        &db_lines(),
    )
    .await
    .expect("store the chunks");
    if activate {
        db.complete_and_activate_git_repository_generation(
            group_id,
            repository_key,
            generation_key,
            GitGenerationCoverage {
                file_count: 1,
                excluded_file_count: 0,
                total_bytes: content.len() as i64,
            },
        )
        .await
        .expect("activate the generation");
    }
}

async fn db_cleanup(db: &Database, group_ids: &[i64]) {
    for group_id in group_ids {
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("clean up a test group");
    }
}

fn member_scope(group_id: i64) -> AccessScope {
    AccessScope {
        user_id: Some(1),
        include_public: true,
        private_group_ids: vec![group_id],
        group_path: Some("research/mcp".to_string()),
        scoped_group_id: Some(group_id),
    }
}

async fn read(
    db: &Database,
    group_id: i64,
    scope: &AccessScope,
    repository_key: Uuid,
    path: &str,
    window: (i32, i32),
    chunk_limit: u8,
) -> Result<McpGetCodeResponse, McpError> {
    let request = McpGetCodeRequest {
        group_path: "research/mcp".to_string(),
        repository_key,
        path: path.to_string(),
        start_line: window.0,
        end_line: window.1,
        chunk_limit,
    };
    get(db, group_id, scope, &request).await
}

#[tokio::test]
async fn database_round_trips_cover_windows_and_refusals() {
    let Some(db) = scratch_db().await else {
        return;
    };
    let (gid, key) = db_seed(&db).await;
    db_index(&db, gid, key, true).await;
    let scope = member_scope(gid);
    let limit = MCP_CODE_GET_CHUNK_LIMIT_DEFAULT;

    // Ready generation: verbatim CRLF/trailing-whitespace window, exact bytes.
    let single = read(&db, gid, &scope, key, DB_PATH, (1, 2), limit).await;
    let single = single.expect("a ready generation serves the window");
    assert_eq!(single.text, "fn alpha() {\r\n    1.0;  \n");
    assert!(single.text.contains("\r\n"));
    assert!(single.text.contains("1.0;  \n"));
    assert_eq!(single.byte_count, single.text.len() as i64);
    assert!(!single.truncated);
    assert_eq!(single.file.path, DB_PATH);
    assert_eq!(single.file.language, "rust");
    assert_eq!((single.start_line, single.end_line), (1, 2));

    // A window spanning two chunks concatenates them verbatim, UTF-8 included.
    let spanning = read(&db, gid, &scope, key, DB_PATH, (2, 3), limit).await;
    let spanning = spanning.expect("the spanning window serves");
    assert_eq!(spanning.text, "    1.0;  \nlet café = \"é\";\n");
    assert!(spanning.text.contains('é'));
    assert_eq!(spanning.byte_count, spanning.text.len() as i64);
    assert!(!spanning.truncated);

    // A chunk limit below the window's chunk count truncates truthfully.
    let one = read(&db, gid, &scope, key, DB_PATH, (1, 6), 1).await;
    let one = one.expect("window with chunk_limit 1");
    assert_eq!(one.text, "fn alpha() {\r\n    1.0;  \n");
    assert!(one.truncated);

    // Unknown repository key.
    let unknown = read(&db, gid, &scope, Uuid::new_v4(), DB_PATH, (1, 2), limit).await;
    let unknown = unknown.expect_err("unknown repository key");

    // A group that does not own the repository.
    let (fgid, _) = db_seed(&db).await;
    let fscope = member_scope(fgid);
    let foreign = read(&db, fgid, &fscope, key, DB_PATH, (1, 2), limit).await;
    let foreign = foreign.expect_err("foreign group");

    // A ready generation that does not hold the requested path.
    let absent = read(&db, gid, &scope, key, "src/missing.rs", (1, 2), limit).await;
    let absent = absent.expect_err("absent path");

    // A repository with a stored manifest but no active generation.
    let (ugid, ukey) = db_seed(&db).await;
    db_index(&db, ugid, ukey, false).await;
    let uscope = member_scope(ugid);
    let unready = read(&db, ugid, &uscope, ukey, DB_PATH, (1, 2), limit).await;
    let unready = unready.expect_err("unready generation");

    // A private repository outside the caller's resolved scope.
    let mut outside = member_scope(gid);
    outside.private_group_ids.clear();
    let scoped = read(&db, gid, &outside, key, DB_PATH, (1, 2), limit).await;
    let scoped = scoped.expect_err("private group outside scope");

    for error in [&unknown, &foreign, &absent, &unready, &scoped] {
        assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
    }
    assert_eq!(unknown.message, foreign.message);
    assert_eq!(unknown.message, absent.message);
    assert_eq!(unknown.message, unready.message);
    assert_eq!(unknown.message, scoped.message);

    db_cleanup(&db, &[gid, fgid, ugid]).await;
}
