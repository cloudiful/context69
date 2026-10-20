//! Process-level startup-recovery regression for issue #955 P2.
//!
//! The runner resets the owned schema and applies the ordinary migration chain
//! only through the migration immediately before the pinned retirement
//! migration, so this test starts from a real pending migration. It runs the
//! real `context69 serve` binary once and drives the whole lifecycle:
//!
//! - the recovery profile serves only liveness and explicit unready health on
//!   the configured API port, and no other ingress;
//! - the real dispatcher reclaims the seeded expired claim and adopts the
//!   persisted Docling remote id against a deterministic local fake, which
//!   keeps the conversion pending so the migration guard stays set;
//! - once the fake reports a terminal failure, the recovery writers quiesce, the
//!   unchanged retirement migration applies exactly once together with the later
//!   pending migrations, and the normal application rebinds the same port with
//!   ordinary readiness;
//! - SIGTERM then stops that normal application under the bounded budget.
//!
//! Registered as the opt-in `upgrade_recovery` class; it is ignored unless the
//! runner selects it explicitly.

use std::{
    net::TcpListener,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use context69::{
    config::Config,
    db::{Database, MIGRATOR, RETIREMENT_VERSION, UPGRADE_PRIOR_VERSION},
    services::app::UpgradeRecoveryApp,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::process::Child;
use uuid::Uuid;

#[path = "upgrade_recovery/object_cleanup.rs"]
mod object_cleanup;

const PRIOR_VERSION: i64 = UPGRADE_PRIOR_VERSION;
const PINNED_RETIREMENT_VERSION: i64 = RETIREMENT_VERSION;
const SENTINEL_URL: &str = "http://127.0.0.1:1";

#[derive(Default)]
struct FakeDoclingState {
    polls: Mutex<Vec<String>>,
    cancels: Mutex<Vec<String>>,
    submits: Mutex<u64>,
    finish: std::sync::atomic::AtomicBool,
}

struct FakeDocling {
    base_url: String,
    state: Arc<FakeDoclingState>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    server: tokio::task::JoinHandle<()>,
}

impl FakeDocling {
    async fn start() -> Self {
        let state = Arc::new(FakeDoclingState::default());
        let app = Router::new()
            .route("/v1/status/poll/{task_id}", get(poll_status))
            .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
            .route("/v1/convert/source", post(record_submit))
            .route("/v1/convert/file", post(record_submit))
            .route("/health", get(|| async { StatusCode::OK }))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the fake Docling server");
        let address = listener.local_addr().expect("fake Docling address");
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        Self {
            base_url: format!("http://{address}"),
            state,
            shutdown: Some(shutdown),
            server,
        }
    }

    fn polls(&self) -> Vec<String> {
        self.state.polls.lock().expect("fake polls").clone()
    }

    fn cancels(&self) -> usize {
        self.state.cancels.lock().expect("fake cancels").len()
    }

    fn submits(&self) -> u64 {
        *self.state.submits.lock().expect("fake submits")
    }

    /// Let the fake conversion reach its terminal failure state.
    fn finish(&self) {
        self.state
            .finish
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = self.server.await;
    }
}

async fn poll_status(
    State(state): State<Arc<FakeDoclingState>>,
    Path(task_id): Path<String>,
) -> Json<Value> {
    state
        .polls
        .lock()
        .expect("fake polls")
        .push(task_id.clone());
    if state.finish.load(std::sync::atomic::Ordering::SeqCst) {
        Json(json!({
            "task_id": task_id,
            "task_status": "failure",
            "error_message": "deterministic fake conversion failure",
        }))
    } else {
        Json(json!({
            "task_id": task_id,
            "task_status": "started",
        }))
    }
}

async fn cancel_task(
    State(state): State<Arc<FakeDoclingState>>,
    Path(task_id): Path<String>,
) -> StatusCode {
    state.cancels.lock().expect("fake cancels").push(task_id);
    StatusCode::NO_CONTENT
}

async fn record_submit(State(state): State<Arc<FakeDoclingState>>) -> StatusCode {
    *state.submits.lock().expect("fake submits") += 1;
    StatusCode::INTERNAL_SERVER_ERROR
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("ephemeral address").port()
}

struct Fixture {
    group_id: i64,
    user_id: i64,
    document_id: i64,
    library_file_id: Uuid,
    storage_object_id: Uuid,
    task_id: Uuid,
    item_id: Uuid,
    remote_job_id: Uuid,
    remote_task_id: String,
    record_hash: String,
}

async fn seed(pool: &PgPool, docling_base_url: &str) -> Fixture {
    let marker = Uuid::new_v4().simple().to_string();
    let group_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("upgrade-recovery-{marker}"))
    .bind("Upgrade Recovery")
    .bind(format!("test/upgrade-recovery-{marker}"))
    .fetch_one(pool)
    .await
    .expect("seed group");
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash, is_admin) \
         VALUES ($1, 'Upgrade Recovery Admin', 'unused', TRUE) RETURNING id",
    )
    .bind(format!("upgrade-recovery-{marker}"))
    .fetch_one(pool)
    .await
    .expect("seed task owner");

    sqlx::query(
        "INSERT INTO context69.docling_settings \
         (singleton, base_url, timeout_secs, poll_interval_secs) \
         VALUES (TRUE, $1, 10, 1) \
         ON CONFLICT (singleton) DO UPDATE SET base_url = EXCLUDED.base_url",
    )
    .bind(docling_base_url)
    .execute(pool)
    .await
    .expect("seed docling settings");

    let record_hash = format!("upgrade-recovery-hash-{marker}");
    let document_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, updated_at_source, \
          record_hash, metadata_json, visibility) \
         VALUES ($1, 'upgrade-recovery-src', $2, 'Upgrade Recovery Doc', NULL, \
                 'https://example.test/upgrade-recovery', now(), $3, \
                 '{\"topic\":\"markets\",\"extraction\":{\"stock.news.v1\":{\"direction\":\"positive\"}}}'::jsonb, \
                 'public') \
         RETURNING id",
    )
    .bind(group_id)
    .bind(format!("upgrade-recovery-{marker}"))
    .bind(&record_hash)
    .fetch_one(pool)
    .await
    .expect("seed affected document");
    sqlx::query(
        "INSERT INTO context69.document_versions \
         (document_id, record_hash, title, summary, body_text, source_uri, metadata_json) \
         VALUES ($1, $2, 'Upgrade Recovery Doc', NULL, 'alpha\nbeta', \
                 'https://example.test/upgrade-recovery', '{\"topic\":\"markets\"}'::jsonb)",
    )
    .bind(document_id)
    .bind(&record_hash)
    .execute(pool)
    .await
    .expect("seed canonical snapshot");
    sqlx::query(
        "INSERT INTO context69.document_chunks \
         (id, document_id, chunk_index, chunk_text, record_hash) \
         VALUES ($1, $2, 0, 'alpha', $3), ($4, $2, 1, 'beta', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(document_id)
    .bind(&record_hash)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed chunks");

    let library_file_id = Uuid::new_v4();
    let storage_object_id = Uuid::new_v4();
    let sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
    sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key) \
         VALUES ($1, $2, $3, 5, 'local', $4)",
    )
    .bind(storage_object_id)
    .bind(group_id)
    .bind(sha256)
    .bind(format!("objects/{group_id}/{library_file_id}"))
    .execute(pool)
    .await
    .expect("seed library storage object");
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, \
          sha256, storage_rel_path, storage_object_id, ingest_status) \
         VALUES ($1, $2, 'public', NULL, $3, 'upgrade-recovery.pdf', 'application/pdf', 5, $4, \
                 $5, $6, 'failed')",
    )
    .bind(library_file_id)
    .bind(group_id)
    .bind(format!("upgrade-recovery-{marker}"))
    .bind(sha256)
    .bind(format!("objects/{group_id}/{library_file_id}"))
    .bind(storage_object_id)
    .execute(pool)
    .await
    .expect("seed library file");

    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks \
         (id, user_id, group_id, kind, status, group_path, total_count, queued_count) \
         VALUES ($1, $2, $3, 'file_batch', 'queued', 'test/upgrade-recovery', 1, 1)",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(group_id)
    .execute(pool)
    .await
    .expect("seed queued task");

    // Parked with its persisted remote reference and a near retry deadline: the
    // guard stays set, the dispatcher must not claim it yet, and the next boot
    // adopts the same identity without any lease-expiry wait.
    let item_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_items \
         (id, task_id, ordinal, payload, status, stage, attempt_count, retryable, file_id, \
          next_attempt_at, waiting_reason, dependency_key) \
         VALUES ($1, $2, 0, '{}'::jsonb, 'waiting', 'docling', 0, TRUE, $3, \
                 now() + interval '15 seconds', 'dependency', 'docling')",
    )
    .bind(item_id)
    .bind(task_id)
    .bind(library_file_id)
    .execute(pool)
    .await
    .expect("seed parked item with a persisted remote reference");

    let remote_task_id = format!("remote-{marker}");
    let remote_job_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_docling_remote_jobs \
         (id, task_id, item_id, remote_task_id, status, next_poll_at, deadline_at, \
          endpoint_base_url) \
         VALUES ($1, $2, $3, $4, 'pending', now(), now() + interval '1 hour', $5)",
    )
    .bind(remote_job_id)
    .bind(task_id)
    .bind(item_id)
    .bind(&remote_task_id)
    .bind(format!("{docling_base_url}/v1"))
    .execute(pool)
    .await
    .expect("seed pending remote job");

    Fixture {
        group_id,
        user_id,
        document_id,
        library_file_id,
        storage_object_id,
        task_id,
        item_id,
        remote_job_id,
        remote_task_id,
        record_hash,
    }
}

