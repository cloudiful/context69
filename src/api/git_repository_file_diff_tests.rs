//! Focused tests for the metadata-only generation comparison (issue #681 phase
//! 5E).
//!
//! The page is composed from the resolved source, the two resolved generations,
//! and the rows the comparison returned, so these tests build the stored types in
//! memory and never touch a database, a network, or a secret. They pin the
//! decisions this route owns: the shared Viewer floor, the checkpoint-to-active
//! default resolution, the group-confined `409`, the compare-order rule, the
//! change-kind projection, and the continuation window. The wire shape of the
//! response itself is pinned in the contract crate's own tests.

use axum::http::StatusCode;

use crate::contracts::{
    MembershipRole,
    sources::{GitFileChangeKind, GitGenerationStatus, GitIndexStatus, GitRepositoryFileDiffQuery},
};

use super::super::git_repository_file_paging::requested_page;
use super::super::git_repository_files::{
    generation_not_ready, repository_not_found, require_manifest_read,
};
use super::fixture::{
    FROM_COMMIT, INDEXED_COMMIT, TO_COMMIT, changed, file_key, from_generation, from_key,
    generation, group, page, page_query, repository_key, response_body, source, to_generation,
    to_key,
};
use super::{checkpoint_match, comparable, found_generation, found_repository};

/// The refusal text one invalid page request produces.
fn refusal(limit: u32, cursor: Option<&str>) -> String {
    requested_page(&page_query(limit, cursor))
        .expect_err("an invalid page request is refused")
        .to_string()
}

#[test]
fn the_comparison_shares_the_manifest_viewer_floor() {
    for role in [
        MembershipRole::Viewer,
        MembershipRole::Maintainer,
        MembershipRole::Owner,
    ] {
        require_manifest_read(&group(Some(role)))
            .unwrap_or_else(|error| panic!("{role:?} may compare generations: {error}"));
    }
    assert!(
        require_manifest_read(&group(None)).is_err(),
        "a caller with no membership is refused before any comparison runs"
    );
}

#[test]
fn the_default_starting_point_is_the_newest_completed_generation_of_the_indexed_commit() {
    let generations = vec![
        // Newest first, as the storage layer returns them.
        generation(
            9,
            to_key(),
            "9999999999999999999999999999999999999999",
            1,
            0,
            1,
        ),
        to_generation(),
        from_generation(),
    ];
    assert_eq!(
        checkpoint_match(&generations, Some(INDEXED_COMMIT))
            .map(|generation| generation.generation_key),
        Some(from_key()),
        "the checkpoint's own commit resolves to that generation, not a newer one"
    );
    assert_eq!(
        checkpoint_match(&generations, Some(TO_COMMIT)).map(|generation| generation.generation_key),
        Some(to_key())
    );
    assert_eq!(
        checkpoint_match(
            &generations,
            Some("0000000000000000000000000000000000000000")
        ),
        None,
        "a checkpoint no stored generation covers has nothing to compare from"
    );
    assert_eq!(checkpoint_match(&generations, None), None);
    assert_eq!(checkpoint_match(&[], Some(INDEXED_COMMIT)), None);
    // The generation a checkpoint names is normally the one a newer snapshot
    // superseded, so that state resolves: this is the whole point of the default
    // pair. A snapshot that is still building or has failed never resolves, even
    // when its commit matches, because its manifest is not comparable.
    let mut superseded = from_generation();
    superseded.status = GitGenerationStatus::Superseded;
    assert_eq!(
        checkpoint_match(&[superseded], Some(INDEXED_COMMIT))
            .map(|generation| generation.generation_key),
        Some(from_key()),
        "the superseded generation the checkpoint names is the default start"
    );
    for status in [GitGenerationStatus::Building, GitGenerationStatus::Failed] {
        let mut incomplete = from_generation();
        incomplete.status = status;
        assert_eq!(
            checkpoint_match(&[incomplete], Some(INDEXED_COMMIT)),
            None,
            "{status:?} is never compared against"
        );
    }
    // The source the route reads carries the checkpoint the default resolves.
    assert_eq!(
        source().checkpoint.indexed_commit_sha.as_deref(),
        Some(INDEXED_COMMIT)
    );
    assert_eq!(from_generation().commit_sha, FROM_COMMIT);
    assert_eq!(to_generation().commit_sha, TO_COMMIT);
}

