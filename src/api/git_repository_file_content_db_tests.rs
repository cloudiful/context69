//! Database-gated round trips for the bounded line-window content read (issue
//! #681 phase 5C).
//!
//! The group, repository, and generation come from the manifest listing's shared
//! disposable fixture, so no second repository fixture exists: this suite only
//! adds the chunk rows a content read needs, on top of a building generation the
//! fixture already started, and then activates that generation the way the
//! snapshot orchestration does.
//!
//! Every test skips unless `CONTEXT69_TEST_DATABASE_URL` names a migrated
//! scratch database and then removes the groups the fixture created. Only
//! synthetic paths, line counts, and statuses are reported: no secret value or
//! provider blob id is printed or asserted.

use uuid::Uuid;

use crate::api::git_repository_files_db_fixture::{Fixture, INDEXED_COMMIT, MANIFEST_ENTRIES};
use crate::contracts::sources::{GitGenerationStatus, GitIndexStatus};
use crate::db::{
    Database, GitCheckpointUpdate, GitGenerationCoverage, NewGitGenerationChunk,
    StoredGitGenerationFile, StoredGitRepositoryGeneration,
};

use super::super::git_repository_file_content::{
    ContentPage, MAX_CHUNK_PAGE_ROWS, content_page, decode_cursor,
};
use super::super::git_repository_files::serving_generation;

/// One indexed path this suite reads.
const STORED_PATH: &str = "src/alpha.rs";

/// The chunk rows the indexed file holds: three chunks over six lines.
fn stored_lines() -> Vec<NewGitGenerationChunk> {
    vec![
        NewGitGenerationChunk {
            chunk_index: 0,
            start_line: 1,
            end_line: 2,
            text: "fn alpha() {\n    1.0;\n".to_string(),
        },
        NewGitGenerationChunk {
            chunk_index: 1,
            start_line: 3,
            end_line: 4,
            text: "fn mid() {\n    2.0;\n".to_string(),
        },
        NewGitGenerationChunk {
            chunk_index: 2,
            start_line: 5,
            end_line: 6,
            text: "fn tail() {\n    3.0;\n".to_string(),
        },
    ]
}

/// An entry to validate a window against when no real one is at hand.
///
/// The window type stays internal to the storage layer, so a caller obtains one
/// from the entry's own constructor rather than naming it.
fn scratch_entry() -> StoredGitGenerationFile {
    StoredGitGenerationFile {
        file_key: Uuid::new_v4(),
        generation_key: Uuid::new_v4(),
        repository_key: Uuid::new_v4(),
        provider_blob_sha: String::new(),
        path: STORED_PATH.to_string(),
        language: "rust".to_string(),
        byte_count: 0,
        line_count: 0,
        created_at: chrono::Utc::now(),
    }
}

/// Indexes a generation whose file carries chunk rows, and activates it.
///
/// The fixture owns the group, repository, and manifest; this supplies the chunk
/// rows through the existing storage API and then completes the same activation
/// the snapshot orchestration performs.
async fn index_with_chunks(fixture: &Fixture) -> Uuid {
    let generation_key = fixture.start_building().await;
    fixture.store_manifest(generation_key).await;
    let file = fixture
        .file(generation_key, STORED_PATH)
        .await
        .expect("the indexed manifest holds the path");
    fixture
        .db
        .replace_git_file_chunks(
            fixture.group_id,
            fixture.repository_key,
            generation_key,
            file.file_key,
            &stored_lines(),
        )
        .await
        .expect("store the chunk rows");
    fixture
        .db
        .complete_and_activate_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            generation_key,
            GitGenerationCoverage {
                file_count: MANIFEST_ENTRIES as i64,
                excluded_file_count: 0,
                total_bytes: 4096,
            },
        )
        .await
        .expect("activate the generation");
    fixture
        .db
        .update_git_repository_checkpoint(
            fixture.group_id,
            fixture.repository_key,
            &GitCheckpointUpdate {
                target_commit_sha: Some(INDEXED_COMMIT.to_string()),
                indexed_commit_sha: Some(INDEXED_COMMIT.to_string()),
                index_status: GitIndexStatus::Ready,
            },
        )
        .await
        .expect("advance the source checkpoint")
        .expect("the repository is owned by this group");
    generation_key
}

