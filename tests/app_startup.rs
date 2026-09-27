//! Regression test for the `Context69App::new` startup path (issue 606).
//!
//! Real startup must survive future splitting of `Context69App::new`: the
//! database migrates, the passed config is imported once into the runtime
//! settings and loaded back onto the effective config, an unreachable Qdrant
//! degrades instead of failing startup, the library dependency gates persist
//! that degradation, and a restart keeps using the persisted runtime settings
//! rather than a freshly passed config.
//!
//! Opt-in: runs only for `CONTEXT69_TEST_APP_STARTUP=1` plus
//! `CONTEXT69_TEST_DATABASE_URL`.
//!
//! Safety: Qdrant and the embedding endpoint point at an unreachable loopback
//! sentinel, so startup degrades locally and never reaches an external service.
//! Runtime settings written by an earlier run of this test are reused; foreign
//! runtime settings and non-empty work tables are rejected before startup. The
//! file library uses isolated temp roots; the app runs on this test's own
//! current-thread Tokio runtime so background tasks cannot outlive the test; and
//! only rows and directories this test created are deleted again.

#[path = "app_startup/support.rs"]
mod support;

use context69::{
    config::DEFAULT_SESSION_VALKEY_URL, db::Database, library_store::LibraryStore,
    services::app::Context69App,
};
use uuid::Uuid;

use crate::support::{
    APP_STARTUP_ENV_VAR, TEST_DATABASE_ENV_VAR, assert_open_gate, assert_runtime_settings_applied,
    assert_test_owned_runtime_settings, cleanup_own_rows, internal_secret_key_for_value,
    internal_secret_keys, remove_test_storage_roots, search_request, test_config,
    test_database_url, test_storage_root, work_row_count,
};

/// Degradation recorded by `LibraryService::initialize_dependency_gates` when
/// the embedding provider builds but no vector runtime could be connected.
const RUNTIME_UNAVAILABLE: &str = "embedding/vector runtime is unavailable";
const DOCLING_UNCONFIGURED: &str = "configuration: docling runtime is not configured";
/// `QueryService::disabled` marks the vector index unavailable, so a degraded
/// search fails at that guard with this exact message, before reaching the
/// "search runtime is not configured" branch behind it.
const DEGRADED_SEARCH_ERROR: &str =
    "vector index is rebuilding or unavailable; retry after the rebuild completes";

