//! Honest liveness and readiness (issue 942 P2).
//!
//! `/livez` answers for the process only; `/healthz` and `/readyz` answer for
//! the work this instance can actually do. The case below boots the real
//! application against a scratch database with unreachable dependency
//! sentinels, so the database answers while the vector runtime and the
//! library processing gates do not. That is exactly the shape the old
//! `/healthz` reported as `200`.
//!
//! Opt-in: runs only for `CONTEXT69_TEST_APP_STARTUP=1` plus
//! `CONTEXT69_TEST_DATABASE_URL`, and is classified `app_startup` in the
//! runner manifest.
//!
//! Safety: Qdrant and the embedding endpoint point at an unreachable loopback
//! sentinel, so startup never reaches an external service. Foreign runtime
//! settings and a scratch database holding work rows are rejected before
//! startup, and only rows and the directory this run created are removed.

#[path = "runtime_readiness/support.rs"]
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use context69::{db::Database, services::app::Context69App};
use serde_json::Value;
use tokio::net::TcpListener;
use uuid::Uuid;

use support::{
    assert_test_owned_runtime_settings, cleanup_own_rows, internal_secret_key_for_value,
    internal_secret_keys, remove_test_storage_root, test_config, test_storage_root, work_row_count,
};

/// A readiness answer probes its dependencies under its own bounds, so a slow
/// dependency must not hold the route open past this ceiling.
const READY_TIMEOUT: Duration = Duration::from_secs(10);

async fn get_json(base: &str, path: &str) -> (reqwest::StatusCode, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .send()
        .await
        .unwrap_or_else(|error| panic!("{path} must answer: {error}"));
    let status = response.status();
    let body = response.json().await.expect("health body must be JSON");
    (status, body)
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires configured PostgreSQL and an opt-in application-startup run"]
async fn liveness_stays_up_while_readiness_reports_the_vector_runtime_honestly() {
    let database_url = context69_test_support::accessor::database_url();
    let db = Database::connect(&database_url)
        .await
        .expect("connect test database");

    let settings_before = db.get_runtime_settings().await.expect("read settings");
    if let Some(before) = &settings_before {
        assert_test_owned_runtime_settings(before);
    }
    assert_eq!(
        work_row_count(&db).await,
        0,
        "refusing to boot: the scratch database already holds work startup would resume or clean up"
    );
    let secret_keys_before = internal_secret_keys(&db).await;

    let marker = Uuid::new_v4().simple().to_string();
    let root = test_storage_root(&marker);
    let config = test_config(&database_url, &marker, &root);
    let started = Instant::now();
    let app = Arc::new(Context69App::new(config).await.expect("app startup"));
    let boot_elapsed = started.elapsed();

    let router = context69::api::router(Arc::clone(&app))
        .await
        .expect("api router");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral loopback port must be available");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let base = format!("http://{address}");

    let live_started = Instant::now();
    let (live_status, live_body) = get_json(&base, "/livez").await;
    let live_elapsed = live_started.elapsed();
    let (ready_status, ready_body) = get_json(&base, "/readyz").await;
    let health_started = Instant::now();
    let (health_status, health_body) = get_json(&base, "/healthz").await;
    let health_elapsed = health_started.elapsed();
    let signing_key = app.browser_sessions.signing_key.to_vec();
    let signing_key_name = internal_secret_key_for_value(&app.db, &signing_key).await;

    server.abort();
    drop(app);

    let settings_created = settings_before.is_none();
    let key_created = signing_key_name
        .as_deref()
        .is_some_and(|name| !secret_keys_before.iter().any(|known| known == name));
    cleanup_own_rows(
        &db,
        settings_created,
        key_created.then_some(signing_key_name.as_deref().expect("key name")),
    )
    .await;
    remove_test_storage_root(&root);

    // Liveness: the process answers without touching a dependency, even though
    // the database-backed boot already proved the dependencies are degraded.
    assert_eq!(
        live_status,
        reqwest::StatusCode::OK,
        "livez body: {live_body}"
    );
    assert_eq!(live_body["status"], "ok");
    assert!(
        live_body.get("db_ok").is_none()
            && live_body.get("qdrant_ok").is_none()
            && live_body.get("library_processing_ready").is_none(),
        "livez must not claim dependency state: {live_body}"
    );
    assert!(
        live_elapsed < Duration::from_secs(2),
        "livez must not wait on a dependency, took {live_elapsed:?}"
    );

    // Readiness: the database answers, the vector runtime cannot, so both
    // readiness routes must answer 503 and say which dependency failed.
    for (path, status, body) in [
        ("/readyz", ready_status, &ready_body),
        ("/healthz", health_status, &health_body),
    ] {
        assert_eq!(
            status,
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            "{path} must not report ready without a vector runtime: {body}"
        );
        assert_eq!(body["status"], "degraded", "{path}");
        assert_eq!(body["db_ok"], true, "{path} must report the database truth");
        assert_eq!(
            body["qdrant_ok"], false,
            "{path} must report the vector runtime truth"
        );
        assert_eq!(
            body["library_processing_ready"], false,
            "{path} must report the processing verdict truthfully"
        );
        assert!(
            body.get("indexed_chunks").is_none(),
            "{path} must not report a chunk count it could not read: {body}"
        );
    }

    // The readiness answer is bounded and does not wait for the startup budget.
    assert!(
        health_elapsed < READY_TIMEOUT,
        "readiness must answer inside its probe bound, took {health_elapsed:?}"
    );
    assert!(
        boot_elapsed < Duration::from_secs(90),
        "startup must stay bounded, took {boot_elapsed:?}"
    );
}
