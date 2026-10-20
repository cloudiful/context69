use std::{env, fs, future::Future, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use context69::{
    api::{self, openapi_document},
    config::Config,
    db::{Database, UpgradeGuardCounts},
    mcp,
    services::{
        app::{Context69App, UpgradeRecoveryApp},
        maintenance,
        scheduler::{
            ManualRunResult, SCHEDULER_EXECUTION_LEASE_PREFIX, SCHEDULER_VALKEY_KEY_PREFIX,
            build_valkey_execution_guard, run_manual_sync_guarded, startup_execution_slot_at,
        },
    },
};
use scheduler::{
    CoordinatedLeaseConfig, ExecutionSlot, GuardedRunResult, GuardedRunner, InMemoryStateStore,
    Job, OverlapPolicy, Schedule, Scheduler, SchedulerConfig, Task, TaskContext,
    ValkeyCoordinatedStateStore, ValkeyLeaseConfig,
};
use tokio::{signal, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    opendal::install_default();
    let mode = env::args().nth(1).unwrap_or_else(|| "serve".to_string());
    if mode == "export-openapi" {
        export_openapi(env::args().nth(2)).await?;
        return Ok(());
    }
    // The audited artifact maintenance tool ships in this binary as well. It
    // parses arguments and exits on its own, before any configuration load,
    // database admission, migration, worker or signal handler starts.
    if mode == "task-artifacts" {
        return run_task_artifacts(env::args().skip(2)).await;
    }
    // Before `Context69App::new`, and deliberately not behind it: a rewrap is a
    // maintenance run over the application database, so it must not start
    // Valkey, Qdrant, the scheduler, the API, or the MCP server, and it must not
    // mint or resolve any runtime credential to do it.
    if mode == "rewrap-secrets" {
        return maintenance::rewrap_secrets().await;
    }

    let config = Config::load()?;
    let db = Database::connect(&config.app_db.url).await?;
    let pending_upgrade = db.pending_upgrade().is_some();
    if mode == "serve" && pending_upgrade {
        // One installed signal source spans the wait, quiescence, migration,
        // rebuild, and the ordinary server, so a stop is never missed and no
        // competing handler is registered.
        return match serve_upgrade_recovery(config, db, shutdown_signal()).await? {
            Some((app, signal)) => serve_with_signal(Arc::new(app), signal).await,
            None => Ok(()),
        };
    }
    let app = Arc::new(Context69App::with_database(config.clone(), db).await?);

    match mode.as_str() {
        "mcp-stdio" => {
            if !config.mcp.enabled {
                return Err(anyhow::anyhow!("mcp is disabled in config"));
            }
            mcp::run_stdio(app).await?;
        }
        "sync-once" => {
            match run_manual_sync_guarded(app.clone(), "sync-all", async move || {
                app.sync.sync_all("cli").await
            })
            .await?
            {
                ManualRunResult::Completed(()) => {}
                ManualRunResult::Contended => {
                    info!(
                        "manual sync skipped because another instance already holds the execution lease"
                    );
                }
            }
        }
        "migrate-library-storage" => {
            let dry_run = env::args().any(|arg| arg == "--dry-run");
            let summary = app
                .library
                .migrate_local_storage_to_active_backend(dry_run)
                .await?;
            info!(
                dry_run,
                scanned = summary.scanned,
                migrated = summary.migrated,
                already_migrated = summary.already_migrated,
                missing = summary.missing,
                invalid = summary.invalid,
                "library storage migration finished"
            );
        }
        "serve" => serve(app).await?,
        other => {
            return Err(anyhow::anyhow!(
                "unsupported mode {other}; {SUPPORTED_MODES}"
            ));
        }
    }

    Ok(())
}

/// Modes this binary accepts after the program name.
const SUPPORTED_MODES: &str = "expected serve, sync-once, mcp-stdio, rewrap-secrets, migrate-library-storage, task-artifacts, or export-openapi";

/// The one shared task-artifact maintenance entry, also used by the standalone
/// `task_artifact_backfill` binary.
async fn run_task_artifacts<I, S>(raw: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    context69::services::library::artifact_backfill::run_cli(raw).await
}

async fn export_openapi(output_path: Option<String>) -> Result<()> {
    let output_path = output_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("frontend/openapi/context69.openapi.json"));

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let document = serde_json::to_string_pretty(&openapi_document())?;
    fs::write(&output_path, document)?;
    info!(path = %output_path.display(), "exported openapi document");
    Ok(())
}

async fn serve(app: Arc<Context69App>) -> Result<()> {
    serve_with_signal(app, shutdown_signal()).await
}

