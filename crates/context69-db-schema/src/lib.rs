use sqlx::PgPool;
use sqlx::migrate::Migrator;

pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// The schema the `cloudiful-tasks` library owns inside the application
/// database. It is separate from the schema Context69 owns and is prepared by
/// the library's own migration entrypoint on every writable normal startup.
pub const LIBRARY_SCHEMA: &str = cloudiful_tasks::migrate::SCHEMA_NAME;

/// The outcome of a library schema preparation.
pub use cloudiful_tasks::migrate::MigrationOutcome;

/// Why a library schema preparation did not complete.
pub use cloudiful_tasks::Error as LibraryError;

/// Bootstrap the schema the `cloudiful-tasks` library owns.
///
/// The call is idempotent and is the only library schema preparation this
/// workspace performs: normal writable serving startup (`Database::connect` in
/// normal admission) and the `db_init` binary both call it after the
/// application migrations, while recovery admission, read-only connections and
/// operator connections never do. A second call applies nothing. A schema that
/// exists without the ownership marker of the library is refused instead of
/// being adopted.
///
/// # Errors
///
/// Returns the library's own error when the schema cannot be created, when it
/// belongs to something else, or when the migration engine fails.
pub async fn bootstrap_library_schema(pool: &PgPool) -> Result<MigrationOutcome, LibraryError> {
    cloudiful_tasks::migrate(pool).await
}