#[tokio::test(flavor = "current_thread")]
async fn app_startup_degrades_without_qdrant_and_reuses_persisted_runtime_settings() {
    let Some(database_url) = test_database_url() else {
        eprintln!("set {APP_STARTUP_ENV_VAR}=1 and {TEST_DATABASE_ENV_VAR} to run this test");
        return;
    };
    let db = Database::connect(&database_url);
    let db = db.await.expect("connect test database");

    let settings_before = db.get_runtime_settings().await.expect("read settings");
    if let Some(before) = &settings_before {
        assert_test_owned_runtime_settings(before);
    }
    let message = "refusing to start the app: the scratch database already holds work startup would resume or clean up";
    assert_eq!(work_row_count(&db).await, 0, "{message}");
    let secret_keys_before = internal_secret_keys(&db).await;

    // First boot: the fresh database imports the passed config into runtime
    // settings and loads them back before vector, library, and worker startup.
    let first_marker = Uuid::new_v4().simple().to_string();
    let first_root = test_storage_root(&first_marker);
    let first_config = test_config(&database_url, &first_marker, &first_root);
    let app = Context69App::new(first_config.clone())
        .await
        .expect("app startup");
    let first_effective = app.config.clone();
    let stored = app.db.get_runtime_settings().await.expect("read settings");
    let stored = stored.expect("startup must persist runtime settings");
    let store = LibraryStore::new(app.db.clone());
    let gates = store.list_dependency_gates().await.expect("gates");
    let signing_key = app.browser_sessions.signing_key.to_vec();
    let session_valkey_url = app.browser_sessions.valkey_url.clone();
    let search = app.query.search(None, search_request());
    let search_error = search.await.err().map(|error| error.to_string());
    let first_root_created = first_effective.file_library.storage_root.is_dir();
    let signing_key_name = internal_secret_key_for_value(&app.db, &signing_key).await;
    let signing_key_name = signing_key_name.expect("startup must persist the signing key");
    drop(app);

    // Restart on the same database: the persisted runtime settings must win
    // over the freshly passed config (config override timing).
    let restart_marker = Uuid::new_v4().simple().to_string();
    let restart_root = test_storage_root(&restart_marker);
    let restart_config = test_config(&database_url, &restart_marker, &restart_root);
    let app = Context69App::new(restart_config.clone());
    let app = app.await.expect("restart startup");
    let restart_effective = app.config.clone();
    drop(app);
    let restart_root_created = restart_root.exists();

    // Clean up this run's own rows and temp roots before asserting, so a
    // failing assertion never leaves imported runtime settings behind.
    let settings_created = settings_before.is_none();
    let key_created = !secret_keys_before.contains(&signing_key_name);
    cleanup_own_rows(
        &db,
        settings_created,
        key_created.then_some(signing_key_name.as_str()),
    )
    .await;
    let first_used_root = first_effective.file_library.storage_root.clone();
    let restart_used_root = restart_effective.file_library.storage_root.clone();
    remove_test_storage_roots([first_used_root.clone(), restart_used_root.clone()]);

    // Import timing: the first boot on a fresh database persists exactly the
    // passed config; later boots must not re-import over existing settings.
    if settings_created {
        assert_runtime_settings_applied(&first_config, &stored);
    } else {
        let before = settings_before.as_ref().expect("checked above");
        assert_eq!(stored.scheduler.job_id, before.scheduler.job_id);
    }
    // Load/apply timing: both boots run with the persisted runtime settings.
    assert_runtime_settings_applied(&first_effective, &stored);
    assert_runtime_settings_applied(&restart_effective, &stored);
    assert_ne!(
        restart_effective.scheduler.job_id,
        restart_config.scheduler.job_id
    );
    assert_eq!(restart_used_root, first_used_root);
    let (effective_ttl, passed_ttl) = (
        first_effective.scheduler.execution_guard_ttl,
        first_config.scheduler.execution_guard_ttl,
    );
    assert!(effective_ttl == passed_ttl, "guard ttl changed");
    let url_unchanged = first_effective.app_db.url == database_url;
    assert!(url_unchanged, "startup changed the app db url");

    // Qdrant degradation: startup still succeeds and the library dependency
    // gates record that embedding was configured but no vector runtime exists.
    assert_open_gate(&gates, "embedding", RUNTIME_UNAVAILABLE);
    assert_open_gate(&gates, "qdrant", RUNTIME_UNAVAILABLE);
    assert_open_gate(&gates, "embedding_vector", RUNTIME_UNAVAILABLE);
    assert_open_gate(&gates, "docling", DOCLING_UNCONFIGURED);
    let search_error = search_error.expect("search must fail while degraded");
    assert_eq!(
        search_error, DEGRADED_SEARCH_ERROR,
        "degraded search branch"
    );

    // Browser session runtime resolved from the database without valkey.
    assert_eq!(session_valkey_url, DEFAULT_SESSION_VALKEY_URL);
    assert_eq!(signing_key.len(), 64);
    assert!(signing_key.iter().any(|byte| *byte != 0));

    // File library: the persisted storage root is created on disk, and the
    // freshly passed root on restart is never touched.
    assert!(first_root_created, "library storage root must exist");
    assert!(
        !restart_root_created,
        "restart must not create the passed root"
    );

    let message = "startup must not create task, library, or metadata-index work rows";
    assert_eq!(work_row_count(&db).await, 0, "{message}");
}