async fn serve_with_signal(app: Arc<Context69App>, mut signal: ShutdownSignal) -> Result<()> {
    let api_bind_addr = app.config.api.bind_addr.clone();

    if app.config.scheduler.run_on_start && app.sync.runtime_configured() {
        info!("running startup sync");
        match await_startup_sync(run_startup_sync(app.clone()), &mut signal).await? {
            StartupSyncOutcome::Completed(Ok(())) => {}
            StartupSyncOutcome::Completed(Err(error)) => {
                warn!(error = %error, "startup sync failed; continuing to serve api");
            }
            StartupSyncOutcome::Stopped => {
                // Stop signal during startup sync: take the same bounded stop
                // before any scheduler, HTTP server, or later writer starts.
                warn!("stop signal observed during startup sync; stopping without serving");
                let mut steps = LiveShutdownSteps {
                    app: app.as_ref(),
                    server_shutdown: CancellationToken::new(),
                    background: OwnedBackground::empty(),
                };
                run_bounded_stop_without_server(&mut steps, SHUTDOWN_BUDGET).await;
                return Ok(());
            }
        }
    } else if app.config.scheduler.run_on_start {
        info!("startup sync skipped because sync runtime is not configured");
    }

    let scheduler_task = if app.sync.runtime_configured() {
        Some(tokio::spawn(run_scheduler(app.clone())))
    } else {
        info!("scheduler disabled until runtime settings are configured and the service restarts");
        None
    };
    let cleanup_task = tokio::spawn(run_rerank_cache_cleanup(app.clone()));
    let mcp_task = if app.config.mcp.enabled {
        Some(tokio::spawn({
            let app = app.clone();
            async move {
                if let Err(error) = mcp::run_http(app).await {
                    error!(error = %error, "mcp http server failed");
                }
            }
        }))
    } else {
        None
    };
    let router = api::router(app.clone()).await?;
    let server_config = server::ServerConfig::new()
        .with_listen_addr(api_bind_addr)
        .build()?;
    let bound = server::axum::Server::new(server_config, router).bind()?;
    info!(addrs = ?bound.addrs(), "http api listening");

    let server_shutdown = CancellationToken::new();
    let http_shutdown = server_shutdown.clone();
    let mut server_task = ServerTask::new(tokio::spawn(async move {
        bound
            .run_with_graceful_shutdown(async move { http_shutdown.cancelled().await })
            .await
            .map_err(anyhow::Error::from)
    }));
    let mut steps = LiveShutdownSteps {
        app: app.as_ref(),
        server_shutdown,
        background: OwnedBackground {
            scheduler_task,
            cleanup_task: Some(cleanup_task),
            mcp_task,
        },
    };

    match wait_for_shutdown_trigger(signal, &mut server_task).await {
        ShutdownTrigger::Signal(Ok(())) => {
            run_bounded_shutdown(&mut steps, &mut server_task, SHUTDOWN_BUDGET).await
        }
        ShutdownTrigger::Signal(Err(error)) => {
            error!(%error, "failed to install shutdown signal handlers");
            run_bounded_shutdown(&mut steps, &mut server_task, SHUTDOWN_BUDGET)
                .await
                .ok();
            Err(error.into())
        }
        // The server outcome was recorded when the trigger fired; cleanup and
        // the returned value both come from that one recorded result.
        ShutdownTrigger::ServerFinished => {
            run_bounded_shutdown(&mut steps, &mut server_task, SHUTDOWN_BUDGET).await
        }
    }
}

#[derive(Debug)]
enum StartupSyncOutcome {
    Completed(Result<()>),
    /// The installed stop source fired while the sync was still running.
    Stopped,
}

/// Races the startup sync against the already-installed stop source.
///
/// The sync future is dropped when a stop wins, so no source batch continues
/// past the coordinator and no scheduler or server starts afterwards.
async fn await_startup_sync<F>(
    sync: F,
    signal: &mut ShutdownSignal,
) -> std::io::Result<StartupSyncOutcome>
where
    F: Future<Output = Result<()>>,
{
    tokio::select! {
        result = sync => Ok(StartupSyncOutcome::Completed(result)),
        signal_result = &mut *signal => {
            signal_result?;
            Ok(StartupSyncOutcome::Stopped)
        }
    }
}

/// The one budget shared by natural item completion and the HTTP drain. It is
/// measured from the observed stop signal, so the two waits cannot add up to ten
/// seconds.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// How often the recovery profile re-reads the pinned migration guards.
const UPGRADE_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, serde::Serialize)]
struct UpgradeRecoveryStatus {
    status: &'static str,
    upgrade_pending: bool,
    affected_documents: i64,
    blocking_task_items: i64,
    blocking_remote_jobs: i64,
    elapsed_seconds: u64,
}

impl UpgradeRecoveryStatus {
    fn pending(guards: UpgradeGuardCounts, elapsed: Duration) -> Self {
        Self {
            status: "degraded",
            upgrade_pending: true,
            affected_documents: guards.affected_documents,
            blocking_task_items: guards.blocking_task_items,
            blocking_remote_jobs: guards.blocking_remote_jobs,
            elapsed_seconds: elapsed.as_secs(),
        }
    }
}

#[derive(Clone)]
struct UpgradeHealthState(std::sync::Arc<std::sync::RwLock<UpgradeRecoveryStatus>>);

/// The temporary health-only surface. No auth, diagnostics, management, task,
/// or search route is reachable while the pinned migration is pending.
fn upgrade_health_router(state: UpgradeHealthState) -> Router {
    Router::new()
        .route(
            "/livez",
            get(|| async { Json(serde_json::json!({"status": "ok"})) }),
        )
        .route("/readyz", get(upgrade_readiness))
        .route("/healthz", get(upgrade_readiness))
        .with_state(state)
}

async fn upgrade_readiness(State(state): State<UpgradeHealthState>) -> impl IntoResponse {
    let status = state
        .0
        .read()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone();
    (StatusCode::SERVICE_UNAVAILABLE, Json(status))
}

