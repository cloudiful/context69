use sqlx::PgPool;
use sqlx::migrate::Migrator;

pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// The schema the `cloudiful-tasks` library owns inside the application
/// database. It is separate from the schema Context69 owns and is created only
/// by an explicit bootstrap.
pub const LIBRARY_SCHEMA: &str = cloudiful_tasks::migrate::SCHEMA_NAME;

/// The outcome of an explicit library bootstrap.
pub use cloudiful_tasks::migrate::MigrationOutcome;

/// Why an explicit library bootstrap did not complete.
pub use cloudiful_tasks::Error as LibraryError;

/// Bootstrap the schema the `cloudiful-tasks` library owns.
///
/// The call is explicit and idempotent: the library schema is created only
/// here, never by connecting the application or by opening a queue, and a
/// second call applies nothing. A schema that exists without the ownership
/// marker of the library is refused instead of being adopted.
///
/// # Errors
///
/// Returns the library's own error when the schema cannot be created, when it
/// belongs to something else, or when the migration engine fails.
pub async fn bootstrap_library_schema(pool: &PgPool) -> Result<MigrationOutcome, LibraryError> {
    cloudiful_tasks::migrate(pool).await
}
