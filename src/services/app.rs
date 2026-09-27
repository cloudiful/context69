use anyhow::Result;
use context69_extraction::ExtractionService;
use context69_translation::TranslationService;

use crate::{
    config::Config,
    db::Database,
    services::{
        auth::AuthService, document_store::DocumentStoreService, library::LibraryService,
        namespace::NamespaceService, personal_access_tokens::PersonalAccessTokenService,
        query::QueryService, settings::SettingsService, source_folders::SourceFoldersService,
        sync::SyncService, tasks::TaskService,
    },
};

mod background_tasks;
mod browser_sessions;
mod config_hydration;
mod library_startup;
mod readiness;
mod runtime_settings;
mod services_init;
mod vector_identity;
mod vector_rebuild;
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
    pub extraction: ExtractionService,
    pub tasks: TaskService,
    pub browser_sessions: BrowserSessionConfig,
}

impl Context69App {
    pub async fn new(mut config: Config) -> Result<Self> {
        let db = Database::connect(&config.app_db.url).await?;
        let namespace = NamespaceService::new(db.clone());
        let auth = AuthService::new(db.clone(), config.auth.clone())?;
        let personal_access_tokens = PersonalAccessTokenService::new(db.clone(), auth.clone());
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
            extraction: services.extraction,
            tasks: background.tasks,
            browser_sessions: hydration.browser_sessions,
        })
    }
}

pub(crate) fn task_worker_capacity(config: &Config) -> usize {
    // 单有效控制：阻塞 FIFO 只允许 1 个 worker；0 箝位到 1。
    crate::services::tasks::normalize_task_worker_concurrency(config.scheduler.max_concurrency)
        .min(1)
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
        assert_eq!(task_worker_capacity(&config), 1);

        // Changing file_library values must not affect capacity.
        config.file_library.ingest_concurrency = 100;
        config.file_library.url_import_concurrency = 100;
        assert_eq!(task_worker_capacity(&config), 1);

        // Scheduler fan-out collapses to a single blocking worker.
        config.scheduler.max_concurrency = 4;
        config.file_library.ingest_concurrency = 1;
        config.file_library.url_import_concurrency = 1;
        assert_eq!(task_worker_capacity(&config), 1);
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
