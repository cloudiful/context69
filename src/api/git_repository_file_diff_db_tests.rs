//! Database-gated round trips for the metadata-only generation comparison (issue
//! #681 phase 5E).
//!
//! The group and repository come from the shared Git fixture through this phase's
//! thin wrapper, so the suite adds no second repository fixture: it indexes two
//! generations with different synthetic commits and manifests and then compares
//! them through the same accessor the route calls.
//!
//! Every test skips unless `CONTEXT69_TEST_DATABASE_URL` names a migrated
//! scratch database and then removes the groups the fixture created. Only
//! synthetic paths, change kinds, counts, statuses, and sizes are reported: no
//! provider blob id, stored content, credential, or secret value is printed or
//! asserted, and the content addresses the comparison itself compares are never
//! read here.
//!
//! The accessor is called with unusable pairs on purpose — a foreign scope, a
//! foreign repository, a key that names nothing, a snapshot that is still
//! building — and must answer no rows rather than a one-sided diff. The reads that
//! came before this phase have their own gated tests in the same binary.

use uuid::Uuid;

use crate::api::git_repository_files_db_fixture::Fixture;
use crate::contracts::sources::{
    GitFileChangeKind, GitGenerationStatus, GitRepositoryFileDiffResponse,
};
use crate::db::StoredGitRepositorySource;

use super::super::git_repository_file_paging::requested_page;
use super::super::git_repository_files::serving_generation;
use super::fixture::{FROM_COMMIT, TO_COMMIT, index_pair, page_query};
use super::{checkpoint_match, comparable, diff_page};

/// The change kinds and paths one comparison page reports, in order.
fn observed(response: &GitRepositoryFileDiffResponse) -> Vec<(&str, GitFileChangeKind)> {
    response
        .changes
        .iter()
        .map(|change| (change.path.as_str(), change.change_kind))
        .collect()
}

/// Runs the comparison read the way the route runs it.
async fn changes(
    fixture: &Fixture,
    from: Uuid,
    to: Uuid,
    limit: i64,
    offset: i64,
) -> Vec<crate::db::StoredGitFileDiff> {
    fixture
        .db
        .list_git_generation_file_diff(
            fixture.group_id,
            fixture.repository_key,
            from,
            to,
            limit,
            offset,
        )
        .await
        .expect("read the comparison page")
}

/// The stored generation behind one key, as the route reads it back.
async fn generation_of(fixture: &Fixture, key: Uuid) -> crate::db::StoredGitRepositoryGeneration {
    fixture
        .db
        .get_git_repository_generation(fixture.group_id, fixture.repository_key, key)
        .await
        .expect("read the generation")
        .expect("the generation is stored")
}

/// One page the way the route composes it: the two named generations, the bounded
/// window, and the extra row that proves another page exists.
async fn page(
    fixture: &Fixture,
    from_key: Uuid,
    limit: u32,
    cursor: Option<&str>,
) -> GitRepositoryFileDiffResponse {
    let source: StoredGitRepositorySource = fixture.source().await;
    let from = generation_of(fixture, from_key).await;
    let to = serving_generation(&fixture.db, &source)
        .await
        .expect("resolve the serving generation")
        .expect("the repository serves an active generation");
    let request = requested_page(&page_query(limit, cursor)).expect("a bounded page");
    let mut rows = changes(
        fixture,
        from.generation_key,
        to.generation_key,
        request.fetch_limit(),
        request.offset,
    )
    .await;
    let has_more = rows.len() as i64 > request.limit();
    if has_more {
        rows.truncate(request.limit() as usize);
    }
    diff_page(&source, &from, &to, rows, &request, has_more)
}

