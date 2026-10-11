//! What a keyed startup does when it meets a row it cannot convert.
//!
//! A row the catalogue does not own, and a row whose markers disagree, both
//! stop the run — atomically, with a sanitized category, and without hiding
//! the row from the next attempt.
//!
//! See [`super`] for the shared harness and the safety rules these cases run under.

use super::*;

// --- refusals -----------------------------------------------------------------

#[test]
fn a_key_name_the_catalogue_does_not_own_stops_the_run_without_leaking() {
    run(async |pool| {
        cleanup(pool).await;
        // One supported row, so the run has real work, and one it cannot name.
        let supported = key_name(SecretPurpose::EmbeddingApiKey, "before-unknown");
        insert_legacy(pool, &supported, &plaintext(&supported)).await;
        let unknown_key = "c69p1sealtest-unowned-secret";
        let unknown_value = plaintext(unknown_key);
        insert_legacy(pool, unknown_key, &unknown_value).await;
        let before = stored_row(pool, &supported).await;

        let error = store(pool)
            .seal_legacy_plaintext()
            .await
            .expect_err("an unnameable row must stop the run");
        assert_eq!(format!("{error:?}"), "UnownedKey");

        // The refusal is a category, and nothing about the row travels with it.
        let rendered = format!("{error}");
        assert!(
            !rendered.contains(unknown_key),
            "{rendered} leaked the key name"
        );
        assert!(
            !rendered.contains(unknown_key),
            "{rendered} leaked the value"
        );
        assert!(
            !rendered.contains(MASTER_KEY_B64),
            "{rendered} leaked key material"
        );
        assert!(!rendered.contains("SELECT"), "{rendered} leaked SQL");

        // The batch rolled back whole, so the row before it in the same batch is
        // exactly as it was — not half converted.
        assert_eq!(stored_row(pool, &supported).await, before);
        assert_eq!(legacy_count(pool).await, 2);
        cleanup(pool).await;
    });
}

#[test]
fn a_legacy_row_whose_markers_disagree_stops_the_run() {
    run(async |pool| {
        cleanup(pool).await;
        // Marked as legacy plaintext but carrying a key version: the table's
        // check allows it, so the sweep has to meet it and refuse rather than
        // strand it outside the worklist.
        let anomalous = key_name(SecretPurpose::GitProviderToken, "anomalous");
        insert_anomalous_legacy(pool, &anomalous, &plaintext(&anomalous)).await;

        let error = store(pool)
            .seal_legacy_plaintext()
            .await
            .expect_err("a malformed marker must stop the run");
        assert_eq!(format!("{error:?}"), "MalformedLegacyRow");
        assert!(!format!("{error}").contains(&anomalous));

        // It is still there, still counted as legacy: a refusal must not look
        // like a completed conversion.
        assert_eq!(legacy_count(pool).await, 1);
        let row = stored_row(pool, &anomalous).await;
        assert_eq!(row.ciphertext_version, 0);
        assert_eq!(row.key_version, 4);
        cleanup(pool).await;
    });
}

#[test]
fn a_refused_run_leaves_its_earlier_batches_sealed_and_a_retry_finishes() {
    run(async |pool| {
        cleanup(pool).await;
        let purpose = SecretPurpose::GitWebhookSigningSecret;
        // More than one batch, with the unnameable row last in key order, so the
        // first batches commit before the refusal. The `zzz` prefix is what puts
        // it there: a row the sweep meets first would simply roll its own batch
        // back, which is the refusal case above.
        let total = BATCH + 3;
        for index in 0..total {
            let key = key_name(purpose, &format!("resume-{index:04}"));
            insert_legacy(pool, &key, &plaintext(&key)).await;
        }
        let unknown_key = "zzz-c69p1sealtest-resume-unowned";
        insert_legacy(pool, unknown_key, &plaintext(unknown_key)).await;

        let refused = store(pool).seal_legacy_plaintext().await;
        assert_eq!(format!("{:?}", refused.expect_err("stopped")), "UnownedKey");
        // Everything the first batches committed stays sealed ...
        assert_eq!(
            legacy_count(pool).await as usize,
            total - BATCH + 1,
            "the refused batch rolled back whole and the earlier batches did not"
        );
        let first = key_name(purpose, "resume-0000");
        assert_eq!(
            stored_row(pool, &first).await.ciphertext_version,
            SEALED_CIPHERTEXT_VERSION
        );

        // ... and once the operator has resolved the row, the next startup
        // resumes from what is still plaintext and finishes.
        delete_key(pool, unknown_key).await;
        let retried = store(pool).seal_legacy_plaintext().await.expect("resume");
        assert_eq!(legacy_count(pool).await, 0);
        assert_eq!(
            retried.sealed,
            total - BATCH,
            "the retry converted exactly the rows the refusal left"
        );
        for index in [0, total - 1] {
            let key = key_name(purpose, &format!("resume-{index:04}"));
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