#[tokio::test]
async fn an_unresolvable_repository_answers_exactly_as_an_unknown_repository() {
    // The handler maps the group-scoped lookup through `found_repository`, so this
    // exercises the arm the route actually takes: a drift back to a distinguishable
    // "no such repository for this group" shape fails here.
    let unknown = repository_not_found();
    let unknown_status = unknown.status();
    let unknown_body = response_body(unknown).await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    assert!(
        unknown_body.contains("unknown git repository"),
        "the shared shape carries the bounded message: {unknown_body}"
    );
    let absent = *found_repository(Ok(None)).expect_err("no source is not a source");
    assert_eq!(absent.status(), unknown_status, "the status must be shared");
    assert_eq!(
        response_body(absent).await,
        unknown_body,
        "an unresolvable repository must not be distinguishable by body either"
    );
    assert!(!unknown_body.contains(&repository_key().to_string()));
    for detail in ["github", "cloudiful", "team-platform"] {
        assert!(!unknown_body.contains(detail), "the 404 echoes {detail}");
    }
    let stored = found_repository(Ok(Some(source()))).expect("a stored source");
    assert_eq!(stored.repository_key, repository_key());
    assert_eq!(stored.group.group_id, 42, "the owning group is unchanged");
}

#[tokio::test]
async fn an_unresolvable_generation_answers_exactly_as_a_missing_one() {
    // The handler maps both sides through `found_generation`, so this exercises
    // the arm the route actually takes for an explicit key, a foreign key, and a
    // key that names a snapshot which is not ready.
    let not_ready = generation_not_ready();
    let not_ready_status = not_ready.status();
    let not_ready_body = response_body(not_ready).await;
    assert_eq!(not_ready_status, StatusCode::CONFLICT);
    assert!(
        not_ready_body.contains("no active index generation"),
        "the shared shape carries the bounded message: {not_ready_body}"
    );

    let absent = *found_generation(Ok(None)).expect_err("no generation is not one");
    assert_eq!(
        absent.status(),
        not_ready_status,
        "the status must be shared"
    );
    assert_eq!(
        response_body(absent).await,
        not_ready_body,
        "an unresolvable generation must not be distinguishable by body either"
    );
    let echoed = to_key().to_string();
    assert!(
        !not_ready_body.contains(&echoed) && !not_ready_body.contains("commit"),
        "the 409 must not echo generation detail: {not_ready_body}"
    );

    // The confined read answers with a row of every lifecycle status, so a key
    // that names a snapshot which is not comparable is refused exactly like a key
    // that names nothing: a partial or abandoned snapshot never answers, and the
    // refusal does not reveal which status it was.
    for status in [GitGenerationStatus::Building, GitGenerationStatus::Failed] {
        let mut partial = from_generation();
        partial.status = status;
        let refused = *found_generation(Ok(Some(partial))).expect_err("not comparable is refused");
        assert_eq!(refused.status(), not_ready_status, "{status:?} status");
        assert_eq!(
            response_body(refused).await,
            not_ready_body,
            "{status:?} must be indistinguishable from a missing generation"
        );
    }

    // A completed generation passes through untouched, ready or superseded: the
    // superseded state is where the checkpoint's generation lives once a newer
    // commit has been indexed.
    for status in [GitGenerationStatus::Ready, GitGenerationStatus::Superseded] {
        let mut completed = to_generation();
        completed.status = status;
        let stored = found_generation(Ok(Some(completed))).expect("a completed generation");
        assert_eq!(stored.generation_key, to_key(), "{status:?} passes through");
        assert_eq!(stored.generation_number, 8);
    }
}

#[test]
fn a_pair_that_runs_backwards_is_a_conflict_and_an_identical_pair_is_not() {
    let reversed = *comparable(&to_generation(), &from_generation())
        .expect_err("the newer generation cannot be compared from");
    assert_eq!(
        reversed.status(),
        StatusCode::CONFLICT,
        "a backwards pair is a conflict, not an empty comparison"
    );
    assert!(
        comparable(&from_generation(), &to_generation()).is_ok(),
        "an older generation may be compared from"
    );
    assert!(
        comparable(&to_generation(), &to_generation()).is_ok(),
        "a generation holds the same bytes as itself, so the pair compares"
    );
    // The order follows the repository's own monotonic sequence, not either key:
    // a high generation number compared from a low one is refused even when the
    // keys look the right way round.
    let late = generation(99, to_key(), TO_COMMIT, 1, 0, 1);
    assert!(comparable(&late, &from_generation()).is_err());
    assert!(comparable(&from_generation(), &late).is_ok());
}

#[test]
fn every_change_kind_keeps_only_the_side_it_owns() {
    let response = page(&changed(), 50, None);
    let added = &response.changes[0];
    assert_eq!(added.path, "src/added.rs");
    assert!(added.before.is_none(), "an added path has no before side");
    assert_eq!(added.after.as_ref().map(|side| side.byte_count), Some(24));
    let modified = &response.changes[1];
    assert_eq!(
        modified.before.as_ref().map(|side| side.file_key),
        Some(file_key(2))
    );
    assert_ne!(
        modified.before.as_ref().map(|side| side.file_key),
        modified.after.as_ref().map(|side| side.file_key),
        "each side names its own manifest entry"
    );
    assert_eq!(
        (
            modified.before.as_ref().map(|side| side.line_count),
            modified.after.as_ref().map(|side| side.line_count)
        ),
        (Some(2), Some(4))
    );
    let deleted = &response.changes[2];
    assert!(deleted.after.is_none(), "a deleted path has no after side");
    assert_eq!(
        deleted.before.as_ref().map(|side| side.language.as_str()),
        Some("rust")
    );
    // The kind is the stored value, verbatim: the projection cannot relabel it.
    assert_eq!(
        response
            .changes
            .iter()
            .map(|change| change.change_kind)
            .collect::<Vec<_>>(),
        vec![
            GitFileChangeKind::Added,
            GitFileChangeKind::Modified,
            GitFileChangeKind::Deleted
        ]
    );
}

