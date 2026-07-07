use std::{env, fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use context69::{
    api::{self, ApiDoc},
    config::Config,
    mcp,
    services::{
        app::Context69App,
        scheduler::{
            ManualRunResult, SCHEDULER_EXECUTION_LEASE_PREFIX, SCHEDULER_VALKEY_KEY_PREFIX,
            build_valkey_execution_guard, run_manual_sync_guarded, startup_execution_slot_at,
        },
    },
};
use jsonwebtoken::crypto::rust_crypto::DEFAULT_PROVIDER as JWT_CRYPTO_PROVIDER;
use scheduler::{
    CoordinatedLeaseConfig, ExecutionSlot, GuardedRunResult, GuardedRunner, InMemoryStateStore,
    Job, OverlapPolicy, Schedule, Scheduler, SchedulerConfig, Task, TaskContext,
    ValkeyCoordinatedStateStore, ValkeyLeaseConfig,
};
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};
use utoipa::OpenApi;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let _ = JWT_CRYPTO_PROVIDER.install_default();

    let mode = env::args().nth(1).unwrap_or_else(|| "serve".to_string());
    if mode == "export-openapi" {
        export_openapi(env::args().nth(2)).await?;
        return Ok(());
    }

    let config = Config::load()?;
    let app = Arc::new(Context69App::new(config.clone()).await?);

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
        "serve" => serve(app).await?,
        other => {
            return Err(anyhow::anyhow!(
                "unsupported mode {other}; expected serve, sync-once, mcp-stdio, or export-openapi"
            ));
        }
    }

    Ok(())
}

async fn export_openapi(output_path: Option<String>) -> Result<()> {
    let output_path = output_path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("frontend/openapi/context69.openapi.json"));

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let document = serde_json::to_string_pretty(&ApiDoc::openapi())?;
    fs::write(&output_path, document)?;
    info!(path = %output_path.display(), "exported openapi document");
    Ok(())
}

async fn serve(app: Arc<Context69App>) -> Result<()> {
    let api_bind_addr = app.config.api.bind_addr.clone();

    if app.config.scheduler.run_on_start && app.sync.runtime_configured() {
        info!("running startup sync");
        if let Err(error) = run_startup_sync(app.clone()).await {
            warn!(error = %error, "startup sync failed; continuing to serve api");
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
    let router = api::router(app);
    let server_config = server::ServerConfig::new()
        .with_listen_addr(api_bind_addr)
        .build()?;
    let bound = server::axum::Server::new(server_config, router).bind()?;
    info!(addrs = ?bound.addrs(), "http api listening");
    bound.run_with_graceful_shutdown(shutdown_signal()).await?;

    if let Some(scheduler_task) = scheduler_task {
        scheduler_task.abort();
    }
    cleanup_task.abort();
    if let Some(mcp_task) = mcp_task {
        mcp_task.abort();
    }
    Ok(())
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

async fn shutdown_signal() {
    if let Err(error) = signal::ctrl_c().await {
        error!(error = %error, "failed to install Ctrl+C handler");
    }
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("context69=info,sqlx=warn"));

    fmt().with_env_filter(env_filter).with_target(false).init();
}
