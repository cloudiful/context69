use anyhow::Result;
use context69_translation::TranslationService;
use std::sync::Arc;
use tracing::warn;

use crate::{
    config::{Config, ConnectionConfig},
    db::{Database, UpgradeGuardCounts},
    services::{
        auth::AuthService,
        document_store::DocumentStoreService,
        library::LibraryService,
        namespace::NamespaceService,
        personal_access_tokens::{LastUsedRecorder, PersonalAccessTokenService},
        query::QueryService,
        secret_store,
        secret_store::SecretStore,
        settings::SettingsService,
        source_folders::SourceFoldersService,
        sync::SyncService,
        tasks::{TaskService, handover},
    },
};

mod background_tasks;
mod browser_sessions;
mod config_hydration;
mod library_startup;
mod readiness;
mod runtime_settings;
mod secret_backfill;
mod services_init;
mod vector_identity;
mod vector_rebuild;
mod vector_recovery;
mod vector_runtime;

pub use browser_sessions::BrowserSessionConfig;

#[derive(Clone)]
pub struct Context69App {
    pub config: Config,
    pub db: Database,
    pub auth: AuthService,
    pub personal_access_tokens: PersonalAccessTokenService,
    pub namespace: NamespaceService,
    pub query: QueryService,
    pub sync: SyncService,
    pub settings: SettingsService,
    pub library: LibraryService,
    pub source_folders: SourceFoldersService,
    pub document_store: DocumentStoreService,
    pub translation: TranslationService,
    pub tasks: TaskService,
    /// The one path to persisted reversible runtime secrets. Later work routes
    /// provider and settings credentials through it instead of reading their
    /// own plaintext columns.
    pub secrets: SecretStore,
    pub browser_sessions: BrowserSessionConfig,
    /// The one process-wide vector runtime every vector consumer reads.
    pub vector_runtime: crate::vector_runtime::VectorRuntime,
    /// The one recovery controller for the process, kept alive (and cancelled
    /// on drop) for the lifetime of the application. It is dormant instead of
    /// absent when this instance starts without runtime settings.
    pub vector_recovery: std::sync::Arc<vector_recovery::VectorRecoveryWiring>,
}

impl Context69App {
    pub async fn new(config: Config) -> Result<Self> {
        let db = Database::connect(&config.app_db.url).await?;
        Self::with_database(config, db).await
    }

    /// Build the normal application on an already admitted database.
    ///
    /// This is the single admission boundary for every ordinary mode: a
    /// database with a pinned pending migration is rejected here, before
    /// bootstrap, hydration, startup sweeps, indexing, or any background
    /// writer exists.
    pub async fn with_database(mut config: Config, db: Database) -> Result<Self> {
        if db.pending_upgrade().is_some() {
            anyhow::bail!(
                "the database has a pinned pending migration that requires upgrade recovery; \
                 run `context69 serve` once so startup recovery finishes the pending work and \
                 applies the migration, then retry this command"
            );
        }
        // The legacy handover runs before the background tasks and before the
        // startup latch, so the engine starts on a database whose legacy items
        // already carry an execution segment. A handover that cannot finish
        // leaves the latch closed: the process stays up, the engine does not.
        let outcome = handover::run_legacy_handover(&db, handover::HandoverLimits::default()).await;
        handover::log_handover_outcome(outcome);
        let namespace = NamespaceService::new(db.clone());
        let auth = AuthService::new(db.clone(), config.auth.clone())?;
        let personal_access_tokens = PersonalAccessTokenService::new(
            db.clone(),
            auth.clone(),
            LastUsedRecorder::start(db.clone()),
        );
        auth.ensure_bootstrap_admin().await?;

        let mut hydration = config_hydration::hydrate(&db, &mut config).await?;
        let vector = vector_runtime::initialize(&db, &config, hydration.runtime_configured).await?;
        let services = services_init::initialize(&db, &config, &auth, &vector).await?;
        let startup =
            library_startup::initialize(&db, &config, &vector, &hydration.settings, &services)
                .await?;
        let background = background_tasks::start(
            &db,
            &config,
            &namespace,
            &mut hydration.settings,
            &vector,
            &services,
            &startup,
        )
        .await?;

        Ok(Self {
            config,
            db,
            auth,
            personal_access_tokens,
            namespace,
            query: services.query,
            sync: services.sync,
            settings: hydration.settings,
            library: startup.library,
            source_folders: startup.source_folders,
            document_store: background.document_store,
            translation: services.translation,
            tasks: background.tasks,
            secrets: hydration.secrets,
            browser_sessions: hydration.browser_sessions,
            vector_runtime: vector.vector.clone(),
            vector_recovery: vector.recovery.clone(),
        })
    }
}

