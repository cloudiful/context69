//! Concurrent per-repository generation numbering (issue #681 work unit 3B1).
//!
//! Two task retries can start a generation for the same repository at the same
//! time. The generation number must come from the repository row's own
//! counter, so the two starts take distinct consecutive numbers: a number
//! derived from a read of the generations table inside the same statement could
//! only see the snapshot taken before the other start committed, and both would
//! try to insert the same number.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::contracts::sources::{GitIndexProfile, GitProviderKind, GitRefreshPolicy};
use context69::db::{Database, NewGitRepositoryGeneration, NewGitRepositorySource};
use sqlx::Row;
use uuid::Uuid;

/// Starts issued together in the concurrency test. The database pool allows
/// ten connections, so this many statements genuinely contend for the
/// repository row instead of being serialised by the pool.
const CONCURRENT_STARTS: usize = 8;

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_group(db: &Database) -> i64 {
    let key = format!("gen-concurrency-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'personal', $3) RETURNING id",
    )
    .bind(&key)
    .bind("Generation Concurrency Test Group")
    .bind(format!("/{}", key))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
}

async fn seed_repository(db: &Database, group_id: i64) -> Uuid {
    let nonce = Uuid::new_v4();
    db.upsert_git_repository_source(
        group_id,
        &NewGitRepositorySource {
            connection_key: None,
            provider: GitProviderKind::Generic,
            canonical_url: format!("https://example.invalid/{nonce}.git"),
            owner: "concurrency".to_string(),
            name: nonce.to_string(),
            default_branch: "main".to_string(),
            target_ref: "refs/heads/main".to_string(),
            target_commit_sha: Some("a".repeat(40)),
            index_profile: GitIndexProfile::Hybrid,
            refresh_policy: GitRefreshPolicy::Manual,
        },
    )
    .await
    .expect("seed repository source")
    .repository_key
}

fn new_generation() -> NewGitRepositoryGeneration {
    NewGitRepositoryGeneration {
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "b".repeat(40),
        index_profile: GitIndexProfile::Hybrid,
    }
}

async fn cleanup_repository(db: &Database, group_id: i64, repository_key: Uuid) {
    sqlx::query("DELETE FROM context69.git_repository_generations WHERE repository_key = $1")
        .bind(repository_key)
        .execute(db.pool())
        .await
        .expect("clean up git repository generations");
    sqlx::query("DELETE FROM context69.git_repository_sources WHERE repository_key = $1")
        .bind(repository_key)
        .execute(db.pool())
        .await
        .expect("clean up git repository source");
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up test group");
}

async fn generation_counter(db: &Database, repository_key: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT generation_counter FROM context69.git_repository_sources \
         WHERE repository_key = $1",
    )
    .bind(repository_key)
    .fetch_one(db.pool())
    .await
    .expect("read repository generation counter")
}

#[tokio::test]
async fn concurrent_starts_take_distinct_consecutive_numbers() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping generation concurrency test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let repository_key = seed_repository(&db, group_id).await;

    // More starts than the pool can serialise, all issued before any is
    // awaited, so their statements overlap on the repository row. Every one
    // must succeed: an allocation that read a snapshot taken before a
    // concurrent commit would hand two starts the same number, and the second
    // insert would fail on uq_git_repository_generations_number.
    let mut starts = Vec::new();
    for _ in 0..CONCURRENT_STARTS {
        let generation = new_generation();
        let start_db = db.clone();
        starts.push(async move {
            start_db
                .start_git_repository_generation(group_id, repository_key, &generation)
                .await
        });
    }
    let results = futures::future::join_all(starts).await;

    let mut numbers = Vec::with_capacity(results.len());
    let mut keys = Vec::with_capacity(results.len());
    for (index, result) in results.into_iter().enumerate() {
        let generation = result.unwrap_or_else(|error| {
            panic!("concurrent start {index} must succeed, got: {error:#}")
        });
        numbers.push(generation.generation_number);
        keys.push(generation.generation_key);
    }
    numbers.sort_unstable();
    let expected = (1..=CONCURRENT_STARTS as i64).collect::<Vec<_>>();
    assert_eq!(
        numbers, expected,
        "concurrent starts must take distinct consecutive numbers"
    );
    let mut distinct_keys = keys.clone();
    distinct_keys.sort_unstable();
    distinct_keys.dedup();
    assert_eq!(
        distinct_keys.len(),
        CONCURRENT_STARTS,
        "each start creates its own generation row"
    );
    assert_eq!(
        generation_counter(&db, repository_key).await,
        CONCURRENT_STARTS as i64,
        "the counter advances once per start"
    );

    // A later start continues the same sequence, so the counter is the single
    // allocation path and no earlier number is reused.
    let next = db
        .start_git_repository_generation(group_id, repository_key, &new_generation())
        .await
        .expect("start after the concurrent batch must succeed");
    assert_eq!(next.generation_number, CONCURRENT_STARTS as i64 + 1);

    cleanup_repository(&db, group_id, repository_key).await;
}

#[tokio::test]
async fn a_failed_start_consumes_no_number_and_stays_in_its_group() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping generation concurrency test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let other_group_id = seed_group(&db).await;
    let repository_key = seed_repository(&db, group_id).await;

    // Another group cannot reach this repository, so it neither starts a
    // generation nor advances this repository's counter.
    assert!(
        db.start_git_repository_generation(other_group_id, repository_key, &new_generation())
            .await
            .is_err(),
        "a repository of another group must not start a generation"
    );
    assert_eq!(
        generation_counter(&db, repository_key).await,
        0,
        "a rejected start consumes no number"
    );

    let first = db
        .start_git_repository_generation(group_id, repository_key, &new_generation())
        .await
        .expect("owning group start must succeed");
    assert_eq!(
        first.generation_number, 1,
        "numbering continues from the counter, not from a rejected attempt"
    );

    // An unknown repository is likewise an error, not a silent allocation.
    assert!(
        db.start_git_repository_generation(group_id, Uuid::new_v4(), &new_generation())
            .await
            .is_err(),
        "an unknown repository must not start a generation"
    );
    assert_eq!(
        generation_counter(&db, repository_key).await,
        1,
        "a rejected start leaves the counter untouched"
    );

    cleanup_repository(&db, group_id, repository_key).await;
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(other_group_id)
        .execute(db.pool())
        .await
        .expect("clean up second test group");
}

#[tokio::test]
async fn a_failed_generation_rolls_back_no_counter_advance() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping generation concurrency test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_id = seed_group(&db).await;
    let repository_key = seed_repository(&db, group_id).await;

    // A ref the generations table rejects fails the whole statement, so the
    // counter increment rolls back with the insert.
    let rejected = NewGitRepositoryGeneration {
        ref_name: "   ".to_string(),
        commit_sha: "c".repeat(40),
        index_profile: GitIndexProfile::Hybrid,
    };
    assert!(
        db.start_git_repository_generation(group_id, repository_key, &rejected)
            .await
            .is_err(),
        "a blank ref must be rejected by the generations table"
    );
    assert_eq!(
        generation_counter(&db, repository_key).await,
        0,
        "a failed insert must roll the counter increment back"
    );

    let first = db
        .start_git_repository_generation(group_id, repository_key, &new_generation())
        .await
        .expect("start after a failed one must succeed");
    assert_eq!(
        first.generation_number, 1,
        "a rolled-back allocation leaves no gap and no consumed number"
    );

    cleanup_repository(&db, group_id, repository_key).await;
}
