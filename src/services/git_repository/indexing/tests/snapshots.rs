//! Basic snapshot publication, no-op reuse, pin stability, and skip summaries.

use uuid::Uuid;

use crate::contracts::sources::{GitGenerationStatus, GitIndexStatus};
use crate::db::GitRepositorySettingsUpdate;

use super::super::super::{limits::GitIndexLimits, support::*};
use super::super::index_git_repository_snapshot;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_bounded_snapshot_persists_exact_content_and_activates() {
    let fixture = Fixture::setup().await;
    let commit = sha(0xa1);
    let provider = FakeProvider::new(binary_spec(&commit));
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    let created = provider.created.load(std::sync::atomic::Ordering::SeqCst);
    let seen_url = provider.seen_url.lock().unwrap().clone();
    fixture.cleanup().await;

    let outcome = result.expect("the snapshot must index");
    assert_eq!(outcome.commit_sha, commit);
    assert_eq!(outcome.generation_number, 1);
    assert_eq!(outcome.file_count, 2);
    assert_eq!(
        outcome.excluded_file_count, 3,
        "two listing exclusions plus the binary file"
    );
    assert_eq!(reads.active, Some(outcome.generation_key));
    assert_eq!(
        reads
            .generations
            .first()
            .map(|generation| generation.status),
        Some(GitGenerationStatus::Ready)
    );
    assert_eq!(
        reads
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["README.md", "src/main.rs"]
    );
    let main = reads
        .files
        .iter()
        .find(|file| file.path == "src/main.rs")
        .expect("src/main.rs must be in the manifest");
    assert_eq!(main.language, "rust");
    assert_eq!(main.byte_count, 13);
    assert_eq!(reads.chunks.len(), 1);
    assert_eq!(reads.chunks[0].text, "fn main() {}\n");
    assert_eq!(
        (reads.chunks[0].start_line, reads.chunks[0].end_line),
        (1, 1)
    );
    let source = reads.source.expect("source still exists");
    assert_eq!(source.index_status, GitIndexStatus::Ready);
    assert_eq!(
        source.checkpoint.target_commit_sha.as_deref(),
        Some(commit.as_str())
    );
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(commit.as_str())
    );
    assert_eq!(source.active_generation_key, Some(outcome.generation_key));
    assert_eq!(created, 1, "one fresh acquirer per generation");
    assert_eq!(seen_url.as_deref(), Some(source.canonical_url.as_str()));
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_empty_eligible_snapshot_completes_safely() {
    let fixture = Fixture::setup().await;
    let commit = sha(0xa2);
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: commit.clone(),
        files: Vec::new(),
        excluded: 5,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    fixture.cleanup().await;

    let outcome = result.expect("an empty snapshot still completes");
    assert_eq!(outcome.file_count, 0);
    assert_eq!(outcome.excluded_file_count, 5);
    assert!(reads.files.is_empty());
    assert_eq!(reads.active, Some(outcome.generation_key));
    let source = reads.source.expect("source still exists");
    assert_eq!(source.index_status, GitIndexStatus::Ready);
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(commit.as_str())
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_failed_fetch_leaves_the_previous_active_generation_serving() {
    let fixture = Fixture::setup().await;
    let prior = fixture.start_prior_generation().await;
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xa4),
        files: vec![("src/lib.rs", sha(0xc2), b"pub fn x() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: Some(sha(0xc2)),
        mutate: None,
    });
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    fixture.cleanup().await;

    let error = result.expect_err("a fetch failure must fail the attempt");
    assert!(error.to_string().contains("git_http_status_500"), "{error}");
    assert_eq!(
        reads.active,
        Some(prior),
        "the previous active generation keeps serving"
    );
    let status = |key: Uuid| {
        reads
            .generations
            .iter()
            .find(|generation| generation.generation_key == key)
            .map(|generation| generation.status)
    };
    assert_eq!(status(prior), Some(GitGenerationStatus::Ready));
    let failed = reads
        .generations
        .iter()
        .find(|generation| generation.generation_key != prior)
        .expect("a new generation row must exist")
        .generation_key;
    assert_eq!(status(failed), Some(GitGenerationStatus::Failed));
    let source = reads.source.expect("source still exists");
    assert_eq!(source.index_status, GitIndexStatus::Failed);
    assert_eq!(source.active_generation_key, Some(prior));
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_same_target_reuses_the_active_generation_without_writing_one() {
    let fixture = Fixture::setup().await;
    let commit = sha(0xb1);
    let provider = FakeProvider::new(binary_spec(&commit));
    let first = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("the first snapshot indexes");
    let second = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("an unchanged target is a no-op");
    let reads = fixture.reads(None).await;
    fixture.cleanup().await;

    assert!(second.no_op);
    assert_eq!(second.generation_key, first.generation_key);
    assert_eq!(
        reads.generations.len(),
        1,
        "no second generation is written"
    );
    assert_eq!(reads.active, Some(first.generation_key));
    let source = reads.source.expect("source still exists");
    assert_eq!(source.index_status, GitIndexStatus::Ready);
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(commit.as_str())
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_explicit_pin_never_drifts_when_the_branch_advances() {
    let fixture = Fixture::setup().await;
    let pinned = sha(0xa8);
    let source = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await
        .expect("read source")
        .expect("the fixture owns the source");
    fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "set",
                pinned_commit_value: Some(&pinned),
                index_profile: None,
                include_patterns: &source.include_patterns,
                exclude_patterns: &source.exclude_patterns,
                interval_seconds: source.interval_seconds,
                webhook_enabled: source.webhook_enabled,
                enabled: true,
                retention_count: source.retention_count,
            },
        )
        .await
        .expect("pin the source")
        .expect("the fixture owns the source");

    let advancing = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xa9),
        files: vec![("src/one.rs", sha(0xd3), b"pub fn one() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let first = index_git_repository_snapshot(
        fixture.db(),
        &advancing,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("a pinned source indexes the configured commit");
    let second = index_git_repository_snapshot(
        fixture.db(),
        &advancing,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("a pinned source stays on the configured commit");

    let reads = fixture.reads(Some(first.generation_key)).await;
    fixture.cleanup().await;

    assert_eq!(first.commit_sha, pinned);
    assert!(second.no_op);
    assert_eq!(second.commit_sha, pinned);
    let source = reads.source.expect("source still exists");
    assert_eq!(source.version.commit_sha.as_deref(), Some(pinned.as_str()));
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(pinned.as_str())
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_source_before_its_first_generation_lists_a_null_active_pointer() {
    let fixture = Fixture::setup().await;
    let repository_key = fixture.repository_key;
    let listed = fixture
        .db()
        .list_git_repository_sources(fixture.group_id)
        .await;
    let fetched = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await;
    fixture.cleanup().await;

    let listed = listed.expect("list the group's sources");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].repository_key, repository_key);
    assert!(
        listed[0].active_generation_key.is_none(),
        "a source with no active generation must read a null pointer, not depend on planner inference"
    );
    let fetched = fetched
        .expect("read the source")
        .expect("the fixture owns the source");
    assert!(fetched.active_generation_key.is_none());
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_lexical_generation_enumerates_without_a_destination_and_deletes_with_its_source() {
    let fixture = Fixture::setup().await;
    let generation_key = fixture.start_prior_generation().await;
    let destinations = fixture
        .db()
        .list_git_generations_with_code_vectors(fixture.group_id, fixture.repository_key)
        .await;
    let deleted = fixture
        .db()
        .delete_git_repository_source(fixture.group_id, fixture.repository_key)
        .await;
    let fetched = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await;
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await;
    fixture.cleanup().await;

    let destinations = destinations.expect("enumerate the source's destinations");
    assert_eq!(destinations.len(), 1);
    assert_eq!(destinations[0].generation_key, generation_key);
    assert!(
        destinations[0].code_vectors.is_none(),
        "a lexical generation has no destination to clean"
    );
    assert!(
        deleted.expect("delete the source"),
        "a lexical-only source still owns its row"
    );
    assert!(
        fetched.expect("read the deleted source").is_none(),
        "the source and its cascade are gone"
    );
    assert!(
        generations
            .expect("read the deleted generations")
            .is_empty(),
        "no generation outlives its source"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn acquisition_too_large_files_reach_the_source_skip_summary() {
    let fixture = Fixture::setup().await;
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xb8),
        files: vec![("src/main.rs", sha(0xb9), b"fn main() {}\n")],
        excluded: 1,
        too_large: 1,
        fail_blob: None,
        mutate: None,
    });
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await;
    let reads = fixture
        .reads(result.as_ref().ok().map(|outcome| outcome.generation_key))
        .await;
    fixture.cleanup().await;

    let outcome = result.expect("the bounded snapshot still activates");
    assert_eq!(
        outcome.excluded_file_count, 1,
        "the provider-excluded file stays counted as excluded"
    );
    assert_eq!(
        outcome.skip_summary.too_large, 1,
        "the provider's oversized reason is not discarded"
    );
    let source = reads.source.expect("source still exists");
    assert_eq!(
        source.last_skip_summary.too_large, 1,
        "the persisted source summary carries the oversized reason"
    );
}