/// The upgrade-recovery application: schema-compatible handles plus only the
/// writers needed to finish persisted ingestion work.
pub struct UpgradeRecoveryApp {
    pub config: Config,
    pub db: Database,
    pub settings: SettingsService,
    pub library: LibraryService,
    pub source_folders: SourceFoldersService,
    pub document_store: DocumentStoreService,
    pub translation: TranslationService,
    pub sync: SyncService,
    pub tasks: TaskService,
    pub vector_runtime: crate::vector_runtime::VectorRuntime,
    pub vector_recovery: Arc<vector_recovery::VectorRecoveryWiring>,
    writers: Option<background_tasks::UpgradeRecoveryWriters>,
}

impl UpgradeRecoveryApp {
    /// Connect and build the recovery profile when admission selected it.
    pub async fn new(config: Config) -> Result<Self> {
        let db = Database::connect(&config.app_db.url).await?;
        Self::with_database(config, db).await
    }

    /// Build the recovery profile on an already admitted database.
    ///
    /// Every constructor here is read-only apart from the dependency-gate state
    /// that task execution itself observes. Bootstrap, secret backfill, runtime
    /// setting import, browser-session key creation, startup sweeps, the
    /// metadata-index worker, and the payload-mirror worker stay off.
    pub async fn with_database(mut config: Config, db: Database) -> Result<Self> {
        if db.pending_upgrade().is_none() {
            anyhow::bail!("upgrade recovery requires a database with a pending pinned migration");
        }
        let namespace = NamespaceService::new(db.clone());
        let (settings, secrets, runtime_configured) =
            hydrate_upgrade_recovery(&db, &mut config).await?;
        let vector = vector_runtime::initialize(&db, &config, runtime_configured).await?;
        let (translation, sync) =
            build_upgrade_recovery_services(&db, &config, &vector, &secrets).await?;
        let startup = library_startup::initialize_recovery(
            &db,
            &config,
            &vector,
            &settings,
            &sync,
            &translation,
        )
        .await?;
        let background = background_tasks::start_upgrade_recovery(
            &db,
            &config,
            &vector,
            background_tasks::UpgradeRecoveryServices {
                namespace: &namespace,
                settings: &settings,
                sync: &sync,
                translation: &translation,
                startup: &startup,
            },
        )
        .await;
        let writers = background_tasks::start_upgrade_recovery_writers(
            &background.tasks,
            &startup.library,
            &vector,
            &background.document_store,
        );
        Ok(Self {
            config,
            db,
            settings,
            library: startup.library,
            source_folders: startup.source_folders,
            document_store: background.document_store,
            translation,
            sync,
            tasks: background.tasks,
            vector_runtime: vector.vector.clone(),
            vector_recovery: vector.recovery.clone(),
            writers: Some(writers),
        })
    }

    /// The pinned migration guards, read afresh.
    pub async fn guard_counts(&self) -> Result<UpgradeGuardCounts> {
        self.db.upgrade_guard_counts().await
    }

    /// Stop acquisition and join every writer this profile owns.
    ///
    /// The owner stays in place while it is awaited: an interrupted join keeps
    /// its handles and cancellation state, so a bounded stop can continue the
    /// same join instead of losing the writers.
    pub async fn stop_writers(&mut self) {
        let Self { writers, tasks, .. } = self;
        if let Some(writers) = writers.as_mut() {
            writers.stop_and_join(tasks).await;
        }
    }

    /// The one synchronous entry for every recovery stop path.
    ///
    /// Acquisition is closed first, before any await, so a held listener join or
    /// a held item permit can never keep the dispatcher admitting work. Already
    /// issued claims keep their workers and leases. Idempotent.
    pub fn cancel_writers(&self) {
        self.tasks.begin_shutdown();
        if let Some(writers) = self.writers.as_ref() {
            writers.cancel();
        }
    }
}

/// Resolve only the persisted runtime settings an operator maintenance tool
/// needs, without bootstrapping the application.
///
/// This is the narrow seam for `task_artifact_backfill`: it builds the secret
/// store from the deployment configuration, loads runtime settings read-only
/// (no legacy rewrite write-back, no secret backfill, no settings write) and
/// applies them to `config`. It starts no workers, writes no bootstrap rows and
/// touches no remote provider.
pub async fn hydrate_runtime_settings_for_maintenance(
    db: &Database,
    config: &mut Config,
) -> Result<Option<crate::db::StoredRuntimeSettings>> {
    let secrets = secret_store::build(db, config)?;
    let runtime = runtime_settings::load_runtime_settings_readonly(db, &secrets).await?;
    if let Some(runtime) = runtime.as_ref() {
        runtime_settings::apply_runtime_settings(config, runtime);
    }
    Ok(runtime)
}