/// The serving generation and the manifest entry for the indexed path, as the
/// route resolves them.
async fn serving(fixture: &Fixture) -> (StoredGitRepositoryGeneration, StoredGitGenerationFile) {
    let source = fixture.source().await;
    let generation = serving_generation(&fixture.db, &source)
        .await
        .expect("resolve the serving generation")
        .expect("the repository serves an active generation");
    let file = fixture
        .file(generation.generation_key, STORED_PATH)
        .await
        .expect("the serving generation holds the path");
    (generation, file)
}

/// Reads one page of a window exactly as the route does, for the owning group.
async fn window_page(
    db: &Database,
    group_id: i64,
    file: &StoredGitGenerationFile,
    start_line: i32,
    end_line: i32,
    offset: i64,
) -> ContentPage {
    let window = scratch_entry()
        .line_window(start_line, end_line, 400)
        .expect("a bounded window");
    let rows = db
        .list_git_generation_chunks_in_line_range(
            group_id,
            file,
            &window,
            MAX_CHUNK_PAGE_ROWS + 1,
            offset,
        )
        .await
        .expect("read the stored window rows");
    content_page(&rows, window.start_line, window.end_line, offset)
}

#[tokio::test]
async fn a_line_window_returns_only_that_window_from_the_serving_generation() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let generation_key = index_with_chunks(&fixture).await;
    let (generation, file) = serving(&fixture).await;
    assert_eq!(generation.generation_key, generation_key);
    assert_eq!(generation.status, GitGenerationStatus::Ready);
    assert_eq!(generation.commit_sha, INDEXED_COMMIT);

    // A window inside one chunk returns that chunk's window text only.
    let first = window_page(&fixture.db, fixture.group_id, &file, 1, 2, 0).await;
    assert_eq!(first.text, "fn alpha() {\n    1.0;\n");
    assert_eq!(first.byte_count, first.text.len() as i64);
    assert!(!first.pagination.has_more);

    // A window inside the second chunk never sees the first or the third.
    let second = window_page(&fixture.db, fixture.group_id, &file, 3, 4, 0).await;
    assert_eq!(second.text, "fn mid() {\n    2.0;\n");
    assert!(!second.text.contains("alpha"));

    // A window spanning two chunks concatenates their text in order, verbatim.
    let spanning = window_page(&fixture.db, fixture.group_id, &file, 2, 3, 0).await;
    assert_eq!(spanning.text, "    1.0;\nfn mid() {\n");
    assert_eq!(
        spanning.byte_count,
        spanning.text.len() as i64,
        "the reported count is the exact UTF-8 length of the spanning text"
    );

    // A window past the stored text returns nothing, not a shorter chunk.
    let past_end = window_page(&fixture.db, fixture.group_id, &file, 50, 60, 0).await;
    assert_eq!(past_end.text, "");
    assert_eq!(past_end.byte_count, 0);

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn the_matching_chunk_rows_come_back_once_and_in_stored_order() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let _generation_key = index_with_chunks(&fixture).await;
    let (_generation, file) = serving(&fixture).await;
    let window = scratch_entry()
        .line_window(1, 6, 400)
        .expect("a bounded window");

    let whole = window_page(&fixture.db, fixture.group_id, &file, 1, 6, 0).await;
    assert_eq!(
        whole.text, "fn alpha() {\n    1.0;\nfn mid() {\n    2.0;\nfn tail() {\n    3.0;\n",
        "the window reassembles the stored file in order"
    );
    assert_eq!(whole.byte_count, whole.text.len() as i64);
    assert!(
        !whole.pagination.has_more,
        "three chunk rows fit inside one page bound"
    );

    // One row per stored chunk, in stored order, and the window reassembles from
    // the rows themselves, so continuation can repeat them without divergence.
    let rows = fixture
        .db
        .list_git_generation_chunks_in_line_range(
            fixture.group_id,
            &file,
            &window,
            MAX_CHUNK_PAGE_ROWS,
            0,
        )
        .await
        .expect("read every matching chunk row");
    assert_eq!(rows.len(), 3);
    let starts: Vec<i32> = rows.iter().map(|row| row.start_line).collect();
    assert_eq!(starts, vec![1, 3, 5], "stored order, not window order");
    let rebuilt: String = rows
        .iter()
        .map(|row| row.text_in_line_window(1, 6))
        .collect();
    assert_eq!(rebuilt, whole.text);

    // The continuation offset is the row count already served, and a window
    // that starts inside a later chunk skips the rows before it.
    assert_eq!(decode_cursor(Some("0")).expect("first continuation"), 0);
    let tail = window_page(&fixture.db, fixture.group_id, &file, 5, 6, 0).await;
    assert_eq!(
        tail.text, "fn tail() {\n    3.0;\n",
        "only the matching row"
    );
    let no_rows = window_page(&fixture.db, fixture.group_id, &file, 1, 6, 3).await;
    assert_eq!(no_rows.text, "", "an offset past the last row is empty");

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn a_foreign_group_reads_no_stored_text() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let _generation_key = index_with_chunks(&fixture).await;
    let (_generation, file) = serving(&fixture).await;
    let foreign_group_id = fixture.foreign_group().await;

    // The foreign group cannot resolve the repository at all, which is the
    // response the route gives before it would read a chunk.
    assert!(
        fixture
            .db
            .get_git_repository_source(foreign_group_id, fixture.repository_key)
            .await
            .expect("read the source as a foreign group")
            .is_none()
    );
    // And the statement itself is confined: the same file, generation, and path
    // read as another group return no rows.
    let leaked = window_page(&fixture.db, foreign_group_id, &file, 1, 6, 0).await;
    assert_eq!(
        leaked.text, "",
        "a foreign group reads no stored text ({} byte(s))",
        leaked.byte_count
    );

    // The owning group still reads its own window, so confinement is the group's
    // and not a missing manifest.
    let owned = window_page(&fixture.db, fixture.group_id, &file, 1, 6, 0).await;
    assert!(!owned.text.is_empty());
    assert_eq!(owned.byte_count, owned.text.len() as i64);

    fixture.cleanup(&[fixture.group_id, foreign_group_id]).await;
}