/// Serve only liveness and explicit unready health while persisted work
/// finishes and the pinned retirement migration is still pending.
///
/// Returns the normal application rebuilt on the same configuration together
/// with the still-live signal source, or `None` when a stop signal ended the
/// wait or interrupted the rebuild.
async fn serve_upgrade_recovery(
    config: Config,
    db: Database,
    mut signal: ShutdownSignal,
) -> Result<Option<(Context69App, ShutdownSignal)>> {
    let mut db = db;
    let mut round = 0_u32;
    loop {
        round += 1;
        if wait_for_the_pinned_migration(&config, db, &mut signal).await?
            == RecoveryOutcome::Stopped
        {
            return Ok(None);
        }
        let built = await_or_signal(Context69App::new(config.clone()), &mut signal).await;
        let error = match built {
            Ok(Some(app)) => {
                info!(round, "the normal application replaced upgrade recovery");
                return Ok(Some((app, signal)));
            }
            Ok(None) => {
                // The construction future was stopped before it returned, so no
                // ordinary writer was started after the migration attempt and
                // the process exits within the observed stop deadline.
                warn!(
                    round,
                    "stop signal observed during migration or normal reconstruction; \
                     exiting without starting ordinary writers"
                );
                return Ok(None);
            }
            Err(error) => error,
        };
        // A writer that outlived quiescence can re-raise the migration guard.
        // That is not a migration fault: admission is re-read, and a still
        // pending pinned migration rebuilds recovery instead of running the
        // migration beside its writers.
        let next = match await_or_signal(Database::connect(&config.app_db.url), &mut signal).await {
            Ok(Some(next)) => next,
            Ok(None) => return Ok(None),
            Err(_) => {
                return Err(
                    error.context("the pinned migration did not apply after recovery quiesced")
                );
            }
        };
        if next.pending_upgrade().is_none() {
            return Err(error.context("the pinned migration did not apply after recovery quiesced"));
        }
        warn!(
            %error,
            round,
            "the migration guard was raised again after recovery quiesced; rebuilding upgrade recovery"
        );
        if await_or_signal(
            async {
                tokio::time::sleep(UPGRADE_POLL_INTERVAL).await;
                Ok(())
            },
            &mut signal,
        )
        .await?
        .is_none()
        {
            return Ok(None);
        }
        db = next;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryOutcome {
    /// A stop signal ended the wait; nothing was migrated.
    Stopped,
    /// The pinned migration's guards cleared and recovery quiesced.
    Quiesced,
}

/// Serve the recovery health surface until the guards clear or a signal stops
/// the process, then stop every recovery writer before returning.
///
/// The installed signal source stays polled across guard-clearing, the health
/// listener join, and the writer join, so a stop during those transitions takes
/// the same bounded exit instead of waiting outside the coordinator.
async fn wait_for_the_pinned_migration(
    config: &Config,
    db: Database,
    signal: &mut ShutdownSignal,
) -> Result<RecoveryOutcome> {
    let mut recovery = UpgradeRecoveryApp::with_database(config.clone(), db).await?;
    let bind_addr = recovery.config.api.bind_addr.clone();
    let started = tokio::time::Instant::now();
    let guards = recovery.guard_counts().await?;
    let status = std::sync::Arc::new(std::sync::RwLock::new(UpgradeRecoveryStatus::pending(
        guards,
        Duration::ZERO,
    )));
    let router = upgrade_health_router(UpgradeHealthState(status.clone()));
    let server_config = server::ServerConfig::new()
        .with_listen_addr(bind_addr.clone())
        .build()?;
    let bound = server::axum::Server::new(server_config, router).bind()?;
    info!(
        addrs = ?bound.addrs(),
        "upgrade recovery is serving health only until persisted work finishes"
    );

    let listener_shutdown = CancellationToken::new();
    let listener_token = listener_shutdown.clone();
    let mut server_task = ServerTask::new(tokio::spawn(async move {
        bound
            .run_with_graceful_shutdown(async move { listener_token.cancelled().await })
            .await
            .map_err(anyhow::Error::from)
    }));

    loop {
        tokio::select! {
            signal_result = &mut *signal => {
                signal_result?;
                shutdown_upgrade_recovery(&mut recovery, &mut server_task, &listener_shutdown).await;
                return Ok(RecoveryOutcome::Stopped);
            }
            _ = tokio::time::sleep(UPGRADE_POLL_INTERVAL) => {}
        }
        let guards = recovery.guard_counts().await?;
        let elapsed = started.elapsed();
        *status.write().unwrap_or_else(|poison| poison.into_inner()) =
            UpgradeRecoveryStatus::pending(guards, elapsed);
        info!(
            affected_documents = guards.affected_documents,
            blocking_task_items = guards.blocking_task_items,
            blocking_remote_jobs = guards.blocking_remote_jobs,
            elapsed_secs = elapsed.as_secs(),
            "upgrade recovery waiting for persisted ingestion work"
        );
        if !guards.blocks_retirement() {
            break;
        }
    }

    match join_quiesced(&mut recovery, &mut server_task, &listener_shutdown, signal).await? {
        Quiescence::Joined => {}
        Quiescence::Signalled => {
            shutdown_upgrade_recovery(&mut recovery, &mut server_task, &listener_shutdown).await;
            return Ok(RecoveryOutcome::Stopped);
        }
    }
    drop(recovery);
    info!("upgrade recovery writers stopped");
    Ok(RecoveryOutcome::Quiesced)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quiescence {
    /// The listener and every recovery writer finished joining.
    Joined,
    /// A stop signal arrived while the transition was still joining.
    Signalled,
}

/// Cancel the health listener and join it plus every recovery writer, while the
/// already-installed signal source stays observed.
async fn join_quiesced<S: RecoveryStop>(
    recovery: &mut S,
    server: &mut ServerTask,
    listener_shutdown: &CancellationToken,
    signal: &mut ShutdownSignal,
) -> Result<Quiescence> {
    let joined = tokio::select! {
        _ = async {
            listener_shutdown.cancel();
            if let Err(error) = server.finish().await {
                warn!(%error, "upgrade recovery health listener did not stop cleanly");
            }
            recovery.stop_writers().await;
        } => true,
        signal_result = &mut *signal => {
            signal_result?;
            false
        }
    };
    Ok(if joined {
        Quiescence::Joined
    } else {
        Quiescence::Signalled
    })
}

/// Poll a transition future while the installed signal source stays observed.
///
/// `Ok(None)` means a stop arrived first; the future was dropped, so a database
/// transaction inside it follows ordinary SQLx rollback semantics and no later
/// startup step runs.
async fn await_or_signal<F, T>(future: F, signal: &mut ShutdownSignal) -> Result<Option<T>>
where
    F: Future<Output = Result<T>>,
{
    tokio::select! {
        result = future => result.map(Some),
        signal_result = &mut *signal => {
            signal_result?;
            Ok(None)
        }
    }
}

/// Bounded stop while recovery is still waiting: no migration runs, and no
/// task, status, lease, or remote identity is mutated.
async fn shutdown_upgrade_recovery(
    recovery: &mut UpgradeRecoveryApp,
    server: &mut ServerTask,
    listener_shutdown: &CancellationToken,
) {
    listener_shutdown.cancel();
    shutdown_waiting_recovery(recovery, server, SHUTDOWN_BUDGET).await;
}

/// One bounded stop of a recovery profile that is still waiting.
///
/// Nothing is migrated here: the listener is stopped, the writers are cancelled
/// and joined, and unfinished work keeps its persisted identity, progress, and
/// leases.
async fn shutdown_waiting_recovery<S: RecoveryStop>(
    recovery: &mut S,
    server: &mut ServerTask,
    budget: Duration,
) {
    recovery.cancel_writers();
    let deadline = tokio::time::Instant::now() + budget;
    let sequence = async {
        let _ = server.finish().await;
        recovery.stop_writers().await;
    };
    if tokio::time::timeout_at(deadline, sequence).await.is_err() {
        warn!(
            budget_secs = budget.as_secs(),
            "upgrade recovery stop budget expired; exiting without cancelling or releasing unfinished work"
        );
        if !server.is_finished() {
            server.abort();
        }
        let _ = server.finish().await;
    }
}

/// The recovery profile's stop surface, so the bounded stop is testable without
/// a database or a real supervisor.
trait RecoveryStop {
    fn cancel_writers(&self);
    fn stop_writers(&mut self) -> impl Future<Output = ()>;
}

impl RecoveryStop for UpgradeRecoveryApp {
    fn cancel_writers(&self) {
        UpgradeRecoveryApp::cancel_writers(self);
    }

    async fn stop_writers(&mut self) {
        UpgradeRecoveryApp::stop_writers(self).await;
    }
}

enum ShutdownTrigger {
    Signal(std::io::Result<()>),
    ServerFinished,
}

/// One owned HTTP server handle.
///
/// The outcome is recorded the first time the handle is polled, so trigger
/// detection and cleanup never poll a completed `JoinHandle` twice.
struct ServerTask {
    handle: Option<JoinHandle<Result<()>>>,
    completed: Option<Result<()>>,
}

impl ServerTask {
    fn new(handle: JoinHandle<Result<()>>) -> Self {
        Self {
            handle: Some(handle),
            completed: None,
        }
    }

    /// Polls the live handle by mutable reference.
    ///
    /// Awaiting `&mut JoinHandle` is cancel-safe: dropping this future neither
    /// cancels the server nor loses the handle. The handle is released only
    /// after its completion has been observed.
    async fn run(&mut self) {
        if self.completed.is_some() || self.handle.is_none() {
            return;
        }
        let handle = self.handle.as_mut().expect("checked above");
        let outcome = server_result((&mut *handle).await);
        self.completed = Some(outcome);
        self.handle = None;
    }

    /// Returns the recorded outcome, or awaits the still-owned handle.
    ///
    /// Cancelling this future keeps the handle owned, so a bounded stop can
    /// resume the same drain instead of detaching the server.
    async fn finish(&mut self) -> Result<()> {
        if let Some(outcome) = self.completed.take() {
            return outcome;
        }
        let Some(handle) = self.handle.as_mut() else {
            return Ok(());
        };
        let outcome = server_result((&mut *handle).await);
        self.handle = None;
        outcome
    }

    fn is_finished(&self) -> bool {
        self.completed.is_some()
            || self
                .handle
                .as_ref()
                .is_none_or(|handle| handle.is_finished())
    }

    /// Requests termination but keeps the handle until it is joined, so an
    /// aborted server is never reported as an observed completion.
    fn abort(&mut self) {
        if let Some(handle) = self.handle.as_ref() {
            handle.abort();
        }
    }
}

async fn wait_for_shutdown_trigger<F>(signal: F, server: &mut ServerTask) -> ShutdownTrigger
where
    F: Future<Output = std::io::Result<()>>,
{
    tokio::select! {
        signal = signal => ShutdownTrigger::Signal(signal),
        _ = server.run() => ShutdownTrigger::ServerFinished,
    }
}

fn server_result(result: std::result::Result<Result<()>, tokio::task::JoinError>) -> Result<()> {
    match result {
        Ok(result) => result,
        Err(error) => Err(anyhow::Error::from(error)),
    }
}

/// The same bounded stop for a phase that owns no HTTP server yet.
async fn run_bounded_stop_without_server<S: ShutdownSteps>(steps: &mut S, budget: Duration) {
    let deadline = tokio::time::Instant::now() + budget;
    let sequence = async {
        steps.begin().await;
        steps.stop_background().await;
        steps.wait_for_idle().await;
    };
    if tokio::time::timeout_at(deadline, sequence).await.is_err() {
        warn!(
            budget_secs = budget.as_secs(),
            "startup stop budget expired; exiting without cancelling or releasing unfinished work"
        );
        steps.abort();
    }
}

/// Ordered steps of one bounded stop.
trait ShutdownSteps {
    fn begin(&mut self) -> impl Future<Output = ()>;
    fn stop_background(&mut self) -> impl Future<Output = ()>;
    fn wait_for_idle(&mut self) -> impl Future<Output = ()>;
    fn abort(&mut self);
}

/// Stop claiming, stop the owned background sources, wait for already-dispatched
/// items, then let the HTTP drain finish — all under one deadline.
///
/// On deadline expiry the process keeps the unfinished work persisted: no item,
/// task status, or lease is mutated, and no remote work is cancelled. The
/// existing expiry and recovery path reclaims it after exit. A server that had
/// already finished keeps its recorded outcome as the returned value.
async fn run_bounded_shutdown<S: ShutdownSteps>(
    steps: &mut S,
    server: &mut ServerTask,
    budget: Duration,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + budget;
    let sequence = async {
        steps.begin().await;
        steps.stop_background().await;
        steps.wait_for_idle().await;
        server.finish().await
    };
    match tokio::time::timeout_at(deadline, sequence).await {
        Ok(result) => result,
        Err(_) => {
            warn!(
                budget_secs = budget.as_secs(),
                "shutdown budget expired; exiting without cancelling or releasing unfinished work"
            );
            steps.abort();
            if server.is_finished() {
                return server.finish().await;
            }
            server.abort();
            let _ = server.finish().await;
            Ok(())
        }
    }
}

struct OwnedBackground {
    scheduler_task: Option<JoinHandle<Result<()>>>,
    cleanup_task: Option<JoinHandle<Result<()>>>,
    mcp_task: Option<JoinHandle<()>>,
}

impl OwnedBackground {
    fn empty() -> Self {
        Self {
            scheduler_task: None,
            cleanup_task: None,
            mcp_task: None,
        }
    }

    async fn stop(&mut self) {
        if let Some(task) = self.scheduler_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.cleanup_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.mcp_task.take() {
            task.abort();
            let _ = task.await;
        }
    }

    fn abort(&mut self) {
        if let Some(task) = self.scheduler_task.take() {
            task.abort();
        }
        if let Some(task) = self.cleanup_task.take() {
            task.abort();
        }
        if let Some(task) = self.mcp_task.take() {
            task.abort();
        }
    }
}

struct LiveShutdownSteps<'a> {
    app: &'a Context69App,
    server_shutdown: CancellationToken,
    background: OwnedBackground,
}

impl ShutdownSteps for LiveShutdownSteps<'_> {
    async fn begin(&mut self) {
        self.app.tasks.begin_shutdown();
        self.server_shutdown.cancel();
    }

    async fn stop_background(&mut self) {
        self.background.stop().await;
    }

    async fn wait_for_idle(&mut self) {
        self.app.tasks.wait_for_dispatch_stop().await;
        self.app.tasks.wait_until_idle().await;
    }

    fn abort(&mut self) {
        self.background.abort();
    }
}

async fn run_startup_sync(app: Arc<Context69App>) -> Result<()> {
    let Some(valkey_url) = app.config.scheduler.valkey_url.as_deref() else {
        return app.sync.sync_all("startup").await;
    };

    let guard = build_valkey_execution_guard(
        valkey_url,
        ValkeyLeaseConfig {
            ttl: app.config.scheduler.execution_guard_ttl,
            renew_interval: app.config.scheduler.execution_guard_renew_interval,
        },
    )
    .await?;
    let runner = GuardedRunner::new(guard);
    let guarded = runner
        .run(
            ExecutionSlot::new(
                app.config.scheduler.job_id.clone(),
                startup_execution_slot_at(),
            ),
            async move || app.sync.sync_all("startup").await,
        )
        .await?;

    match guarded {
        GuardedRunResult::Completed(result) => result,
        GuardedRunResult::Contended => {
            info!(
                "startup sync skipped because another instance already holds the execution lease"
            );
            Ok(())
        }
    }
}

async fn run_scheduler(app: Arc<Context69App>) -> Result<()> {
    let valkey_url = app.config.scheduler.valkey_url.clone();
    let execution_guard_ttl = app.config.scheduler.execution_guard_ttl;
    let execution_guard_renew_interval = app.config.scheduler.execution_guard_renew_interval;

    let job = Job::new(
        app.config.scheduler.job_id.clone(),
        Schedule::Interval(app.config.scheduler.interval),
        app,
        Task::from_async(async |context: TaskContext<Arc<Context69App>>| {
            context
                .deps
                .sync
                .sync_all("scheduled")
                .await
                .map_err(|error| error.to_string())
        }),
    )
    .with_overlap_policy(OverlapPolicy::Forbid);

    let report = if let Some(valkey_url) = valkey_url.as_deref() {
        let store = ValkeyCoordinatedStateStore::with_prefixes(
            valkey_url,
            SCHEDULER_VALKEY_KEY_PREFIX,
            SCHEDULER_EXECUTION_LEASE_PREFIX,
        )
        .await?;
        Scheduler::with_coordinated_state_store(
            SchedulerConfig::default(),
            store,
            CoordinatedLeaseConfig {
                ttl: execution_guard_ttl,
                renew_interval: execution_guard_renew_interval,
            },
        )
        .run(job)
        .await?
    } else {
        Scheduler::new(SchedulerConfig::default(), InMemoryStateStore::new())
            .run(job)
            .await?
    };
    info!(
        runs = report.history.len(),
        next_run_at = ?report.state.next_run_at,
        "scheduler loop exited"
    );
    Ok(())
}

async fn run_rerank_cache_cleanup(app: Arc<Context69App>) -> Result<()> {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 60 * 60));
    loop {
        interval.tick().await;
        match app.db.delete_expired_rerank_item_scores(30).await {
            Ok(deleted) => info!(deleted, "expired rerank item scores pruned"),
            Err(error) => warn!(error = %error, "failed to prune expired rerank item scores"),
        }
    }
}

