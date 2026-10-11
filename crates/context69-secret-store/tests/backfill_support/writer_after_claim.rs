use std::time::{Duration, Instant};

use sqlx::postgres::PgPoolOptions;

use super::*;

async fn wait_for_blocked(pool: &PgPool, application: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
             WHERE application_name = $1 AND cardinality(pg_blocking_pids(pid)) > 0)",
        )
        .bind(application)
        .fetch_one(pool)
        .await
        .expect("observe lock wait");
        if blocked {
            return;
        }
        assert!(Instant::now() < deadline, "{application} did not block");
        std::thread::sleep(Duration::from_millis(10));
    }
}

async fn worker_pool(application: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url().expect("owned database"))
        .await
        .expect("worker connection");
    sqlx::query("SELECT set_config('application_name', $1, false)")
        .bind(application)
        .execute(&pool)
        .await
        .expect("name worker");
    pool
}

#[test]
fn a_writer_after_claim_waits_and_keeps_its_newer_value() {
    run(async |pool| {
        cleanup(pool).await;
        let purpose = SecretPurpose::GitProviderToken;
        let first = key_name(purpose, "aaa-reviewer-claimed");
        let last = key_name(purpose, "zzz-reviewer-blocker");
        insert_legacy(pool, &first, b"synthetic-old-value").await;
        insert_legacy(pool, &last, b"synthetic-blocker-value").await;
        let mut blocker = pool.begin().await.expect("blocker transaction");
        sqlx::query("SELECT 1 FROM context69.internal_secrets WHERE key = $1 FOR UPDATE")
            .bind(&last)
            .fetch_one(&mut *blocker)
            .await
            .expect("hold the last ordered row");
        let sweep = std::thread::spawn(|| {
            sqlx::test_block_on(async {
                let pool = worker_pool("c69-reviewer-sweep").await;
                store(&pool).seal_legacy_plaintext().await.expect("sweep")
            })
        });
        // The ordered claim holds the first row while blocked on the last.
        wait_for_blocked(pool, "c69-reviewer-sweep").await;
        let written = first.clone();
        let writer = std::thread::spawn(move || {
            sqlx::test_block_on(async {
                let pool = worker_pool("c69-reviewer-writer").await;
                store(&pool)
                    .write(purpose, &written, b"synthetic-newer-value")
                    .await
                    .expect("writer after claim")
            });
        });
        wait_for_blocked(pool, "c69-reviewer-writer").await;
        blocker.commit().await.expect("allow sweep to finish");
        assert_eq!(sweep.join().expect("sweep thread").sealed, 2);
        writer.join().expect("writer thread");
        assert_eq!(
            store(pool)
                .get(purpose, &first)
                .await
                .expect("open newer value")
                .expect("stored row")
                .expose(),
            b"synthetic-newer-value"
        );
        assert_eq!(legacy_count(pool).await, 0);
        cleanup(pool).await;
    });
}