#[test]
fn the_page_names_both_generations_and_both_coverage_envelopes() {
    let response = page(&[], 50, None);
    assert_eq!(response.repository_key, repository_key());
    assert_eq!(response.from_generation_key, from_key());
    assert_eq!(response.from_generation_number, 7);
    assert_eq!(response.from_ref_name, "refs/heads/main");
    assert_eq!(response.from_commit_sha, FROM_COMMIT);
    assert_eq!(response.to_generation_key, to_key());
    assert_eq!(response.to_generation_number, 8);
    assert_eq!(response.to_commit_sha, TO_COMMIT);
    assert_eq!(response.index_status, GitIndexStatus::Stale);
    assert_eq!(
        response.checkpoint.indexed_commit_sha.as_deref(),
        Some(INDEXED_COMMIT)
    );
    assert_eq!(
        (response.from_file_count, response.to_file_count),
        (120, 122)
    );
    assert_eq!(
        (
            response.from_excluded_file_count,
            response.to_excluded_file_count
        ),
        (3, 3)
    );
    assert_eq!(
        (response.from_total_bytes, response.to_total_bytes),
        (4096, 4300)
    );
    assert_ne!(
        response.from_generation_key, response.to_generation_key,
        "the two sides of an empty comparison are still named"
    );
    // Two generations that hold the same bytes compare to an empty, terminal
    // page, never to an error: unchanged paths are omitted, not reported.
    assert!(response.changes.is_empty() && response.pagination.is_terminal());
}

#[test]
fn the_continuation_window_covers_every_change_exactly_once() {
    let all = changed();
    let first = page(&all, 2, None);
    assert_eq!(
        first.changes.len(),
        2,
        "the probe row is not a returned change"
    );
    assert!(first.pagination.has_more);
    assert_eq!(first.pagination.next_cursor.as_deref(), Some("2"));
    first
        .pagination
        .validate_continuation()
        .expect("has_more must carry its continuation token");

    let second = page(&all, 2, Some("2"));
    assert_eq!(
        second.changes.len(),
        1,
        "the window continues where it left off"
    );
    assert_eq!(second.changes[0].path, "src/dropped.rs");
    assert!(
        second.pagination.is_terminal(),
        "a complete page ends the walk"
    );
    assert!(second.pagination.next_cursor.is_none());
    let mut walked: Vec<&str> = first
        .changes
        .iter()
        .chain(second.changes.iter())
        .map(|change| change.path.as_str())
        .collect();
    walked.sort_unstable();
    let mut expected: Vec<&str> = all.iter().map(|diff| diff.path.as_str()).collect();
    expected.sort_unstable();
    assert_eq!(
        walked, expected,
        "the continuation covers every change once"
    );
}

#[test]
fn the_page_window_is_the_shared_bounded_cursor_contract() {
    for limit in [1, 50, 100] {
        let request = requested_page(&page_query(limit, None)).expect("a bounded limit");
        assert_eq!(request.limit(), i64::from(limit));
        assert_eq!(request.fetch_limit(), i64::from(limit) + 1);
    }
    for (limit, cursor) in [
        (0, None),
        (101, None),
        (u32::MAX, None),
        (50, Some("-1")),
        (50, Some("not-a-cursor")),
    ] {
        let error = refusal(limit, cursor);
        assert!(
            error.contains("limit must be between 1 and 100")
                || error.contains("git_manifest_cursor_invalid"),
            "limit {limit} cursor {cursor:?} must be a bounded refusal: {error}"
        );
    }
    // The token is a window position, not a request replay.
    let continued = requested_page(&page_query(2, Some("4"))).expect("a continuation");
    assert_eq!(continued.fetch_limit(), 3);
    assert_eq!(continued.next_cursor(true).as_deref(), Some("6"));
    assert!(requested_page(&page_query(2, Some("0"))).is_ok());
    // The default limit is the shared cursor default, not a second number.
    let default = GitRepositoryFileDiffQuery {
        from_generation: None,
        to_generation: None,
        limit: crate::contracts::CursorPageQuery::default().limit,
        cursor: None,
    };
    assert_eq!(default.limit, crate::contracts::pagination::default_limit());
    assert!(default.cursor.is_none());
}
