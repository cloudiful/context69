//! What happens when a second startup, or another writer, is working on the
//! same rows at the same time.
//!
//! See [`super`] for the shared harness and the safety rules these cases run under.

use std::sync::{Arc, Mutex, mpsc};

use super::*;

// --- concurrent startups and writers -------------------------------------------

#[test]
fn two_independent_runs_convert_every_row_exactly_once() {
    let sealed: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(vec![0, 0]));
    let left = Arc::clone(&sealed);
    let right = Arc::clone(&sealed);
    run_pair_after(
        async |pool| {
            cleanup(pool).await;
            for index in 0..RACE_ROWS {
                let key = key_name(RACE_PURPOSE, &format!("race-{index:04}"));
                insert_legacy(pool, &key, &plaintext(&key)).await;
            }
        },
        async move |pool| {
            let report = store(pool).seal_legacy_plaintext().await.expect("seal");
            left.lock().expect("record")[0] = report.sealed;
        },
        async move |pool| {
            let report = store(pool).seal_legacy_plaintext().await.expect("seal");
            right.lock().expect("record")[1] = report.sealed;
        },
        async |pool| {
            let counts = sealed.lock().expect("read the recorded counts").clone();
            // Between them the two startups account for every row: no row
            // missed, and none converted twice. What is *not* asserted is that
            // each side found work — a startup that reaches the table only
            // after the other finished is correctly owed zero. The property a
            // startup must never fake is a false-empty worklist while rows are
            // still plaintext, and that is what the final state below rules out.
            assert_eq!(
                counts.iter().sum::<usize>(),
                RACE_ROWS,
                "the two startups reported {counts:?}"
            );
            // Had either startup skipped past locked rows instead of waiting for
            // them, it would have reported success over rows it never converted,
            // and these are the assertions that would notice.
            assert_eq!(legacy_count(pool).await, 0);
            assert_recovered(pool, RACE_PURPOSE, "race", &[0, RACE_ROWS - 1]).await;
            cleanup(pool).await;
        },
    );
}

#[test]
fn a_writer_that_commits_before_the_sweep_claims_a_row_is_not_overwritten() {
    run(async |pool| {
        cleanup(pool).await;
        let purpose = SecretPurpose::SourceConnectionDatabaseUrl;
        let written = key_name(purpose, "writer-first");
        let untouched = key_name(purpose, "writer-first-other");
        insert_legacy(pool, &written, &plaintext(&written)).await;
        insert_legacy(pool, &untouched, &plaintext(&untouched)).await;

        // The ordinary write path, on the deployment's own key: the row is now a
        // sealed row this sweep must not enumerate at all.
        store(pool)
            .write(purpose, &written, b"synthetic-newer-value")
            .await
            .expect("write the newer value");
        let after_write = stored_row(pool, &written).await;

        let report = store(pool).seal_legacy_plaintext().await.expect("seal");

        assert_eq!(report.sealed, 1, "only the untouched row is swept");
        assert_eq!(
            stored_row(pool, &written).await,
            after_write,
            "a row sealed by a writer keeps its bytes and its timestamps"
        );
        assert_eq!(
            store(pool)
                .get(purpose, &written)
                .await
                .expect("open the writer's row")
                .expect("the row is stored")
                .expose(),
            b"synthetic-newer-value"
        );
        cleanup(pool).await;
    });
}

#[test]
fn a_writer_that_commits_while_the_sweep_waits_for_its_row_wins() {
    // `aaa` sorts before `zzz`, so the claim query locks the first row and then
    // blocks on the second — the row this writer is holding.
    let purpose = SecretPurpose::GitWebhookSigningSecret;
    let claimed = key_name(purpose, "aaa-blocked-first");
    let blocked = key_name(purpose, "zzz-blocked-second");
    let (start, awaiting) = mpsc::channel();
    let (seed_claimed, seed_blocked) = (claimed.clone(), blocked.clone());
    let (write_blocked, assert_claimed, assert_blocked) =
        (blocked.clone(), claimed.clone(), blocked.clone());

    run_pair_after(
        async move |pool| {
            cleanup(pool).await;
            insert_legacy(pool, &seed_claimed, &plaintext(&seed_claimed)).await;
            insert_legacy(pool, &seed_blocked, &plaintext(&seed_blocked)).await;
        },
        // The writer: holds the row the sweep is about to reach, then replaces
        // the value underneath it. It writes on its own locked connection,
        // because going through the store would wait for the very lock it holds.
        async move |pool| {
            let mut lock = pool.begin().await.expect("begin the writer's transaction");
            sqlx::query("SELECT 1 FROM context69.internal_secrets WHERE key = $1 FOR UPDATE")
                .bind(&write_blocked)
                .fetch_one(&mut *lock)
                .await
                .expect("lock the row the sweep will reach");
            start.send(()).expect("the sweep is waiting to start");

            // Give the sweep time to reach the locked row. The case does not
            // depend on this landing before it does: a row that was already
            // sealed before the claim simply is not offered, which is the same
            // outcome the assertion below requires.
            std::thread::sleep(std::time::Duration::from_millis(300));
            let sealed = seal(
                MASTER_KEY_B64,
                KEY_VERSION,
                purpose,
                &write_blocked,
                b"synthetic-newer-value",
            );
            sqlx::query(
                "UPDATE context69.internal_secrets \
                 SET value = $2, purpose = $3, key_version = $4, ciphertext_version = $5 \
                 WHERE key = $1",
            )
            .bind(&write_blocked)
            .bind(sealed.value)
            .bind(sealed.purpose)
            .bind(sealed.key_version)
            .bind(sealed.ciphertext_version)
            .execute(&mut *lock)
            .await
            .expect("write the newer value while holding the lock");
            lock.commit().await.expect("release the row");
        },
        // The sweep, which has to wait for the row above and then re-read it.
        async move |pool| {
            awaiting.recv().expect("the writer holds the row");
            let report = store(pool)
                .seal_legacy_plaintext()
                .await
                .expect("the sweep finishes");
            assert_eq!(
                report.sealed, 1,
                "only the unblocked row was the sweep's work"
            );
        },
        async |pool| {
            assert_eq!(legacy_count(pool).await, 0);
            assert_eq!(
                store(pool)
                    .get(purpose, &assert_claimed)
                    .await
                    .expect("open the swept row")
                    .expect("the row is stored")
                    .expose(),
                plaintext(&assert_claimed),
                "the row the sweep did reach is its own value"
            );
            assert_eq!(
                store(pool)
                    .get(purpose, &assert_blocked)
                    .await
                    .expect("open the writer's row")
                    .expect("the row is stored")
                    .expose(),
                b"synthetic-newer-value",
                "the sweep did not clobber the writer's newer value"
            );
            assert_eq!(
                stored_row(pool, &assert_blocked).await.key_version,
                KEY_VERSION_AS_INT
            );
            cleanup(pool).await;
        },
    );
}

#[test]
fn the_swept_value_is_sealed_under_this_deployments_key_and_version() {
    run(async |pool| {
        cleanup(pool).await;
        let key = key_name(SecretPurpose::RuntimeS3SecretKey, "versioned");
        insert_legacy(pool, &key, &plaintext(&key)).await;

        store(pool).seal_legacy_plaintext().await.expect("seal");

        // The other key at another version cannot open it: the conversion used
        // the configured key, not just some key that happens to be configured.
        assert!(
            other_store(pool)
                .get(SecretPurpose::RuntimeS3SecretKey, &key)
                .await
                .is_err()
        );
        assert_eq!(stored_row(pool, &key).await.key_version, KEY_VERSION_AS_INT);
        cleanup(pool).await;
    });
}