async fn hydrate_upgrade_recovery(
    db: &Database,
    config: &mut Config,
) -> Result<(SettingsService, SecretStore, bool)> {
    let secrets = secret_store::build(db, config)?;
    let settings = SettingsService::with_secrets(db.clone(), secrets.clone());
    let runtime = runtime_settings::load_runtime_settings_readonly(db, &secrets).await?;
    if let Some(runtime) = &runtime {
        runtime_settings::apply_runtime_settings(config, runtime);
    }
    let mut connections = Vec::new();
    for connection in db.list_source_connections().await? {
        let Some(secret_key) = connection.database_url_secret_key.as_deref() else {
            continue;
        };
        match crate::services::settings::secrets::resolve_stored(
            &secrets,
            crate::services::secret_store::SecretPurpose::SourceConnectionDatabaseUrl,
            secret_key,
        )
        .await
        {
            Ok(Some(database_url)) => connections.push(ConnectionConfig {
                name: connection.name,
                database_url,
            }),
            Ok(None) => {}
            Err(error) => warn!(
                connection = connection.name,
                error = %error,
                "source connection credential is unavailable during upgrade recovery; leaving it \
                 out of the effective configuration"
            ),
        }
    }
    config.connections = connections;
    config.docling = match settings.resolve_docling_config().await {
        Ok(docling) => docling,
        Err(error) => {
            warn!(error = %error, "docling settings are invalid; upgrade recovery continues without them");
            None
        }
    };
    Ok((settings, secrets, runtime.is_some()))
}

async fn build_upgrade_recovery_services(
    db: &Database,
    config: &Config,
    vector: &vector_runtime::VectorRuntime,
    store: &SecretStore,
) -> Result<(TranslationService, SyncService)> {
    let chunking = crate::chunking::ChunkingConfig {
        max_chars: config.chunking.max_chars,
        overlap_chars: config.chunking.overlap_chars,
    };
    let translation = TranslationService::new(context69_translation::TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?,
        publisher: Arc::new(
            crate::services::translation::TranslationPublisherAdapter::with_vector_runtime(
                vector.vector.clone(),
                chunking.clone(),
            ),
        ),
        concurrency: config.scheduler.max_concurrency,
        readiness: Arc::new(readiness::LibraryEmbeddingVectorReadiness {
            store: crate::library_store::LibraryStore::new(db.clone()),
            configuration_fingerprint: vector_identity::configuration_fingerprint(config),
        }),
    })
    .with_secret_store(store.clone());
    let sync = SyncService::with_vector_runtime(
        db.clone(),
        store.clone(),
        vector.vector.clone(),
        chunking,
        config.scheduler.max_concurrency,
        translation.clone(),
    );
    sync.reload_sources().await?;
    Ok((translation, sync))
}

pub(crate) fn task_worker_capacity(config: &Config) -> usize {
    // `scheduler.max_concurrency` is the global parent-task capacity (issue 650
    // P2): the durable task lease enforces it across replicas and the local
    // worker pool mirrors it so one replica cannot run more parents than the
    // global limit. Zero still clamps to one worker.
    crate::services::tasks::normalize_task_worker_concurrency(config.scheduler.max_concurrency)
}

#[cfg(test)]
mod tests {
    use super::task_worker_capacity;
    use crate::config::Config;

    #[test]
    fn task_worker_capacity_uses_scheduler_not_file_library() {
        let mut config = Config::default();
        config.scheduler.max_concurrency = 8;
        config.file_library.ingest_concurrency = 1;
        config.file_library.url_import_concurrency = 1;
        assert_eq!(task_worker_capacity(&config), 8);

        // Changing file_library values must not affect capacity.
        config.file_library.ingest_concurrency = 100;
        config.file_library.url_import_concurrency = 100;
        assert_eq!(task_worker_capacity(&config), 8);

        config.scheduler.max_concurrency = 2;
        config.file_library.ingest_concurrency = 1;
        config.file_library.url_import_concurrency = 1;
        assert_eq!(task_worker_capacity(&config), 2);
    }

    #[test]
    fn task_worker_capacity_defaults_to_single_worker() {
        let config = Config::default();
        assert_eq!(config.scheduler.max_concurrency, 1);
        assert_eq!(task_worker_capacity(&config), 1);
    }

    #[test]
    fn task_worker_capacity_clamps_zero_to_one() {
        let mut config = Config::default();
        config.scheduler.max_concurrency = 0;
        assert_eq!(task_worker_capacity(&config), 1);

        config.scheduler.max_concurrency = 1;
        assert_eq!(task_worker_capacity(&config), 1);
    }
}
