//! Database-gated round trips for the active-generation manifest listing
//! (issue #681 phase 5A).
//!
//! What only a real schema can answer is whether the group-scoped reads the
//! route composes return one coherent generation: that a repository serves the
//! generation it advertises, that a page of its manifest is ordered by path and
//! neither skips nor repeats, that a repository with nothing activated reads as
//! not ready rather than empty, and that a foreign group sees none of it.
//!
//! Every test skips unless `CONTEXT69_TEST_DATABASE_URL` names a migrated
//! scratch database and then removes the groups it created, so a run leaves
//! nothing behind. Nothing here reads a secret. The fixture that builds the
//! scratch rows lives beside this suite in `git_repository_files_db_fixture.rs`.

use uuid::Uuid;

use crate::contracts::sources::{GitIndexProfile, GitIndexStatus};
use crate::db::NewGitRepositoryGeneration;

use super::serving_generation;
use fixture::{Fixture, INDEXED_COMMIT, MANIFEST_ENTRIES, TARGET_COMMIT, paths};

#[path = "git_repository_files_db_fixture.rs"]
mod fixture;

#[tokio::test]
async fn a_page_reports_the_serving_generation_provenance_and_coverage() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let generation_key = fixture.indexed().await;
    let page = fixture.page(50, None).await;

    assert_eq!(page.repository_key, fixture.repository_key);
    assert_eq!(page.generation_key, generation_key);
    assert_eq!(
        page.generation_number, 1,
        "the first activation is number 1"
    );
    assert_eq!(page.ref_name, "refs/heads/main");
    assert_eq!(page.commit_sha, INDEXED_COMMIT);
    assert_eq!(page.index_status, GitIndexStatus::Ready);
    assert_eq!(
        page.file_count, MANIFEST_ENTRIES as i64,
        "coverage counts the stored manifest"
    );
    assert_eq!(page.excluded_file_count, 2);
    assert_eq!(page.total_bytes, 4096);
    assert_eq!(
        page.checkpoint.indexed_commit_sha.as_deref(),
        Some(INDEXED_COMMIT),
        "the page states which commit is indexed, not only which one it serves"
    );
    assert_eq!(
        page.checkpoint.target_commit_sha.as_deref(),
        Some(TARGET_COMMIT),
        "a ref that has moved past the indexed commit stays visible"
    );
    assert!(page.checkpoint.indexed_at.is_some());
    assert_eq!(page.files.len(), MANIFEST_ENTRIES);
    assert!(
        page.files
            .iter()
            .all(|file| file.generation_key == generation_key
                && file.repository_key == fixture.repository_key
                && !file.path.is_empty()),
        "every entry belongs to the serving generation and names a real path"
    );

    // The page is the safe projection: identity and counters only, with no
    // content, provider blob id, or secret reference anywhere in the payload.
    let serialized = serde_json::to_string(&page).expect("the page serializes");
    for forbidden in [
        "internal/secret",
        "provider_blob_sha",
        "\"text\"",
        "canonical_url",
        "connection_key",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the page must not carry {forbidden}: {serialized}"
        );
    }

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn manifest_pages_are_path_ordered_and_cover_the_whole_manifest_once() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    fixture.indexed().await;

    // One unpaginated read is the order the storage layer defines.
    let whole = fixture.page(100, None).await;
    assert_eq!(whole.files.len(), MANIFEST_ENTRIES);
    assert!(whole.pagination.is_terminal());

    let first = fixture.page(2, None).await;
    assert_eq!(first.pagination.next_cursor.as_deref(), Some("2"));
    assert!(first.pagination.has_more);

    let second = fixture
        .page(2, first.pagination.next_cursor.as_deref())
        .await;
    assert_eq!(second.pagination.next_cursor.as_deref(), Some("4"));
    assert!(second.pagination.has_more);

    let terminal = fixture
        .page(2, second.pagination.next_cursor.as_deref())
        .await;
    assert_eq!(terminal.files.len(), 1);
    assert!(!terminal.pagination.has_more);
    assert_eq!(
        terminal.pagination.next_cursor, None,
        "the last page must not invite a continuation"
    );

    // Paging neither reorders, skips, nor repeats an entry.
    let paged: Vec<String> = [&first, &second, &terminal]
        .iter()
        .flat_map(|page| paths(page))
        .collect();
    assert_eq!(paged, paths(&whole), "pages follow the stored path order");
    let mut unique = paged.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), MANIFEST_ENTRIES, "no path is served twice");

    // Every page describes the same generation, so a caller can quote any of
    // them against the same pinned commit, and coverage always describes the
    // whole manifest rather than the page.
    for page in [&first, &second, &terminal] {
        assert_eq!(page.generation_key, whole.generation_key);
        assert_eq!(page.commit_sha, INDEXED_COMMIT);
        assert_eq!(page.file_count, MANIFEST_ENTRIES as i64);
        page.pagination
            .validate_continuation()
            .expect("has_more must carry its continuation");
    }

    // A cursor at the end of the manifest is an empty terminal page, not an
    // error, and one past it cannot skip an entry.
    let past_end = fixture.page(2, Some("5")).await;
    assert!(past_end.files.is_empty());
    assert!(!past_end.pagination.has_more);

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn a_repository_with_no_active_generation_reads_as_not_ready() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    // Registered but never indexed: there is no generation to page at all.
    let source = fixture.source().await;
    assert_eq!(source.index_status, GitIndexStatus::Pending);
    assert!(source.active_generation_key.is_none());
    assert!(
        serving_generation(&fixture.db, &source)
            .await
            .expect("resolve the serving generation")
            .is_none(),
        "a repository with no activated generation is not ready, not empty"
    );
    assert!(
        fixture
            .db
            .get_git_active_generation(fixture.group_id, fixture.repository_key)
            .await
            .expect("read the active generation pointer")
            .is_none()
    );

    // A generation that exists but was never activated is equally not ready:
    // the route serves only what the repository advertises.
    let building = fixture
        .db
        .start_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: INDEXED_COMMIT.to_string(),
                index_profile: GitIndexProfile::Lexical,
            },
        )
        .await
        .expect("start a generation")
        .generation_key;
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
async fn a_foreign_group_sees_no_repository_no_generation_and_no_manifest() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let generation_key = fixture.indexed().await;
    let foreign_group_id = fixture.foreign_group().await;
    // The foreign group is a real group with its own repository, so an empty
    // result cannot be explained by the group not existing.
    let foreign_repository_key = fixture.repository_for(foreign_group_id).await;

    assert!(
        fixture
            .db
            .get_git_repository_source(foreign_group_id, fixture.repository_key)
            .await
            .expect("read the source as a foreign group")
            .is_none(),
        "another group's repository must not be readable"
    );
    assert!(
        fixture
            .db
            .get_git_repository_source(foreign_group_id, Uuid::new_v4())
            .await
            .expect("read an unknown repository")
            .is_none(),
        "an unknown repository reads the same as a foreign one"
    );
    assert!(
        fixture
            .db
            .get_git_active_generation(foreign_group_id, fixture.repository_key)
            .await
            .expect("read the active generation pointer as a foreign group")
            .is_none(),
        "a foreign group cannot learn which generation is active"
    );
    assert!(
        fixture
            .db
            .get_git_repository_generation(foreign_group_id, fixture.repository_key, generation_key)
            .await
            .expect("read the generation as a foreign group")
            .is_none(),
        "a foreign group cannot read generation metadata"
    );
    let leaked = fixture
        .db
        .list_git_generation_files(
            foreign_group_id,
            fixture.repository_key,
            generation_key,
            10,
            0,
        )
        .await
        .expect("list the manifest as a foreign group");
    assert!(
        leaked.is_empty(),
        "a foreign group reads no manifest entry: {} entry(ies)",
        leaked.len()
    );

    // The owning group still reads its own repository and manifest, so the
    // confinement is the group's and not an artifact of the fixture.
    assert!(
        fixture
            .db
            .get_git_repository_source(fixture.group_id, foreign_repository_key)
            .await
            .expect("read another group's repository from the owning group")
            .is_none(),
        "each group's repository stays its own"
    );
    assert_eq!(fixture.page(50, None).await.files.len(), MANIFEST_ENTRIES);

    fixture.cleanup(&[fixture.group_id, foreign_group_id]).await;
}
