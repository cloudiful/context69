//! Database-gated round trips for the exact-file metadata read (issue #681
//! phase 5B).
//!
//! These reuse the manifest listing's disposable fixture instead of seeding a
//! second repository: the same group, repository, and activated generation serve
//! both reads, so the detail read runs against rows the manifest page already
//! produced. Only synthetic paths, counts, and statuses are reported.
//!
//! Every test skips unless `CONTEXT69_TEST_DATABASE_URL` names a migrated
//! scratch database and then removes the groups the fixture created. Nothing
//! here reads a secret or repository content.

use uuid::Uuid;

use crate::api::git_repository_files_db_fixture::{Fixture, INDEXED_COMMIT, MANIFEST_ENTRIES};
use crate::contracts::sources::GitIndexStatus;

use super::super::git_repository_file::file_detail;
use super::super::git_repository_files::serving_generation;

/// One path the fixture indexes, read verbatim.
const STORED_PATH: &str = "src/alpha.rs";

#[tokio::test]
async fn an_exact_file_read_returns_the_entry_with_provenance_and_coverage() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let generation_key = fixture.indexed().await;
    let source = fixture.source().await;
    let generation = serving_generation(&fixture.db, &source)
        .await
        .expect("resolve the serving generation")
        .expect("the repository serves an active generation");

    let stored = fixture
        .file(generation_key, STORED_PATH)
        .await
        .expect("the indexed manifest holds the path");
    let detail = file_detail(&source, &generation, stored);

    assert_eq!(detail.repository_key, fixture.repository_key);
    assert_eq!(detail.generation_key, generation_key);
    assert_eq!(detail.generation_number, 1);
    assert_eq!(detail.ref_name, "refs/heads/main");
    assert_eq!(detail.commit_sha, INDEXED_COMMIT);
    assert_eq!(detail.index_status, GitIndexStatus::Ready);
    assert_eq!(detail.file_count, MANIFEST_ENTRIES as i64);
    assert_eq!(detail.excluded_file_count, 2);
    assert_eq!(
        detail.checkpoint.indexed_commit_sha.as_deref(),
        Some(INDEXED_COMMIT)
    );
    assert_eq!(detail.file.path, STORED_PATH);

    // The same entry the manifest page serves, field for field.
    let page = fixture.page(100, None).await;
    let from_page = page
        .files
        .iter()
        .find(|file| file.path == STORED_PATH)
        .expect("the manifest page serves the same entry");
    assert_eq!(&detail.file, from_page);
    assert_eq!(detail.generation_key, page.generation_key);
    assert_eq!(detail.commit_sha, page.commit_sha);
    assert_eq!(detail.file_count, page.file_count);

    let serialized = serde_json::to_string(&detail).expect("the detail serializes");
    for forbidden in [
        "internal/secret",
        "provider_blob_sha",
        "\"text\"",
        "canonical_url",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the detail must not carry {forbidden}: {serialized}"
        );
    }

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn identity_is_the_exact_stored_path_and_nothing_else() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let generation_key = fixture.indexed().await;

    // A well-formed path the generation never stored, and paths that differ from
    // the stored one only by case, suffix, or nesting, all read as absent.
    for absent in [
        "src/not-indexed.rs",
        "SRC/alpha.rs",
        "src/Alpha.rs",
        "src/alpha.rs.bak",
        "alpha.rs",
        "src/./alpha.rs",
    ] {
        assert!(
            fixture.file(generation_key, absent).await.is_none(),
            "path {absent:?} is not the stored identity"
        );
    }
    assert!(
        fixture.file(generation_key, STORED_PATH).await.is_some(),
        "the stored path still resolves"
    );

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn a_building_or_absent_generation_serves_no_file() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    // Registered but never indexed: there is no generation to read a file from.
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

    // A building generation can hold a manifest, and the entry is readable by
    // key, but the read refuses it: only the generation the repository
    // advertises may answer, so a snapshot in flight is never served.
    let building = fixture.start_building().await;
    fixture.store_manifest(building).await;
    assert!(
        fixture.file(building, STORED_PATH).await.is_some(),
        "the building generation really does hold the entry"
    );
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
async fn a_foreign_or_unknown_group_reads_no_file() {
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
        "another group's repository is not resolvable, so no file is read from it"
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
            .file_in(
                foreign_group_id,
                fixture.repository_key,
                generation_key,
                STORED_PATH
            )
            .await
            .is_none(),
        "a foreign group reads no manifest entry, by path or otherwise"
    );
    assert!(
        fixture
            .file_in(
                foreign_group_id,
                foreign_repository_key,
                generation_key,
                STORED_PATH
            )
            .await
            .is_none(),
        "an unknown generation of a real group reads no entry"
    );

    // The owning group still reads its own entry, so the confinement is the
    // group's and not a missing manifest.
    assert!(
        fixture.file(generation_key, STORED_PATH).await.is_some(),
        "the owning group reads its own entry"
    );

    fixture.cleanup(&[fixture.group_id, foreign_group_id]).await;
}