#[tokio::test]
async fn add_modify_and_delete_are_classified_and_unchanged_paths_are_omitted() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let (from, to) = index_pair(&fixture).await;
    let response = page(&fixture, from, 50, None).await;
    assert_eq!(response.from_generation_key, from);
    assert_eq!(response.to_generation_key, to);
    assert_eq!(response.from_commit_sha, FROM_COMMIT);
    assert_eq!(response.to_commit_sha, TO_COMMIT);
    assert_ne!(response.from_commit_sha, response.to_commit_sha);
    assert!(
        response.to_generation_number > response.from_generation_number,
        "the endpoint is the newer generation"
    );

    // The two generations share three paths and differ on each of the other two:
    // one rewritten, one added, one dropped. A path whose stored bytes are
    // identical is not a change and must not appear.
    assert_eq!(
        observed(&response),
        vec![
            ("src/added.rs", GitFileChangeKind::Added),
            ("src/alpha.rs", GitFileChangeKind::Modified),
            ("src/dropped.rs", GitFileChangeKind::Deleted),
        ],
        "path order is deterministic and each kind owns its side"
    );
    for change in &response.changes {
        match change.change_kind {
            GitFileChangeKind::Added => {
                assert!(change.before.is_none() && change.after.is_some());
            }
            GitFileChangeKind::Deleted => {
                assert!(change.after.is_none() && change.before.is_some());
            }
            GitFileChangeKind::Modified => {
                let before = change
                    .before
                    .as_ref()
                    .expect("a modified path has both sides");
                let after = change
                    .after
                    .as_ref()
                    .expect("a modified path has both sides");
                assert_ne!(before.file_key, after.file_key, "each side names its entry");
                assert_ne!(
                    (before.byte_count, before.line_count),
                    (after.byte_count, after.line_count),
                    "a modified path stores different bytes"
                );
                assert_eq!(
                    (before.language.as_str(), after.language.as_str()),
                    ("rust", "rust")
                );
            }
        }
        let serialized = serde_json::to_string(change).expect("serialize one change");
        for forbidden in ["blob", "content", "text", "chunk", "secret"] {
            assert!(
                !serialized.contains(forbidden),
                "leaked {forbidden}: {serialized}"
            );
        }
    }

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn an_identical_pair_is_empty_and_the_reverse_order_is_a_conflict() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let (from, to) = index_pair(&fixture).await;
    // A generation holds the same bytes as itself, so the comparison is empty
    // rather than a conflict.
    let same = page(&fixture, to, 50, None).await;
    assert_eq!(same.from_generation_key, same.to_generation_key);
    assert!(same.changes.is_empty() && same.pagination.is_terminal());
    assert!(changes(&fixture, to, to, 10, 0).await.is_empty());

    // The other direction is refused by the order rule, not answered as empty.
    let older = generation_of(&fixture, from).await;
    let newer = generation_of(&fixture, to).await;
    assert!(comparable(&older, &newer).is_ok());
    assert!(comparable(&newer, &older).is_err());
    assert!(comparable(&newer, &newer).is_ok());

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn the_default_pair_resolves_the_superseded_checkpoint_to_the_active_generation() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let (from, to) = index_pair(&fixture).await;
    let source = fixture.source().await;
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(FROM_COMMIT),
        "the checkpoint names the compared-from commit"
    );
    // Activating the newer generation is what supersedes the older one, so the
    // generation this checkpoint names is the historical, completed snapshot a
    // default comparison starts from.
    let stored_from = generation_of(&fixture, from).await;
    assert_eq!(
        stored_from.status,
        GitGenerationStatus::Superseded,
        "the indexed generation is completed and superseded, never discarded"
    );
    let generations = fixture
        .db
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("list the repository's generations");
    let resolved = checkpoint_match(
        &generations,
        source.checkpoint.indexed_commit_sha.as_deref(),
    )
    .expect("the checkpoint resolves to a stored generation");
    let serving = serving_generation(&fixture.db, &source)
        .await
        .expect("resolve the serving generation")
        .expect("the repository serves the newer generation");
    assert_eq!(
        resolved.generation_key, from,
        "the default start is the checkpoint"
    );
    assert_eq!(
        serving.generation_key, to,
        "the default end is the active generation"
    );
    assert!(comparable(resolved, &serving).is_ok());
    // The explicit pair and the default pair are the same comparison.
    assert_eq!(observed(&page(&fixture, from, 50, None).await).len(), 3);

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn a_foreign_repository_or_unknown_generation_compares_nothing() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let (from, to) = index_pair(&fixture).await;
    let foreign_group_id = fixture.foreign_group().await;
    let foreign_repository = fixture.repository_for(foreign_group_id).await;
    let unknown = Uuid::new_v4();

    // Another group's scope reads no row of this repository, a repository of
    // another group is not this repository, and a key that names no generation
    // makes the pair ineligible: all three are empty, never someone else's diff
    // and never the other side's paths reported as added or deleted.
    let foreign_scope = fixture
        .db
        .list_git_generation_file_diff(foreign_group_id, fixture.repository_key, from, to, 10, 0)
        .await
        .expect("read as a foreign group");
    assert!(foreign_scope.is_empty(), "a foreign group reads no change");
    let foreign_repository_rows = fixture
        .db
        .list_git_generation_file_diff(fixture.group_id, foreign_repository, from, to, 10, 0)
        .await
        .expect("read a repository of another group");
    assert!(
        foreign_repository_rows.is_empty(),
        "another group's repository is not comparable here"
    );
    for (start, end) in [(from, unknown), (unknown, to), (unknown, unknown)] {
        assert!(
            changes(&fixture, start, end, 10, 0).await.is_empty(),
            "a generation key that names nothing compares nothing"
        );
    }
    assert_eq!(
        changes(&fixture, from, to, 10, 0).await.len(),
        3,
        "the owner reads its own"
    );

    fixture.cleanup(&[fixture.group_id, foreign_group_id]).await;
}

