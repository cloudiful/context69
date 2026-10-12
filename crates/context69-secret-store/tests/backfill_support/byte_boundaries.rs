use super::*;

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn empty_and_binary_values_and_non_null_timestamps_survive_sealing() {
    run(async |pool| {
        cleanup(pool).await;
        let purpose = SecretPurpose::SourceConnectionDatabaseUrl;
        let empty = key_name(purpose, "reviewer-empty");
        let prefix = key_name(purpose, "reviewer-binary._-");
        let binary = format!("{prefix}{}", "x".repeat(128 - prefix.len()));
        assert_eq!(binary.len(), 128);
        let bytes: Vec<u8> = (0..=255).collect();
        insert_legacy(pool, &empty, b"").await;
        insert_legacy(pool, &binary, &bytes).await;
        sqlx::query(
            "UPDATE context69.internal_secrets \
             SET created_at = '2020-01-02 03:04:05.123456+00', \
                 updated_at = '2021-02-03 04:05:06.654321+00' WHERE key = $1",
        )
        .bind(&binary)
        .execute(pool)
        .await
        .expect("synthetic timestamps");
        let before = stored_row(pool, &binary).await;
        assert!(before.updated_at.is_some());
        assert_eq!(
            store(pool)
                .seal_legacy_plaintext()
                .await
                .expect("seal")
                .sealed,
            2
        );
        for (key, expected) in [(&empty, &[][..]), (&binary, bytes.as_slice())] {
            assert_eq!(
                store(pool)
                    .get(purpose, key)
                    .await
                    .expect("open bytes")
                    .expect("row remains")
                    .expose(),
                expected
            );
        }
        let after = stored_row(pool, &binary).await;
        assert_eq!(after.key, before.key);
        assert_eq!(after.created_at, before.created_at);
        assert_eq!(after.updated_at, before.updated_at);
        assert_ne!(after.value, before.value);
        cleanup(pool).await;
    });
}
