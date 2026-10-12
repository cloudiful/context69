//! What a keyed startup does to the rows it finds: every catalogue purpose,
//! byte-for-byte recovery, the row identity and timestamps, batching,
//! idempotence, and a deployment with no key.
//!
//! See [`super`] for the shared harness and the safety rules these cases run under.

use super::*;

// --- the whole catalogue -------------------------------------------------------

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_configured_key_seals_every_supported_legacy_row() {
    run(async |pool| {
        cleanup(pool).await;
        for purpose in SecretPurpose::ALL {
            let key = key_name(purpose, "all");
            insert_legacy(pool, &key, &plaintext(&key)).await;
        }

        let report = store(pool).seal_legacy_plaintext().await.expect("seal");

        assert_eq!(report.sealed, SecretPurpose::ALL.len());
        assert_eq!(legacy_count(pool).await, 0, "no plaintext row remains");
        for purpose in SecretPurpose::ALL {
            let key = key_name(purpose, "all");
            let row = stored_row(pool, &key).await;
            assert_eq!(row.key, key, "the row key is its own identity");
            assert_eq!(row.purpose.as_deref(), Some(purpose.as_str()), "{key}");
            assert_eq!(row.key_version, KEY_VERSION_AS_INT, "{key}");
            assert_eq!(row.ciphertext_version, SEALED_CIPHERTEXT_VERSION, "{key}");
            // The value is gone from the table ...
            assert!(
                !row.value
                    .windows(plaintext(&key).len())
                    .any(|window| window == plaintext(&key)),
                "{key}: the stored bytes must not contain the plaintext"
            );
            // ... and what the deployment reads back is exactly what it held.
            let opened = store(pool)
                .get(purpose, &key)
                .await
                .expect("open the sealed row")
                .expect("the row is stored");
            assert_eq!(opened.expose(), plaintext(&key), "{key}");
        }
        cleanup(pool).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_sealed_row_is_left_byte_and_timestamp_identical() {
    run(async |pool| {
        cleanup(pool).await;
        let sealed_key = key_name(SecretPurpose::SearchApiKey, "presealed");
        let foreign_key = key_name(SecretPurpose::GitWebhookSigningSecret, "foreign");
        let legacy_key = key_name(SecretPurpose::DoclingVlmApiKey, "legacy");
        store(pool)
            .write(
                SecretPurpose::SearchApiKey,
                &sealed_key,
                b"synthetic-already-sealed",
            )
            .await
            .expect("write an already sealed row");
        // Sealed under a *different* key and version: the sweep must not open
        // it, so it must not reject it either.
        other_store(pool)
            .write(
                SecretPurpose::GitWebhookSigningSecret,
                &foreign_key,
                b"synthetic-foreign-key",
            )
            .await
            .expect("write a row sealed under the other key");
        insert_legacy(pool, &legacy_key, &plaintext(&legacy_key)).await;

        let before_foreign = stored_row(pool, &foreign_key).await;
        let before_sealed = stored_row(pool, &sealed_key).await;
        let report = store(pool).seal_legacy_plaintext().await.expect("seal");

        assert_eq!(report.sealed, 1, "only the legacy row is the sweep's work");
        assert_eq!(stored_row(pool, &foreign_key).await, before_foreign);
        assert_eq!(stored_row(pool, &sealed_key).await, before_sealed);
        // The foreign row still opens only under the key that sealed it.
        assert_eq!(
            other_store(pool)
                .get(SecretPurpose::GitWebhookSigningSecret, &foreign_key)
                .await
                .expect("open under the sealing key")
                .expect("the row is stored")
                .expose(),
            b"synthetic-foreign-key"
        );
        cleanup(pool).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn the_row_identity_and_its_timestamps_survive_the_conversion() {
    run(async |pool| {
        cleanup(pool).await;
        let key = key_name(SecretPurpose::GitProviderToken, "timestamps");
        insert_legacy(pool, &key, &plaintext(&key)).await;
        // A legacy row's `updated_at` is NULL, and that is a real fact about it:
        // the row records when a *value* was written, and a representation
        // change is not one. The sweep has to leave that NULL alone.
        let before = stored_row(pool, &key).await;
        assert_eq!(before.updated_at, None);

        store(pool).seal_legacy_plaintext().await.expect("seal");

        let after = stored_row(pool, &key).await;
        assert_eq!(after.key, before.key);
        assert_eq!(after.created_at, before.created_at);
        assert_eq!(after.updated_at, before.updated_at);
        assert_ne!(after.value, before.value, "the representation changed");
        cleanup(pool).await;
    });
}

// --- batching, resumability, idempotence ---------------------------------------

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn more_rows_than_one_batch_are_sealed_across_batches() {
    run(async |pool| {
        cleanup(pool).await;
        let purpose = SecretPurpose::SourceConnectionDatabaseUrl;
        let total = BATCH + BATCH / 2 + 5;
        for index in 0..total {
            let key = key_name(purpose, &format!("many-{index:04}"));
            insert_legacy(pool, &key, &plaintext(&key)).await;
        }

        let report = store(pool).seal_legacy_plaintext().await.expect("seal");

        assert_eq!(report.sealed, total);
        assert_eq!(
            report.batches,
            total.div_ceil(BATCH),
            "the work is committed one bounded batch at a time"
        );
        assert_eq!(legacy_count(pool).await, 0);
        // Spot-check the two ends of the ordered worklist, so the batching is
        // seen to have covered the whole range rather than a prefix of it.
        for index in [0, total - 1] {
            let key = key_name(purpose, &format!("many-{index:04}"));
            assert_eq!(
                store(pool)
                    .get(purpose, &key)
                    .await
                    .expect("open the sealed row")
                    .expect("the row is stored")
                    .expose(),
                plaintext(&key),
                "{key}"
            );
        }
        cleanup(pool).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_second_run_reports_no_work() {
    run(async |pool| {
        cleanup(pool).await;
        let key = key_name(SecretPurpose::EmbeddingApiKey, "idempotent");
        insert_legacy(pool, &key, &plaintext(&key)).await;

        let first = store(pool).seal_legacy_plaintext().await.expect("seal");
        let second = store(pool)
            .seal_legacy_plaintext()
            .await
            .expect("seal again");

        assert_eq!(first.sealed, 1);
        assert_eq!(second.sealed, 0, "a sealed row is never offered again");
        assert_eq!(second.batches, 0);
        assert_eq!(legacy_count(pool).await, 0);
        cleanup(pool).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_deployment_without_a_key_leaves_every_legacy_row_alone() {
    run(async |pool| {
        cleanup(pool).await;
        let key = key_name(SecretPurpose::TranslationProviderApiKey, "unkeyed");
        insert_legacy(pool, &key, &plaintext(&key)).await;
        let before = stored_row(pool, &key).await;

        let report = unkeyed_store(pool)
            .seal_legacy_plaintext()
            .await
            .expect("an unkeyed deployment has nothing to seal");

        assert_eq!(
            format!("{report:?}"),
            "SealedLegacyReport { sealed: 0, batches: 0 }"
        );
        assert_eq!(stored_row(pool, &key).await, before, "the row is untouched");
        assert_eq!(legacy_count(pool).await, 1);
        // And it is still readable, byte for byte, by the deployment that wrote it.
        assert_eq!(
            unkeyed_store(pool)
                .get(SecretPurpose::TranslationProviderApiKey, &key)
                .await
                .expect("read the legacy row")
                .expect("the legacy row is stored")
                .expose(),
            plaintext(&key)
        );
        cleanup(pool).await;
    });
}