#[tokio::test]
async fn a_building_generation_is_never_compared() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let (_, to) = index_pair(&fixture).await;
    // A building generation can hold manifest rows, but it is not a completed
    // snapshot, so the pair is ineligible in the statement and the comparison
    // contributes nothing rather than a partial or invented diff; the route
    // refuses the key outright before it ever reaches the read.
    let building = fixture.start_building().await;
    fixture.store_manifest(building).await;
    let response = page(&fixture, building, 50, None).await;
    assert_eq!(
        response.from_generation_key, building,
        "the page still names the generation the caller asked for"
    );
    assert!(
        response.changes.is_empty(),
        "a building generation has no comparable manifest"
    );
    // The repository still serves the activated generation, so the default pair
    // is unchanged.
    let source = fixture.source().await;
    assert_eq!(
        serving_generation(&fixture.db, &source)
            .await
            .expect("resolve the serving generation")
            .expect("the repository serves the newer generation")
            .generation_key,
        to
    );

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn the_window_continues_without_repeating_or_skipping_a_path() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let (from, to) = index_pair(&fixture).await;
    let first = page(&fixture, from, 2, None).await;
    assert_eq!(
        first.changes.len(),
        2,
        "the probe row is not a returned change"
    );
    assert!(first.pagination.has_more);
    let cursor = first
        .pagination
        .next_cursor
        .clone()
        .expect("has_more carries its continuation token");
    let second = page(&fixture, from, 2, Some(&cursor)).await;
    assert_eq!(
        second.changes.len(),
        1,
        "the window continues where it stopped"
    );
    assert!(second.pagination.is_terminal());

    let mut walked: Vec<&str> = first
        .changes
        .iter()
        .chain(second.changes.iter())
        .map(|change| change.path.as_str())
        .collect();
    walked.sort_unstable();
    let all = changes(&fixture, from, to, 50, 0).await;
    let mut expected: Vec<&str> = all.iter().map(|diff| diff.path.as_str()).collect();
    expected.sort_unstable();
    assert_eq!(
        walked, expected,
        "every changed path is returned exactly once"
    );

    // A window outside the shared bounds is refused, never silently clamped.
    for (limit, offset) in [(0, 0), (10_000, 0), (10, -1)] {
        let error = fixture
            .db
            .list_git_generation_file_diff(
                fixture.group_id,
                fixture.repository_key,
                from,
                to,
                limit,
                offset,
            )
            .await
            .expect_err("an out-of-bounds window must be refused")
            .to_string();
        assert!(error.contains("git_page_out_of_bounds"), "{error}");
    }

    fixture.cleanup(&[fixture.group_id]).await;
}
