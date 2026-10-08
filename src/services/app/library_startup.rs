use anyhow::Result;
use tracing::{info, warn};

use crate::{
    chunking::ChunkingConfig,
    config::Config,
    db::Database,
    services::{
        library::{
            DEFAULT_LEGACY_CLEANUP_BATCH_SIZE, DEFAULT_SOURCE_OBJECT_CLEANUP_BATCH_SIZE,
            DEFAULT_SOURCE_RELEASE_RETRY_BATCH_SIZE, LibraryService, LibraryServiceConfig,
            MissingSourceCleanupSummary, SourceCleanupDispatcher,
        },
        settings::SettingsService,
        source_folders::SourceFoldersService,
    },
};

use super::{services_init::ServicesInit, vector_identity, vector_runtime::VectorRuntime};

pub struct LibraryStartup {
    pub library: LibraryService,
    pub source_folders: SourceFoldersService,
}

pub async fn initialize(
    db: &Database,
    config: &Config,
    vector: &VectorRuntime,
    settings: &SettingsService,
    services: &ServicesInit,
) -> Result<LibraryStartup> {
    let mut library = LibraryService::new(
        db.clone(),
        vector.embedding.clone(),
        vector.index.clone(),
        LibraryServiceConfig {
            chunking: ChunkingConfig {
                max_chars: config.chunking.max_chars,
                overlap_chars: config.chunking.overlap_chars,
            },
            file_library: config.file_library.clone(),
            valkey_url: config.scheduler.valkey_url.clone(),
            embedding_vector_configured: vector.embedding_vector_configured,
            embedding_vector_configuration_fingerprint: vector_identity::configuration_fingerprint(
                config,
            ),
        },
        settings.clone(),
        services.translation.clone(),
        services.extraction.clone(),
    )
    .await?;
    // Wire the shared readiness flag before the startup gates resolve, so a
    // startup rebuild keeps the embedding/qdrant gates closed until it is done.
    library.set_vector_index_ready(vector.gate.flag());
    // Shared source-cleanup dispatcher (issue 389): one Notify for the
    // process. Manual and auto releases wake it after commit; the
    // background loop drains on wake with a 5-minute fallback. Wired
    // before startup drains and task creation so every clone shares it.
    let source_cleanup_dispatcher = SourceCleanupDispatcher::new();
    library.set_source_cleanup_dispatcher(source_cleanup_dispatcher);
    // Before task workers resume, migrate any remaining legacy UUID
    // direct-path library files (storage_object_id IS NULL) onto the
    // content-addressed layout. This runs after LibraryService is ready and
    // before tasks.resume_pending()/translation/extraction resume, so new
    // ingestion cannot race the transition. Per-row errors are handled and
    // retried inside the migration; only a fatal selection error bubbles up
    // here. We tolerate that fatal error (log and continue) instead of
    // failing the whole startup: Docker operators must not enter the
    // container or run a manual migration, so the app keeps serving and
    // retries the migration on the next restart. This choice deliberately
    // leaves all unrelated startup behavior unchanged.
    match library.run_startup_legacy_migration().await {
        Ok(summary) => info!(
            scanned = summary.scanned,
            migrated = summary.migrated,
            already_migrated = summary.already_migrated,
            missing = summary.missing,
            invalid = summary.invalid,
            conflicts = summary.conflicts,
            errors = summary.errors,
            "startup legacy library direct-path migration complete"
        ),
        Err(error) => warn!(
            %error,
            "startup legacy library direct-path migration failed; it will retry on the next restart"
        ),
    }
    // Run missing-source cleanup before task workers resume: it
    // re-checks each terminal legacy direct-path row that the
    // migration could not bring onto the content-addressed layout
    // because its recorded source is gone from the active storage
    // backend. Qdrant availability gates the whole run so an outage
    // can never strand PostgreSQL rows without their vector points.
    // Per-row failures stay for the next startup; this only logs and
    // continues.
    let missing_summary = match library.run_startup_missing_source_cleanup().await {
        Ok(summary) => {
            info!(
                scanned = summary.scanned,
                confirmed_missing = summary.confirmed_missing,
                deleted = summary.deleted,
                still_present = summary.still_present,
                skipped_recent_nonterminal = summary.skipped_recent_nonterminal,
                errors = summary.errors,
                qdrant_unavailable = summary.qdrant_unavailable,
                "startup library missing-source cleanup complete"
            );
            summary
        }
        Err(error) => {
            warn!(
                %error,
                "startup library missing-source cleanup failed; it will retry on the next restart"
            );
            MissingSourceCleanupSummary::default()
        }
    };
    // Retry upload-time opt-in source releases whose success commit
    // finished in a previous process (crash or transient storage error).
    // Safe before workers resume: each file is locked per file and only
    // succeeded, opted-in, not-yet-released rows are candidates.
    match library
        .retry_pending_source_releases(DEFAULT_SOURCE_RELEASE_RETRY_BATCH_SIZE)
        .await
    {
        Ok(summary) => info!(
            scanned = summary.scanned,
            released = summary.released,
            skipped_active = summary.skipped_active,
            errors = summary.errors,
            "startup pending source release retry complete"
        ),
        Err(error) => warn!(
            %error,
            "startup pending source release retry failed; it will retry on the next pass"
        ),
    }
    // Drain any physical-deletion intents left by a previous process. Each
    // intent is bounded and rescheduled on failure, so a slow storage
    // backend cannot block startup.
    match library
        .run_source_object_cleanup(DEFAULT_SOURCE_OBJECT_CLEANUP_BATCH_SIZE)
        .await
    {
        Ok(summary) => info!(
            scanned = summary.scanned,
            deleted = summary.deleted,
            cancelled = summary.cancelled,
            failed = summary.failed,
            "startup source object cleanup complete"
        ),
        Err(error) => warn!(
            %error,
            "startup source object cleanup failed; it will retry on the next pass"
        ),
    }
    // Old-key cleanup is gated on no remaining legacy direct-path rows
    // and a clean missing-source cleanup so a partially-completed
    // migration cannot strand old objects that the missing-source
    // phase is still about to act on. Old-key cleanup honors its own
    // 7-day grace, backend matching, live reference checks, and
    // idempotent delete/mark; running it here is safe once the gate
    // holds. When no candidates ever existed (and no migration ever
    // recorded a legacy old-key record), this is a no-op.
    let legacy_direct_paths_remaining = library
        .store()
        .has_legacy_direct_path_files()
        .await
        .unwrap_or_else(|error| {
            warn!(
                %error,
                "failed to count remaining legacy direct-path rows; \
                 skipping startup old-key cleanup"
            );
            true
        });
    if missing_summary.qdrant_unavailable || missing_summary.errors > 0 {
        warn!(
            qdrant_unavailable = missing_summary.qdrant_unavailable,
            errors = missing_summary.errors,
            "skipping startup old-key cleanup because the missing-source cleanup was incomplete"
        );
    } else if legacy_direct_paths_remaining {
        info!(
            "skipping startup old-key cleanup because legacy direct-path rows remain; \
             the missing-source cleanup will revisit them on the next startup"
        );
    } else {
        match library
            .cleanup_legacy_objects(true, DEFAULT_LEGACY_CLEANUP_BATCH_SIZE)
            .await
        {
            Ok(summary) => info!(
                scanned = summary.scanned,
                eligible = summary.eligible,
                deleted = summary.deleted,
                already_missing = summary.already_missing,
                skipped_referenced = summary.skipped_referenced,
                skipped_backend = summary.skipped_backend,
                errors = summary.errors,
                "startup legacy old-key cleanup complete"
            ),
            Err(error) => warn!(
                %error,
                "startup legacy old-key cleanup failed; it will retry on the next restart"
            ),
        }
    }
    let source_folders =
        SourceFoldersService::new(db.clone(), library.clone(), services.sync.clone());
    library.initialize_dependency_gates().await?;

    Ok(LibraryStartup {
        library,
        source_folders,
    })
}
