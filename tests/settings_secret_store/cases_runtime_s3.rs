//! The runtime S3 secret key singleton, round-tripped on a real database.
//!
//! The access key is a non-secret identifier and stays in the settings row with
//! its existing outward behavior; only the secret key is sealed. That split is
//! what these cases check alongside the round trip.

use super::support::{
    assert_fails_closed, assert_sealed_under, assert_stored_bytes_exclude, keyed, reset, run,
    runtime_request, s3_request, service, unkeyed,
};
use context69::services::secret_store::key_names;
use sqlx::Row;

const SYNTHETIC_KEY: &str = "synthetic-s3-secret-key";
const PURPOSE: &str = "runtime_s3.secret_key";

async fn legacy_row(db: &context69::db::Database) -> Option<(String, String)> {
    let row = sqlx::query(
        "SELECT s3_access_key, s3_secret_key \
         FROM context69.runtime_file_library_settings WHERE singleton",
    )
    .fetch_optional(db.pool())
    .await
    .expect("read the legacy columns");
    row.map(|row| {
        (
            row.get::<Option<String>, _>("s3_access_key")
                .unwrap_or_default(),
            row.get::<Option<String>, _>("s3_secret_key")
                .unwrap_or_default(),
        )
    })
}

#[test]
fn the_runtime_s3_secret_key_round_trips_sealed_and_mirrors_the_legacy_column() {
    run(async |db| {
        reset(db).await;

        let settings = service(db, keyed(db));
        let saved = settings
            .update_runtime_settings(&runtime_request(
                None,
                Some(s3_request(Some(SYNTHETIC_KEY))),
            ))
            .await
            .expect("save the s3 secret key");
        let s3 = saved.file_library.s3.expect("the s3 block is stored");
        assert!(s3.has_secret_key, "presence must be reported");

        assert_sealed_under(db.pool(), key_names::RUNTIME_S3_SECRET_KEY, PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::RUNTIME_S3_SECRET_KEY, SYNTHETIC_KEY)
            .await;
        let (access_key, secret_key) = legacy_row(db).await.expect("the s3 row is stored");
        assert_eq!(
            access_key, "AKIACONTEXT69TESTONLY",
            "the access key is an identifier"
        );
        assert_eq!(secret_key, SYNTHETIC_KEY);

        // Store-first, observed through the write path. The mirror is made stale, and
        // a save that omits the secret key resolves the value in effect — which is the
        // sealed row, not the mirror — so the mirror is rewritten to the stored value.
        // The whole runtime S3 block is only readable while the mirror is complete,
        // so a blanked mirror could not have shown this.
        sqlx::query(
            "UPDATE context69.runtime_file_library_settings \
         SET s3_secret_key = 'stale-mirror-value' WHERE singleton",
        )
        .execute(db.pool())
        .await
        .expect("make the mirror stale");
        settings
            .update_runtime_settings(&runtime_request(None, Some(s3_request(None))))
            .await
            .expect("save while keeping the secret key");
        let (_, secret_key) = legacy_row(db).await.expect("the s3 row is stored");
        assert_eq!(
            secret_key, SYNTHETIC_KEY,
            "the store, not the stale mirror, decides the value in effect"
        );

        // The response stays redacted: presence is a boolean and the key is never
        // carried back out.
        let read = settings.get_runtime_settings().await.expect("read");
        assert!(
            read.file_library
                .s3
                .as_ref()
                .is_some_and(|s3| s3.has_secret_key)
        );
        assert!(
            !format!("{read:?}").contains(SYNTHETIC_KEY),
            "a settings response must not carry the key back"
        );

        reset(db).await;
    });
}

#[test]
fn a_sealed_s3_secret_key_fails_closed_and_still_reports_presence() {
    run(async |db| {
        reset(db).await;
        service(db, keyed(db))
            .update_runtime_settings(&runtime_request(
                None,
                Some(s3_request(Some(SYNTHETIC_KEY))),
            ))
            .await
            .expect("seed the sealed row");

        // A probe that has to resolve the stored key must fail rather than fall back
        // to the legacy column. It is refused before any network call, so the
        // unreachable endpoint above is never contacted.
        let unkeyed_service = service(db, unkeyed(db));
        assert_fails_closed(unkeyed_service.test_s3_connection(&s3_request(None)).await);
        assert!(
            unkeyed_service
                .get_runtime_settings()
                .await
                .expect("read presence without a master key")
                .file_library
                .s3
                .as_ref()
                .is_some_and(|s3| s3.has_secret_key)
        );

        reset(db).await;
    });
}

#[test]
fn an_absent_s3_secret_key_keeps_the_stored_one() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(
                None,
                Some(s3_request(Some(SYNTHETIC_KEY))),
            ))
            .await
            .expect("seed the sealed row");

        // An absent or blank secret key has never been a clear: it keeps what is
        // stored instead of blanking the row.
        for keep in [None, Some("   ")] {
            settings
                .update_runtime_settings(&runtime_request(None, Some(s3_request(keep))))
                .await
                .expect("save while keeping the secret key");
            assert_sealed_under(db.pool(), key_names::RUNTIME_S3_SECRET_KEY, PURPOSE).await;
            let (_, secret_key) = legacy_row(db).await.expect("the s3 row is stored");
            assert_eq!(secret_key, SYNTHETIC_KEY);
        }

        reset(db).await;
    });
}