fn server_environment(
    database_url: &str,
    api_port: u16,
    storage_root: &std::path::Path,
    collection: &str,
) -> Vec<(String, String)> {
    let mut environment: Vec<(String, String)> = std::env::vars()
        .filter(|(key, _)| {
            !key.starts_with("PG")
                && !key.starts_with("CONTEXT69_")
                && !key.starts_with("CLOUDIFUL_")
                && key != "DATABASE_URL"
        })
        .collect();
    for (key, value) in [
        ("RUST_LOG", "context69=info,sqlx=warn".to_string()),
        ("DATABASE_URL", database_url.to_string()),
        ("CONTEXT69_API__BIND_ADDR", format!("127.0.0.1:{api_port}")),
        ("CONTEXT69_QDRANT__URL", SENTINEL_URL.to_string()),
        ("CONTEXT69_QDRANT__COLLECTION_NAME", collection.to_string()),
        (
            "CONTEXT69_EMBEDDING__BASE_URL",
            "http://127.0.0.1:1/v1".to_string(),
        ),
        ("CONTEXT69_EMBEDDING__MODEL", "upgrade-recovery".to_string()),
        ("CONTEXT69_EMBEDDING__DIMENSIONS", "128".to_string()),
        (
            "CONTEXT69_FILE_LIBRARY__STORAGE_ROOT",
            storage_root.display().to_string(),
        ),
        ("CONTEXT69_MCP__ENABLED", "false".to_string()),
        ("CONTEXT69_SCHEDULER__RUN_ON_START", "false".to_string()),
    ] {
        environment.push((key.to_string(), value));
    }
    environment
}