#[cfg(unix)]
struct UnixShutdownSignals {
    sigterm: signal::unix::Signal,
    sigint: signal::unix::Signal,
}

#[cfg(unix)]
impl UnixShutdownSignals {
    fn install() -> std::io::Result<Self> {
        Ok(Self {
            sigterm: signal::unix::signal(signal::unix::SignalKind::terminate())?,
            sigint: signal::unix::signal(signal::unix::SignalKind::interrupt())?,
        })
    }

    async fn recv(&mut self) {
        tokio::select! {
            _ = self.sigterm.recv() => {}
            _ = self.sigint.recv() => {}
        }
    }
}

/// The installed stop source, kept alive across recovery transitions and the
/// ordinary server so a signal is never observed by two competing streams.
type ShutdownSignal = std::pin::Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;

fn shutdown_signal() -> ShutdownSignal {
    Box::pin(async {
        #[cfg(unix)]
        {
            UnixShutdownSignals::install()?.recv().await;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            signal::ctrl_c().await
        }
    })
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("context69=info,sqlx=warn"));

    fmt().with_env_filter(env_filter).with_target(false).init();
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::{Duration, Instant};

    use super::{
        Quiescence, RecoveryStop, ServerTask, ShutdownSignal, ShutdownSteps, ShutdownTrigger,
        StartupSyncOutcome, await_or_signal, await_startup_sync, join_quiesced,
        run_bounded_shutdown, run_bounded_stop_without_server, shutdown_waiting_recovery,
        wait_for_shutdown_trigger,
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum IdleMode {
        Immediate,
        Hold,
    }

    struct FakeSteps {
        order: Arc<Mutex<Vec<&'static str>>>,
        idle: IdleMode,
        aborted: Arc<AtomicBool>,
    }

    impl FakeSteps {
        fn new(order: Arc<Mutex<Vec<&'static str>>>, idle: IdleMode) -> Self {
            Self {
                order,
                idle,
                aborted: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl ShutdownSteps for FakeSteps {
        async fn begin(&mut self) {
            self.order.lock().expect("order").push("begin");
        }

        async fn stop_background(&mut self) {
            self.order.lock().expect("order").push("stop-background");
        }

        async fn wait_for_idle(&mut self) {
            self.order.lock().expect("order").push("wait-for-idle");
            if self.idle == IdleMode::Hold {
                std::future::pending::<()>().await;
            }
        }

        fn abort(&mut self) {
            self.aborted.store(true, Ordering::SeqCst);
        }
    }

    fn pending_server() -> ServerTask {
        ServerTask::new(tokio::spawn(std::future::pending::<super::Result<()>>()))
    }

    fn immediate_signal() -> ShutdownSignal {
        Box::pin(async { Ok(()) })
    }

    #[tokio::test]
    async fn resolved_signal_selects_the_signal_trigger() {
        let mut server = pending_server();

        let trigger = wait_for_shutdown_trigger(async { Ok(()) }, &mut server).await;

        assert!(matches!(trigger, ShutdownTrigger::Signal(Ok(()))));
        server.abort();
    }

    #[tokio::test]
    async fn delayed_signal_preserves_the_pending_server_handle() {
        let started = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let task_started = started.clone();
        let task_completed = completed.clone();
        let mut server = ServerTask::new(tokio::spawn(async move {
            task_started.store(true, Ordering::SeqCst);
            let _ = released.await;
            task_completed.store(true, Ordering::SeqCst);
            Ok(())
        }));
        let mut signal: ShutdownSignal = Box::pin(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(())
        });

        // The trigger polls the real server future first; the later signal wins.
        let trigger = wait_for_shutdown_trigger(&mut signal, &mut server).await;
        assert!(matches!(trigger, ShutdownTrigger::Signal(Ok(()))));
        assert!(started.load(Ordering::SeqCst), "the real server future ran");
        assert!(
            !completed.load(Ordering::SeqCst),
            "the real server future is still pending after the signal"
        );
        assert!(
            !server.is_finished(),
            "a pending server must not look finished after cancellation"
        );

        // Releasing the real future must still be observed by the owner: the
        // handle was retained, not detached.
        release.send(()).expect("release the server future");
        let outcome = server.finish().await;
        assert!(outcome.is_ok(), "the retained server outcome is observed");
        assert!(
            completed.load(Ordering::SeqCst),
            "the real server future reached natural completion"
        );
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn held_drain_is_aborted_and_reaped_only_after_the_deadline() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut steps = FakeSteps::new(order, IdleMode::Immediate);
        let aborted = steps.aborted.clone();
        let dropped = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let task_dropped = dropped.clone();
        let task_completed = completed.clone();
        let mut server = ServerTask::new(tokio::spawn(async move {
            let _drop_guard = DropFlag(task_dropped);
            std::future::pending::<()>().await;
            task_completed.store(true, Ordering::SeqCst);
            Ok(())
        }));

        let started = Instant::now();
        let result = run_bounded_shutdown(&mut steps, &mut server, Duration::from_millis(50)).await;

        assert!(result.is_ok());
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(aborted.load(Ordering::SeqCst), "owned sources were stopped");
        assert!(
            !completed.load(Ordering::SeqCst),
            "a held drain never reports completion"
        );
        assert!(
            dropped.load(Ordering::SeqCst),
            "the deadline aborts and reaps the real drain future"
        );
        assert!(
            server.is_finished(),
            "the aborted handle is joined, not described as complete while detached"
        );
    }

    #[tokio::test]
    async fn startup_sync_held_past_a_signal_takes_the_bounded_stop() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut steps = FakeSteps::new(order.clone(), IdleMode::Hold);
        let aborted = steps.aborted.clone();
        let sync_ran = Arc::new(AtomicBool::new(false));
        let sync_ran_by_future = sync_ran.clone();
        let mut signal: ShutdownSignal = Box::pin(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(())
        });

        let outcome = await_startup_sync(
            async move {
                sync_ran_by_future.store(true, Ordering::SeqCst);
                std::future::pending::<()>().await;
                Ok(())
            },
            &mut signal,
        )
        .await
        .expect("the installed signal source resolves cleanly");

        assert!(matches!(outcome, StartupSyncOutcome::Stopped));
        assert!(sync_ran.load(Ordering::SeqCst), "the real sync future ran");

        let started = Instant::now();
        run_bounded_stop_without_server(&mut steps, Duration::from_millis(50)).await;

        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(
            aborted.load(Ordering::SeqCst),
            "the startup stop aborts owned sources at the deadline"
        );
        assert_eq!(
            *order.lock().expect("order"),
            vec!["begin", "stop-background", "wait-for-idle"]
        );
    }

    #[tokio::test]
    async fn startup_sync_completion_before_a_signal_keeps_its_result() {
        let mut signal: ShutdownSignal = Box::pin(std::future::pending());
        let outcome = await_startup_sync(async { Ok(()) }, &mut signal)
            .await
            .expect("no signal fired");
        assert!(matches!(outcome, StartupSyncOutcome::Completed(Ok(()))));
    }

    #[tokio::test]
    async fn server_exit_before_a_signal_selects_the_server_trigger() {
        let mut server = ServerTask::new(tokio::spawn(async {
            Err(anyhow::anyhow!("listen failed"))
        }));

        let trigger =
            wait_for_shutdown_trigger(std::future::pending::<std::io::Result<()>>(), &mut server)
                .await;

        assert!(matches!(trigger, ShutdownTrigger::ServerFinished));
        let outcome = server.finish().await;
        assert!(
            outcome
                .expect_err("the recorded server error is preserved")
                .to_string()
                .contains("listen failed")
        );
    }

    #[tokio::test]
    async fn composed_server_error_is_returned_after_cleanup() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut steps = FakeSteps::new(order.clone(), IdleMode::Immediate);
        let mut server = ServerTask::new(tokio::spawn(async {
            Err(anyhow::anyhow!("listen failed"))
        }));

        // The server finishes before any signal, exactly the path that used to
        // poll the consumed handle a second time during cleanup.
        let trigger =
            wait_for_shutdown_trigger(std::future::pending::<std::io::Result<()>>(), &mut server)
                .await;
        assert!(matches!(trigger, ShutdownTrigger::ServerFinished));

        let outcome = run_bounded_shutdown(&mut steps, &mut server, Duration::from_secs(1)).await;

        assert!(
            outcome
                .expect_err("cleanup preserves the original server result")
                .to_string()
                .contains("listen failed")
        );
        assert_eq!(
            *order.lock().expect("order"),
            vec!["begin", "stop-background", "wait-for-idle"]
        );
    }

    #[tokio::test]
    async fn composed_server_success_is_returned_after_cleanup() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut steps = FakeSteps::new(order.clone(), IdleMode::Immediate);
        let mut server = ServerTask::new(tokio::spawn(async { Ok(()) }));

        let trigger =
            wait_for_shutdown_trigger(std::future::pending::<std::io::Result<()>>(), &mut server)
                .await;
        assert!(matches!(trigger, ShutdownTrigger::ServerFinished));

        run_bounded_shutdown(&mut steps, &mut server, Duration::from_secs(1))
            .await
            .expect("cleanup returns the recorded success");
        assert_eq!(
            *order.lock().expect("order"),
            vec!["begin", "stop-background", "wait-for-idle"]
        );
    }

    #[tokio::test]
    async fn bounded_shutdown_stops_claims_then_background_then_idle_then_http() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut steps = FakeSteps::new(order.clone(), IdleMode::Immediate);
        let http_order = order.clone();
        let mut server = ServerTask::new(tokio::spawn(async move {
            http_order.lock().expect("order").push("http");
            Ok(())
        }));

        run_bounded_shutdown(&mut steps, &mut server, Duration::from_secs(1))
            .await
            .expect("bounded shutdown");

        assert_eq!(
            *order.lock().expect("order"),
            vec!["begin", "stop-background", "wait-for-idle", "http"]
        );
    }

    #[tokio::test]
    async fn held_worker_slot_is_bounded_by_the_shutdown_budget() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut steps = FakeSteps::new(order, IdleMode::Hold);
        let aborted = steps.aborted.clone();
        let mut server = pending_server();

        let started = Instant::now();
        let result = run_bounded_shutdown(&mut steps, &mut server, Duration::from_millis(50)).await;

        assert!(result.is_ok());
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(aborted.load(Ordering::SeqCst));
        assert!(server.is_finished());
    }

    #[tokio::test]
    async fn held_http_drain_shares_the_same_budget_after_items_are_idle() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut steps = FakeSteps::new(order.clone(), IdleMode::Immediate);
        let aborted = steps.aborted.clone();
        let mut server = pending_server();

        let started = Instant::now();
        let result = run_bounded_shutdown(&mut steps, &mut server, Duration::from_millis(50)).await;

        assert!(result.is_ok());
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert_eq!(
            *order.lock().expect("order"),
            vec!["begin", "stop-background", "wait-for-idle"]
        );
        assert!(aborted.load(Ordering::SeqCst));
        assert!(server.is_finished());
    }

    #[tokio::test]
    async fn server_failure_still_stops_owned_sources() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut steps = FakeSteps::new(order.clone(), IdleMode::Immediate);
        let mut server = ServerTask::new(tokio::spawn(async {
            Err(anyhow::anyhow!("listen failed"))
        }));

        let result = run_bounded_shutdown(&mut steps, &mut server, Duration::from_secs(1)).await;

        assert!(result.is_err());
        assert_eq!(
            *order.lock().expect("order"),
            vec!["begin", "stop-background", "wait-for-idle"]
        );
    }

    #[cfg(unix)]
    const SIGTERM_CHILD_ENV: &str = "CONTEXT69_SIGTERM_CHILD";

    #[cfg(unix)]
    #[test]
    fn sigterm_child_process_helper() {
        if std::env::var(SIGTERM_CHILD_ENV).as_deref() != Ok("1") {
            return;
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let mut signals = super::UnixShutdownSignals::install().expect("install handlers");
            use std::io::Write;
            println!("ready");
            std::io::stdout().flush().expect("flush ready line");
            signals.recv().await;
        });
        std::process::exit(0);
    }

    #[cfg(unix)]
    #[test]
    fn sigterm_reaches_a_child_process_coordinator() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let exe = std::env::current_exe().expect("current test binary");
        let mut child = match Command::new(exe)
            .args([
                "--exact",
                "tests::sigterm_child_process_helper",
                "--nocapture",
            ])
            .env(SIGTERM_CHILD_ENV, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                eprintln!("skipping SIGTERM delivery test: cannot spawn helper: {error}");
                return;
            }
        };

        let stdout = child.stdout.take().expect("child stdout");
        let ready = BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
            .any(|line| line.contains("ready"));
        if !ready {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child did not report signal-handler registration");
        }

        let kill = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
        match kill {
            Ok(status) if status.success() => {}
            other => {
                eprintln!("skipping SIGTERM delivery test: kill unavailable: {other:?}");
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().expect("wait for child") {
                assert!(
                    status.success(),
                    "child exited with {status:?} after SIGTERM"
                );
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child did not exit after SIGTERM delivery");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    const TASK_ARTIFACTS_CHILD_ENV: &str = "CONTEXT69_TASK_ARTIFACTS_CLI_CHILD";
    const TASK_ARTIFACTS_ARGS_ENV: &str = "CONTEXT69_TASK_ARTIFACTS_CLI_ARGS";

    #[test]
    fn task_artifacts_cli_child_helper() {
        if std::env::var(TASK_ARTIFACTS_CHILD_ENV).as_deref() != Ok("1") {
            return;
        }
        let raw = std::env::var(TASK_ARTIFACTS_ARGS_ENV).unwrap_or_default();
        let args: Vec<String> = if raw.is_empty() {
            Vec::new()
        } else {
            raw.split('\u{1f}').map(str::to_string).collect()
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("child runtime");
        if let Err(error) = runtime.block_on(super::run_task_artifacts(args)) {
            eprintln!("fatal: {error:#}");
            std::process::exit(1);
        }
    }

    /// Run the same entry `main` uses in a child process with an unusable
    /// database URL, so a successful or argument-only run proves no connection
    /// was attempted.
    fn run_task_artifacts_child(args: &[&str]) -> std::process::Output {
        use std::process::{Command, Stdio};

        let exe = std::env::current_exe().expect("current test binary");
        Command::new(exe)
            .args([
                "--exact",
                "tests::task_artifacts_cli_child_helper",
                "--nocapture",
            ])
            .env(TASK_ARTIFACTS_CHILD_ENV, "1")
            .env(TASK_ARTIFACTS_ARGS_ENV, args.join("\u{1f}"))
            .env(
                "DATABASE_URL",
                "postgres://invalid:invalid@127.0.0.1:1/none",
            )
            .current_dir(std::env::temp_dir())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn the task-artifacts child")
    }

    fn child_text(output: &std::process::Output) -> (String, String) {
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    #[test]
    fn task_artifacts_help_finishes_without_config_or_database() {
        let output = run_task_artifacts_child(&["--help"]);
        let (stdout, stderr) = child_text(&output);
        assert!(output.status.success(), "help failed: {stderr}");
        assert!(stdout.contains("Usage: context69 task-artifacts"));
        assert!(stdout.contains("--restore-inline"));
        assert!(
            !stdout.contains("connect"),
            "help must not connect: {stdout}"
        );
        assert!(
            !stderr.contains("postgres") && !stderr.contains("connect"),
            "help touched the database: {stderr}"
        );
    }

    #[test]
    fn task_artifacts_invalid_flags_fail_before_connecting() {
        for args in [
            vec!["--kind", "bogus"],
            vec!["--apply"],
            vec!["--restore-inline", "--apply"],
            vec!["--apply=false"],
            vec!["--apply=false", "--acknowledge-reader-upgrade=false"],
            vec!["--restore-inline=false"],
            vec![
                "--restore-inline=false",
                "--apply=false",
                "--acknowledge-inline-growth=false",
            ],
            vec!["--apply", "false"],
        ] {
            let output = run_task_artifacts_child(&args);
            let (stdout, stderr) = child_text(&output);
            assert!(
                !output.status.success(),
                "invalid flags succeeded for {args:?}: {stdout}"
            );
            assert!(
                !stdout.contains("connect") && !stderr.contains("connect"),
                "invalid flags reached the database for {args:?}: {stdout} {stderr}"
            );
            assert!(
                !stdout.contains("Usage:")
                    && !stdout.contains(
                        "{
"
                    ),
                "invalid flags produced normal output for {args:?}"
            );
        }
    }

    #[test]
    fn supported_modes_document_the_task_artifacts_command() {
        assert!(super::SUPPORTED_MODES.contains("task-artifacts"));
    }

    struct FakeRecovery {
        order: Arc<Mutex<Vec<&'static str>>>,
        stops: bool,
    }

    impl RecoveryStop for FakeRecovery {
        fn cancel_writers(&self) {
            self.order.lock().expect("order").push("cancel-writers");
        }

        async fn stop_writers(&mut self) {
            self.order.lock().expect("order").push("stop-writers");
            if !self.stops {
                std::future::pending::<()>().await;
            }
        }
    }

    #[tokio::test]
    async fn waiting_recovery_stop_cancels_then_joins_under_one_budget() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut recovery = FakeRecovery {
            order: order.clone(),
            stops: true,
        };
        // The real listener exits as soon as its cancellation token fires.
        let mut server = ServerTask::new(tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(())
        }));

        shutdown_waiting_recovery(&mut recovery, &mut server, Duration::from_secs(1)).await;

        assert_eq!(
            *order.lock().expect("order"),
            vec!["cancel-writers", "stop-writers"]
        );
        assert!(server.is_finished(), "the health listener must stop too");
    }

    #[tokio::test]
    async fn signal_during_the_quiescence_join_returns_to_bounded_stop() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut recovery = FakeRecovery {
            order: order.clone(),
            stops: false,
        };
        let mut server = ServerTask::new(tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(())
        }));
        let listener = tokio_util::sync::CancellationToken::new();
        let mut signal: ShutdownSignal = Box::pin(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(())
        });

        let started = Instant::now();
        let quiescence = join_quiesced(&mut recovery, &mut server, &listener, &mut signal)
            .await
            .expect("the installed signal source resolves cleanly");

        assert_eq!(quiescence, Quiescence::Signalled);
        assert!(started.elapsed() >= Duration::from_millis(20));
        assert_eq!(*order.lock().expect("order"), vec!["stop-writers"]);
        assert!(listener.is_cancelled());

        // The caller then takes the same bounded stop instead of waiting for
        // the held writer join.
        shutdown_waiting_recovery(&mut recovery, &mut server, Duration::from_millis(50)).await;
        assert_eq!(
            *order.lock().expect("order"),
            vec!["stop-writers", "cancel-writers", "stop-writers"]
        );
    }

    #[tokio::test]
    async fn signal_during_the_migration_future_returns_none_without_completing_it() {
        let started = Instant::now();
        let mut signal = immediate_signal();

        let result = await_or_signal(
            async {
                std::future::pending::<()>().await;
                Ok::<_, anyhow::Error>(())
            },
            &mut signal,
        )
        .await
        .expect("the signal resolves cleanly");

        assert!(
            result.is_none(),
            "a signal must not be treated as completion"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn waiting_recovery_stop_is_bounded_when_writers_do_not_finish() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut recovery = FakeRecovery {
            order: order.clone(),
            stops: false,
        };
        let mut server = ServerTask::new(tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(())
        }));

        let started = Instant::now();
        shutdown_waiting_recovery(&mut recovery, &mut server, Duration::from_millis(50)).await;

        assert!(started.elapsed() >= Duration::from_millis(50));
        assert_eq!(
            *order.lock().expect("order"),
            vec!["cancel-writers", "stop-writers"]
        );
        assert!(server.is_finished(), "the health listener must be stopped");
    }

    #[tokio::test]
    async fn recovery_health_router_serves_only_liveness_and_unready_health() {
        use context69::db::UpgradeGuardCounts;

        use super::{UpgradeHealthState, UpgradeRecoveryStatus, upgrade_health_router};

        let status = UpgradeRecoveryStatus::pending(
            UpgradeGuardCounts {
                affected_documents: 1,
                blocking_task_items: 1,
                blocking_remote_jobs: 1,
            },
            Duration::from_secs(3),
        );
        let state = UpgradeHealthState(Arc::new(std::sync::RwLock::new(status)));
        let router = upgrade_health_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the health listener");
        let address = listener.local_addr().expect("health listener address");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let base = format!("http://{address}");

        let live = reqwest::get(format!("{base}/livez"))
            .await
            .expect("GET /livez");
        assert_eq!(live.status(), reqwest::StatusCode::OK);

        let ready = reqwest::get(format!("{base}/readyz"))
            .await
            .expect("GET /readyz");
        assert_eq!(ready.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = ready.json().await.expect("readiness body");
        assert_eq!(body["upgrade_pending"], serde_json::json!(true));
        assert_eq!(body["blocking_remote_jobs"], serde_json::json!(1));
        assert_eq!(body["elapsed_seconds"], serde_json::json!(3));

        let health = reqwest::get(format!("{base}/healthz"))
            .await
            .expect("GET /healthz");
        assert_eq!(health.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);

        let other = reqwest::get(format!("{base}/api/v1/tasks"))
            .await
            .expect("GET a non-health route");
        assert_eq!(other.status(), reqwest::StatusCode::NOT_FOUND);

        server.abort();
    }
}