#[tokio::test]
async fn a_building_generation_serves_no_text() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    // A building generation can hold chunk rows, and they are readable by key,
    // but the route refuses them: only the generation the repository advertises
    // may answer.
    let building = fixture.start_building().await;
    fixture.store_manifest(building).await;
    let file = fixture
        .file(building, STORED_PATH)
        .await
        .expect("the building manifest holds the path");
    fixture
        .db
        .replace_git_file_chunks(
            fixture.group_id,
            fixture.repository_key,
            building,
            file.file_key,
            &stored_lines(),
        )
        .await
        .expect("store the chunk rows");
    let held = window_page(&fixture.db, fixture.group_id, &file, 1, 6, 0).await;
    assert!(
        !held.text.is_empty(),
        "the building generation really does hold the rows"
    );

    let source = fixture.source().await;
    assert!(
        serving_generation(&fixture.db, &source)
            .await
            .expect("resolve the serving generation")
            .is_none(),
        "a building generation is never served: {building} is not activated"
    );

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn the_statement_returns_stored_chunk_text_and_nothing_else() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let _generation_key = index_with_chunks(&fixture).await;
    let (_generation, file) = serving(&fixture).await;
    let window = scratch_entry()
        .line_window(1, 6, 400)
        .expect("a bounded window");
    let rows = fixture
        .db
        .list_git_generation_chunks_in_line_range(
            fixture.group_id,
            &file,
            &window,
            MAX_CHUNK_PAGE_ROWS,
            0,
        )
        .await
        .expect("read the window rows");
    assert!(!rows.is_empty());

    // The rows carry chunk identity and stored text only: the acquisition blob
    // and its provider blob id are never selected by this statement.
    let contract = rows[0].to_contract();
    assert_eq!(contract.file_key, file.file_key);
    let serialized = serde_json::to_string(&contract).expect("chunk contract serializes");
    assert!(
        !serialized.contains("provider_blob_sha")
            && !serialized.contains("internal/secret")
            && !serialized.contains("canonical_url"),
        "the stored chunk projection carries no blob, provider, or connection detail"
    );
    // The window text is a slice of the stored chunk text, never a rewrite.
    let trimmed = rows[0].text_in_line_window(1, 1);
    assert!(trimmed.len() < rows[0].text.len());
    assert!(rows[0].text.contains(trimmed.as_str()));

    fixture.cleanup(&[fixture.group_id]).await;
}