struct Server {
    child: Child,
    api_port: u16,
    base: String,
    log_path: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        // A panic must never leave the serving child behind: the runner treats a
        // surviving descendant as a failed target and it would hold the schema.
        let _ = self.child.start_kill();
        let _ = std::fs::remove_file(&self.log_path);
    }
}

impl Server {
    fn spawn(
        database_url: &str,
        api_port: u16,
        storage_root: &std::path::Path,
        collection: &str,
    ) -> Self {
        let binary = env!("CARGO_BIN_EXE_context69");
        let mut command = tokio::process::Command::new(binary);
        command
            .args(["serve"])
            .env_clear()
            .envs(server_environment(
                database_url,
                api_port,
                storage_root,
                collection,
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let log_path = std::env::temp_dir().join(format!(
            "context69-upgrade-recovery-{api_port}-{}.log",
            std::process::id()
        ));
        let log = std::fs::File::create(&log_path).expect("create the server log");
        command.stdout(Stdio::from(log.try_clone().expect("clone log")));
        command.stderr(Stdio::from(log));
        let child = command.spawn().expect("start the context69 server");
        Self {
            child,
            api_port,
            base: format!("http://127.0.0.1:{api_port}"),
            log_path,
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.try_get(path).await.unwrap_or_else(|error| {
            panic!(
                "GET {path} failed: {error}\n--- server log ---\n{}",
                self.log()
            )
        })
    }

    async fn try_get(&self, path: &str) -> reqwest::Result<reqwest::Response> {
        reqwest::Client::new()
            .get(format!("{}{path}", self.base))
            .send()
            .await
    }

    async fn wait_livez(&self) {
        for _ in 0..1200 {
            if let Ok(response) = reqwest::get(format!("{}/livez", self.base)).await
                && response.status() == StatusCode::OK
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the server never became live on {}", self.api_port);
    }

    async fn ready(body: reqwest::Response) -> Value {
        body.json::<Value>().await.unwrap_or_else(|error| {
            panic!("readiness response was not JSON: {error}");
        })
    }

    async fn stop(mut self) {
        let pid = self.child.id().expect("server pid");
        let status = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
        assert!(
            matches!(&status, Ok(status) if status.success()),
            "SIGTERM could not be delivered: {status:?}"
        );
        let exited = tokio::time::timeout(Duration::from_secs(20), self.child.wait()).await;
        match exited {
            Ok(Ok(status)) => assert!(status.success(), "server exited with {status:?}"),
            Ok(Err(error)) => panic!("waiting for the server failed: {error}"),
            Err(_) => {
                let _ = self.child.kill().await;
                panic!("the server did not stop within the bounded shutdown budget");
            }
        }
    }
}

async fn ledger_versions(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar::<_, i64>("SELECT version FROM public._sqlx_migrations ORDER BY version")
        .fetch_all(pool)
        .await
        .expect("read migration ledger")
}

/// The ordered ledger this build applies once every pending migration runs.
///
/// Derived from the compiled chain so a migration added after the pinned
/// retirement never turns a correct startup into a stale expectation.
fn compiled_ledger_versions() -> Vec<i64> {
    MIGRATOR.iter().map(|migration| migration.version).collect()
}

async fn item_status(pool: &PgPool, item_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
        .bind(item_id)
        .fetch_one(pool)
        .await
        .expect("read item status")
}

async fn outbox_table_exists(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT to_regclass('context69.document_metadata_payload_sync')::text",
    )
    .fetch_one(pool)
    .await
    .expect("read outbox table presence")
    .is_some()
}

/// The same deployment-shaped configuration the binary receives, for the
/// in-process recovery owner.
fn recovery_config(database_url: &str, storage_root: &std::path::Path, collection: &str) -> Config {
    let mut config = Config::default();
    config.app_db.url = database_url.to_string();
    config.file_library.storage_root = storage_root.to_path_buf();
    config.qdrant.url = SENTINEL_URL.to_string();
    config.qdrant.collection_name = collection.to_string();
    config.embedding.base_url = "http://127.0.0.1:1/v1".to_string();
    config.embedding.model = "upgrade-recovery".to_string();
    config.embedding.dimensions = 128;
    config.mcp.enabled = false;
    config.scheduler.run_on_start = false;
    config
}

fn run_cli_mode(
    database_url: &str,
    mode: &str,
    storage_root: &std::path::Path,
    collection: &str,
) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_context69");
    std::process::Command::new(binary)
        .arg(mode)
        .env_clear()
        .envs(server_environment(
            database_url,
            free_port(),
            storage_root,
            collection,
        ))
        .output()
        .expect("run context69 in the requested CLI mode")
}

async fn item_snapshot(pool: &PgPool, item_id: Uuid) -> (String, i32, Option<String>) {
    sqlx::query_as(
        "SELECT status, attempt_count, waiting_reason FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(pool)
    .await
    .expect("read the item snapshot")
}

async fn remote_job_snapshot(pool: &PgPool, job_id: Uuid) -> (String, String) {
    sqlx::query_as(
        "SELECT remote_task_id, status FROM context69.task_docling_remote_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
    .expect("read the remote job snapshot")
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires configured reusable PostgreSQL and the runner upgrade-recovery opt-in"]
async fn upgrade_recovery_waits_live_unready_adopts_the_remote_id_and_migrates_once() {
    let database_url = context69_test_support::accessor::database_url();
    let pool = PgPool::connect(&database_url)
        .await
        .expect("connect the runner-owned test schema");
    assert_eq!(
        ledger_versions(&pool).await.last().copied(),
        Some(PRIOR_VERSION),
        "the runner must initialize the schema immediately before the pinned retirement migration"
    );
    assert!(
        !outbox_table_exists(&pool).await,
        "the retirement outbox must not exist before the migration"
    );

    let docling = FakeDocling::start().await;
    let fixture = seed(&pool, &docling.base_url).await;
    let api_port = free_port();
    let storage_root = std::env::temp_dir().join(format!(
        "context69-upgrade-recovery-{}",
        Uuid::new_v4().simple()
    ));
    let collection = format!("upgrade_recovery_{}", Uuid::new_v4().simple());

    // The real recovery cancellation entry must close the real dispatcher gate
    // synchronously, before any listener join is awaited. This composes the
    // production `UpgradeRecoveryApp::cancel_writers` with the production
    // `TaskService`/dispatcher stop gate: if the recovery-to-dispatcher call is
    // missing, `wait_for_dispatch_stop` never resolves and this fails.
    let recovery_db = Database::connect(&database_url)
        .await
        .expect("connect the recovery database");
    // Issue #975 P9: while the schema is still before the artifact migration,
    // every object GC/staging statement must run its known-profile twin instead
    // of failing on the absent artifact relation.
    object_cleanup::exercise_known_profile_object_lifecycle(&recovery_db, &storage_root).await;
    let mut recovery = UpgradeRecoveryApp::with_database(
        recovery_config(&database_url, &storage_root, &collection),
        recovery_db,
    )
    .await
    .expect("build the real recovery profile");
    let held_listener = tokio::spawn(std::future::pending::<()>());
    recovery.cancel_writers();
    assert!(
        !held_listener.is_finished(),
        "the held listener join is not complete at the cancellation entry"
    );
    tokio::time::timeout(
        Duration::from_secs(2),
        recovery.tasks.wait_for_dispatch_stop(),
    )
    .await
    .expect("the recovery cancellation entry closed the real dispatcher gate");
    assert!(
        !held_listener.is_finished(),
        "acquisition is already closed while the listener join is still held"
    );
    // Idempotent, and a repeated stop still joins the real writers.
    recovery.cancel_writers();
    recovery.stop_writers().await;
    held_listener.abort();
    let _ = held_listener.await;
    assert_eq!(
        item_snapshot(&pool, fixture.item_id).await.0,
        "waiting",
        "the cancelled recovery owner never touched the parked item"
    );
    assert_eq!(
        remote_job_snapshot(&pool, fixture.remote_job_id).await,
        (fixture.remote_task_id.clone(), "pending".to_string()),
        "the cancelled recovery owner never fenced the remote job"
    );
    assert!(docling.polls().is_empty(), "no adoption ran in-process");

    // Every ordinary mode must reject the pending schema at the normal
    // constructor boundary, before bootstrap, hydration, sweeps, or writers.
    let rejected = run_cli_mode(
        &database_url,
        "sync-once",
        &storage_root,
        &format!("{collection}_rejected"),
    );
    assert!(
        !rejected.status.success(),
        "sync-once must refuse a pending schema"
    );
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        stderr.contains("requires upgrade recovery"),
        "the rejection must be actionable: {stderr}"
    );
    let bootstrap_admins: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.users WHERE login_name = 'admin'")
            .fetch_one(&pool)
            .await
            .expect("count bootstrap admins");
    assert_eq!(
        bootstrap_admins, 0,
        "the normal constructor must reject before creating the bootstrap admin"
    );
    let runtime_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.runtime_qdrant_settings")
            .fetch_one(&pool)
            .await
            .expect("count runtime settings rows");
    assert_eq!(
        runtime_rows, 0,
        "the normal constructor must reject before importing runtime settings"
    );
    assert_eq!(
        ledger_versions(&pool).await.last().copied(),
        Some(PRIOR_VERSION)
    );
    assert!(!outbox_table_exists(&pool).await);

    // First boot: the park blocks the migration and is not claimable yet, so
    // only liveness and explicit unready health are served. A stop here must be
    // bounded and must leave every persisted identity untouched.
    let first_boot = Server::spawn(&database_url, api_port, &storage_root, &collection);
    first_boot.wait_livez().await;
    let readiness = first_boot.get("/readyz").await;
    assert_eq!(readiness.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = Server::ready(readiness).await;
    assert_eq!(body["upgrade_pending"], json!(true), "{body}");
    assert!(
        body["blocking_task_items"].as_i64().unwrap_or_default() >= 1,
        "{body}"
    );
    assert!(
        docling.polls().is_empty(),
        "a parked item must not be claimed"
    );
    assert_eq!(docling.submits(), 0);
    first_boot.stop().await;

    assert_eq!(
        ledger_versions(&pool).await.last().copied(),
        Some(PRIOR_VERSION),
        "a stop while recovery waits must leave the migration pending"
    );
    assert!(
        !outbox_table_exists(&pool).await,
        "a stop while recovery waits must not touch the migration's outbox"
    );
    assert_eq!(
        item_snapshot(&pool, fixture.item_id).await.0,
        "waiting",
        "the whole item row is unchanged by the stop"
    );
    assert_eq!(
        remote_job_snapshot(&pool, fixture.remote_job_id).await,
        (fixture.remote_task_id.clone(), "pending".to_string()),
        "the persisted remote identity is unchanged by the stop"
    );
    assert_eq!(docling.submits(), 0);

    // Second boot: the retry deadline passes, the real worker adopts the
    // persisted remote id, and the pending migration stays blocked while the
    // conversion is in flight.
    let server = Server::spawn(&database_url, api_port, &storage_root, &collection);
    server.wait_livez().await;
    let readiness = server.get("/readyz").await;
    assert_eq!(readiness.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = Server::ready(readiness).await;
    assert_eq!(body["upgrade_pending"], json!(true), "{body}");
    assert!(
        body["blocking_remote_jobs"].as_i64().unwrap_or_default() >= 1,
        "{body}"
    );
    let health = server.get("/healthz").await;
    assert_eq!(health.status(), StatusCode::SERVICE_UNAVAILABLE);

    // No auth, task, management, or search route exists while recovery waits.
    let unavailable = server.get("/api/v1/tasks").await;
    assert_eq!(
        unavailable.status(),
        StatusCode::NOT_FOUND,
        "non-health ingress must not be routed during recovery"
    );

    // The real worker reclaims the expired claim and adopts the persisted
    // remote id instead of submitting a replacement. The fake keeps the
    // conversion non-terminal, so the item keeps running and still blocks.
    let mut adopted = false;
    for _ in 0..900 {
        if docling
            .polls()
            .iter()
            .any(|id| id == &fixture.remote_task_id)
        {
            adopted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        adopted,
        "the recovery worker never adopted the persisted remote id"
    );
    assert_eq!(docling.submits(), 0, "adoption must never resubmit");
    assert_eq!(
        item_status(&pool, fixture.item_id).await,
        "running",
        "the adopted claim keeps running while the conversion is pending"
    );

    // Give the guard poll a full interval: waiting work must never clear it.
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        ledger_versions(&pool).await.last().copied(),
        Some(PRIOR_VERSION),
        "pending remote work must not clear the migration guard"
    );
    assert!(
        !outbox_table_exists(&pool).await,
        "the migration's outbox must not be touched before it runs"
    );
    let still_unready = server.get("/readyz").await;
    assert_eq!(still_unready.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Terminal completion: the worker commits the failure, the guards clear,
    // recovery writers quiesce, the unchanged migration applies once, the later
    // pending migrations apply in version order, and the normal application
    // rebinds the same port.
    docling.finish();
    let expected_versions = compiled_ledger_versions();
    let mut migrated = false;
    for _ in 0..1800 {
        // The health listener is stopped before the normal app rebinds, so the
        // port is briefly closed; that is not a readiness answer.
        if ledger_versions(&pool).await == expected_versions
            && let Ok(response) = server.try_get("/readyz").await
            && let Ok(body) = response.json::<Value>().await
            && body.get("upgrade_pending").is_none()
        {
            migrated = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        migrated,
        "the normal application never replaced the recovery health surface\n--- server log ---\n{}",
        server.log()
    );

    let versions = ledger_versions(&pool).await;
    assert_eq!(
        versions
            .iter()
            .filter(|version| **version == PINNED_RETIREMENT_VERSION)
            .count(),
        1,
        "the pinned retirement migration must apply exactly once"
    );
    assert_eq!(
        versions, expected_versions,
        "the ordinary migrator must apply every compiled migration in version order"
    );

    let job: (Uuid, String, String) = sqlx::query_as(
        "SELECT id, remote_task_id, status FROM context69.task_docling_remote_jobs \
         WHERE item_id = $1",
    )
    .bind(fixture.item_id)
    .fetch_one(&pool)
    .await
    .expect("read the persisted remote job");
    assert_eq!(job.0, fixture.remote_job_id);
    assert_eq!(
        job.1, fixture.remote_task_id,
        "remote identity is preserved"
    );
    assert_eq!(job.2, "cancelled", "the terminal-owner row is fenced");
    assert!(
        !docling.polls().is_empty()
            && docling
                .polls()
                .iter()
                .all(|id| id == &fixture.remote_task_id),
        "every poll targets the one persisted remote id: {:?}",
        docling.polls()
    );
    assert_eq!(docling.submits(), 0, "no replacement was ever submitted");
    assert!(docling.cancels() >= 1, "the orphaned remote is cancelled");

    let metadata: Value =
        sqlx::query_scalar("SELECT metadata_json FROM context69.documents WHERE id = $1")
            .bind(fixture.document_id)
            .fetch_one(&pool)
            .await
            .expect("read the retired document metadata");
    assert_eq!(metadata, json!({"topic": "markets"}));
    let record_hash: String =
        sqlx::query_scalar("SELECT record_hash FROM context69.documents WHERE id = $1")
            .bind(fixture.document_id)
            .fetch_one(&pool)
            .await
            .expect("read the preserved record hash");
    assert_eq!(record_hash, fixture.record_hash);
    let outbox: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.document_metadata_payload_sync WHERE document_id = $1",
    )
    .bind(fixture.document_id)
    .fetch_one(&pool)
    .await
    .expect("read the payload outbox");
    assert_eq!(outbox, 1, "the unchanged migration enqueues one mirror job");
    let task_status: String =
        sqlx::query_scalar("SELECT status FROM context69.tasks WHERE id = $1")
            .bind(fixture.task_id)
            .fetch_one(&pool)
            .await
            .expect("read the task status");
    assert_eq!(task_status, "failed", "the task reached a terminal state");

    server.stop().await;

    sqlx::query("DELETE FROM context69.document_metadata_payload_sync WHERE document_id = $1")
        .bind(fixture.document_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.document_results WHERE document_id = $1")
        .bind(fixture.document_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.documents WHERE id = $1")
        .bind(fixture.document_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.task_docling_remote_cleanup WHERE task_id = $1")
        .bind(fixture.task_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.task_docling_remote_jobs WHERE task_id = $1")
        .bind(fixture.task_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.task_attempts WHERE task_id = $1")
        .bind(fixture.task_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(fixture.task_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(fixture.task_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(fixture.user_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(fixture.library_file_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.library_storage_objects WHERE id = $1")
        .bind(fixture.storage_object_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.docling_settings WHERE singleton")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(fixture.group_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(fixture.user_id)
        .execute(&pool)
        .await
        .ok();
    let _ = tokio::fs::remove_dir_all(&storage_root).await;
    docling.stop().await;
}
