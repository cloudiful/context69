//! The runtime S3 secret key singleton, round-tripped on a real database.
//!
//! The access key is a non-secret identifier and stays in the settings row with
//! its existing outward behavior; only the secret key is sealed, and the store is
//! its only representation. That split is what these cases check alongside the
//! round trip.

use super::support::{
    assert_fails_closed, assert_retired_columns_absent, assert_sealed_under,
    assert_stored_bytes_exclude, keyed, reset, run, runtime_request, s3_request, service, unkeyed,
};
use context69::services::secret_store::key_names;
use sqlx::Row;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SYNTHETIC_KEY: &str = "synthetic-s3-secret-key";
const PURPOSE: &str = "runtime_s3.secret_key";

/// The access key the settings row keeps. The secret key has no column to read.
async fn access_key(db: &context69::db::Database) -> Option<String> {
    let row = sqlx::query(
        "SELECT s3_access_key FROM context69.runtime_file_library_settings WHERE singleton",
    )
    .fetch_optional(db.pool())
    .await
    .expect("read the access key");
    row.map(|row| {
        row.get::<Option<String>, _>("s3_access_key")
            .unwrap_or_default()
    })
}

#[test]
fn the_runtime_s3_secret_key_round_trips_sealed_and_leaves_no_plaintext_column() {
    run(async |db| {
        reset(db).await;
        assert_retired_columns_absent(db).await;

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
        assert_eq!(s3.access_key, "AKIACONTEXT69TESTONLY");

        assert_sealed_under(db.pool(), key_names::RUNTIME_S3_SECRET_KEY, PURPOSE).await;
        assert_stored_bytes_exclude(db.pool(), key_names::RUNTIME_S3_SECRET_KEY, SYNTHETIC_KEY)
            .await;
        // The identifiers stay in the settings row; the credential does not.
        assert_eq!(
            access_key(db).await.as_deref(),
            Some("AKIACONTEXT69TESTONLY")
        );
        assert_retired_columns_absent(db).await;

        // A save that omits the secret key keeps the stored one, so presence and the
        // non-secret block both survive it.
        settings
            .update_runtime_settings(&runtime_request(None, Some(s3_request(None))))
            .await
            .expect("save while keeping the secret key");
        assert_sealed_under(db.pool(), key_names::RUNTIME_S3_SECRET_KEY, PURPOSE).await;

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

        // A probe that has to resolve the stored key must fail rather than report
        // none. It is refused before any network call, so the unreachable endpoint
        // above is never contacted.
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
        // stored instead of dropping the credential.
        for keep in [None, Some("   ")] {
            settings
                .update_runtime_settings(&runtime_request(None, Some(s3_request(keep))))
                .await
                .expect("save while keeping the secret key");
            assert_sealed_under(db.pool(), key_names::RUNTIME_S3_SECRET_KEY, PURPOSE).await;
        }
        assert!(
            settings
                .get_runtime_settings()
                .await
                .expect("read after the keeps")
                .file_library
                .s3
                .as_ref()
                .is_some_and(|s3| s3.has_secret_key)
        );

        reset(db).await;
    });
}

/// A runtime settings save must notify the observer so the S3 dependency gate
/// can refresh and re-probe; the connection test stays a pure read-only probe
/// and must never notify.
#[test]
fn a_runtime_settings_save_notifies_the_observer_but_a_connection_test_does_not() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        let notifications = Arc::new(AtomicUsize::new(0));
        let counter = notifications.clone();
        settings.set_runtime_settings_observer(Some(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })));

        settings
            .update_runtime_settings(&runtime_request(
                None,
                Some(s3_request(Some(SYNTHETIC_KEY))),
            ))
            .await
            .expect("save the runtime s3 settings");
        assert_eq!(
            notifications.load(Ordering::SeqCst),
            1,
            "a runtime settings save must notify the observer once"
        );

        // The connection test is a pure read-only probe against an unreachable
        // loopback endpoint: it may fail, but it must not notify the observer or
        // touch the persisted settings. Install the process-wide HTTP transport
        // the production binary installs so the probe makes a real attempt.
        opendal::install_default();
        let probe = settings.test_s3_connection(&s3_request(None)).await;
        assert!(
            probe.is_err(),
            "the unreachable endpoint must fail the check"
        );
        assert_eq!(
            notifications.load(Ordering::SeqCst),
            1,
            "test_s3_connection must stay read-only and never notify the observer"
        );

        reset(db).await;
    });
}
