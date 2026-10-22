//! Read-only inspection of the configured target before any reset.
//!
//! [`collect`] is the only function here that touches the database, and every
//! statement it runs is a catalog read. The safety decision lives in
//! [`decide_reset`], which is a pure function over [`PreflightObservation`] so
//! every guard is unit-testable without a live server.

use context69_db_schema::MIGRATOR;
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgConnectOptions, PgPool};

use crate::config::{ConnectionOptionsUnavailable, Endpoint};

/// Schema Context69 owns inside the target database. Reset drops and recreates
/// exactly this schema; nothing outside it is ever touched.
pub const OWNED_SCHEMA: &str = "context69";

/// The ordinary namespace a sqlx ledger lands in when no search path moves it.
pub const PUBLIC_SCHEMA: &str = "public";

/// Migration ledger table sqlx creates for the migrations this runner owns.
pub const LEDGER_TABLE: &str = "_sqlx_migrations";

/// Namespace the `cloudiful-tasks` library owns inside the target database.
///
/// The library keeps its own migration ledger in this schema and verifies the
/// ownership of that schema itself. The runner never reads, writes, or drops
/// it, so a ledger the library confirms is not one of the runner's own.
pub const LIBRARY_SCHEMA: &str = cloudiful_tasks::migrate::SCHEMA_NAME;

/// Application name the runner sets so its own pooled sessions are excluded
/// from the "another backend is attached" guard.
pub const RUNNER_APPLICATION_NAME: &str = "context69-test-runner";

/// How many objects of each kind the owned schema currently holds.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedObjectCounts {
    pub tables: usize,
    pub views: usize,
    pub sequences: usize,
    pub functions: usize,
    pub indexes: usize,
}

impl OwnedObjectCounts {
    /// Total number of owned objects, used for run summaries.
    #[must_use]
    pub fn total(&self) -> usize {
        self.tables + self.views + self.sequences + self.functions + self.indexes
    }
}

/// Where a `_sqlx_migrations` table was found and whether its shape is the one
/// this build writes.
///
/// A ledger that the `cloudiful-tasks` library owns inside its own schema is
/// not listed here: the library owns that schema, the library verifies its own
/// ledger, and the runner never touches either. Every other ledger is listed,
/// so an unknown or an unowned foreign ledger stays a refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerLocation {
    pub namespace: String,
    pub owned_shape: bool,
}

/// One applied migration read from a ledger in `public`.
///
/// The ledger table is shared, so a row is Context69's only when its version and
/// checksum match a migration this build ships. Shape alone cannot establish
/// that: any application using sqlx writes the same six columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRow {
    pub version: i64,
    /// The migration's sha384 digest, as `BYTEA`.
    pub checksum: Vec<u8>,
    pub success: bool,
}

/// The `(version, checksum)` pairs this build's migrator owns.
///
/// Reading the compiled-in migrations is not I/O, so this stays a pure input to
/// the reset decision and to the parameterised ledger clear.
#[must_use]
pub fn repository_migrations() -> Vec<(i64, Vec<u8>)> {
    MIGRATOR
        .iter()
        .map(|migration| (migration.version, migration.checksum.to_vec()))
        .collect()
}

/// One other backend attached to the target database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignBackend {
    pub application_name: String,
    pub state: String,
    pub xact_start: Option<String>,
    pub details_visible: bool,
}

impl ForeignBackend {
    #[must_use]
    pub fn is_visible_idle(&self) -> bool {
        self.details_visible && self.state == "idle" && self.xact_start.is_none()
    }
}

/// Everything the reset decision is allowed to look at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightObservation {
    /// `current_database()` on the connected session.
    pub connected_database: String,
    /// `current_setting('search_path')` on the connected session.
    pub search_path: String,
    /// The schema PostgreSQL would use for an unqualified CREATE statement.
    pub current_schema: Option<String>,
    /// Resolved, explicit entries from `current_schemas(false)`.
    pub effective_schemas: Vec<String>,
    pub owned_schema_exists: bool,
    pub owned_objects: OwnedObjectCounts,
    /// Objects inside the owned schema that this build does not own.
    pub foreign_schema_dependencies: Vec<String>,
    /// Extensions installed inside the owned schema.
    pub extensions_in_owned_schema: Vec<String>,
    /// Migration ledgers this runner has to account for.
    ///
    /// The ledger of the `cloudiful-tasks` library is not one of them: the
    /// library owns its own schema and its own ledger inside it, and the
    /// runner neither reads nor writes either.
    pub ledgers: Vec<LedgerLocation>,
    /// Rows read from the `public` ledger, empty when it does not exist there.
    ///
    /// Only rows belonging to this build are cleared on reset, so the decision
    /// needs their contents and not just the table's shape.
    pub public_ledger_rows: Vec<LedgerRow>,
    /// Relations in `public` whose name also exists inside the owned schema.
    pub shadowed_relations: Vec<String>,
    /// Extensions installed in the database, kept across the reset.
    pub preserved_extension_count: usize,
    pub foreign_backends: Vec<ForeignBackend>,
}

/// A reason the runner refuses to reset the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The connected session reports a different database than the one that
    /// was selected.
    ConnectedDatabaseMismatch,
    /// A relation in `public` shadows a name inside the owned schema, so
    /// unqualified resolution is ambiguous.
    ShadowedOwnedRelation,
    /// Objects outside the owned schema depend on objects inside it, so a
    /// cascade would reach objects the user never designated for a test reset.
    ForeignSchemaDependency,
    /// An extension is installed inside the owned schema, so dropping it would
    /// remove a shared object.
    ExtensionInsideOwnedSchema,
    /// A migration ledger exists in a namespace that is neither the owned
    /// schema nor the ordinary `public` location, or multiple ledgers exist.
    AmbiguousLedgerNamespace,
    /// The effective search path does not establish one safe migration schema.
    UnverifiedMigrationNamespace,
    /// The `public` ledger does not have the shape this build writes, so it
    /// cannot be confirmed as Context69's.
    UnverifiedLedgerShape,
    /// The `public` ledger holds a row this build does not ship, so the ledger
    /// is shared and only some of it may be cleared.
    UnverifiedLedgerRows,
    /// Another backend is attached to the target database, so a reset could
    /// destroy in-flight application work.
    ActiveApplicationBackends,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::ConnectedDatabaseMismatch => "the connected database differs from the selection",
            Self::ShadowedOwnedRelation => {
                "a public relation shadows a relation inside the owned schema"
            }
            Self::ForeignSchemaDependency => "an object outside the owned schema depends on it",
            Self::ExtensionInsideOwnedSchema => "an extension is installed inside the owned schema",
            Self::AmbiguousLedgerNamespace => "migration ledgers do not have one verified location",
            Self::UnverifiedMigrationNamespace => {
                "the effective search path does not identify a safe migration schema"
            }
            Self::UnverifiedLedgerShape => {
                "the public migration ledger does not have the shape this build writes"
            }
            Self::UnverifiedLedgerRows => {
                "the public migration ledger holds a row this build does not ship"
            }
            Self::ActiveApplicationBackends => "another backend is attached to the target database",
        };
        f.write_str(text)
    }
}

/// The pure verdict on whether a reset may proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetDecision {
    pub allowed: bool,
    pub refusals: Vec<Refusal>,
}

impl ResetDecision {
    /// A decision that permits nothing.
    #[must_use]
    pub fn refusing(refusals: Vec<Refusal>) -> Self {
        Self {
            allowed: false,
            refusals,
        }
    }
}

/// Read the catalog of `endpoint` without writing anything.
///
/// # Errors
///
/// Returns the sqlx error when the connection cannot be established or a
/// catalog query fails.
pub async fn collect(endpoint: &Endpoint) -> Result<PreflightObservation, sqlx::Error> {
    let options = connect_options(endpoint)?;
    let pool = PgPool::connect_with(options).await?;

    let (database, search_path, current_schema, effective_schemas): (
        String,
        String,
        Option<String>,
        Vec<String>,
    ) = sqlx::query_as(include_str!("../sql/connected_identity.sql"))
        .fetch_one(&pool)
        .await?;
    let _: (i32, i32) = sqlx::query_as(include_str!("../sql/advisory_lock_keys.sql"))
        .fetch_one(&pool)
        .await?;
    let (exists,): (bool,) = sqlx::query_as(include_str!("../sql/owned_schema_exists.sql"))
        .bind(OWNED_SCHEMA)
        .fetch_one(&pool)
        .await?;
    let (tables, views, sequences, functions, indexes): (i64, i64, i64, i64, i64) =
        sqlx::query_as(include_str!("../sql/owned_object_counts.sql"))
            .bind(OWNED_SCHEMA)
            .fetch_one(&pool)
            .await?;
    let dependencies: Vec<(String,)> =
        sqlx::query_as(include_str!("../sql/foreign_schema_dependencies.sql"))
            .bind(OWNED_SCHEMA)
            .fetch_all(&pool)
            .await
            .map_err(|error| {
                sqlx::Error::Protocol(format!("dependency catalog query failed: {error}"))
            })?;
    let extensions: Vec<(String,)> =
        sqlx::query_as(include_str!("../sql/extensions_in_owned_schema.sql"))
            .bind(OWNED_SCHEMA)
            .fetch_all(&pool)
            .await?;
    let ledgers: Vec<(String, bool, bool)> =
        sqlx::query_as(include_str!("../sql/ledger_locations.sql"))
            .bind(LEDGER_TABLE)
            .bind(LIBRARY_SCHEMA)
            .fetch_all(&pool)
            .await?;
    // The library keeps its own ledger inside the schema it owns. Its own
    // read-only check decides whether that schema really is the library's, so
    // a lookalike schema and an unknown foreign ledger both stay listed and
    // stay a refusal.
    let library_owns_its_ledger = ledgers
        .iter()
        .any(|(_, _, library_namespace)| *library_namespace)
        && cloudiful_tasks::check_schema(&pool).await.is_ok();
    let ledgers = runner_ledgers(ledgers, library_owns_its_ledger);
    // The shared ledger is only read once its existence is known, so the shape
    // check above can never be preceded by a query against a missing table.
    let public_ledger_rows: Vec<(i64, Vec<u8>, bool)> = if ledgers
        .iter()
        .any(|ledger| ledger.namespace == PUBLIC_SCHEMA)
    {
        sqlx::query_as(include_str!("../sql/ledger_rows.sql"))
            .fetch_all(&pool)
            .await?
    } else {
        Vec::new()
    };
    let shadowed: Vec<(String,)> = sqlx::query_as(include_str!("../sql/shadowed_relations.sql"))
        .bind(OWNED_SCHEMA)
        .fetch_all(&pool)
        .await?;
    let (preserved,): (i64,) = sqlx::query_as(include_str!("../sql/preserved_extension_count.sql"))
        .fetch_one(&pool)
        .await?;
    let backends: Vec<(String, String, Option<String>, bool)> =
        sqlx::query_as(include_str!("../sql/foreign_backends.sql"))
            .fetch_all(&pool)
            .await?;

    pool.close().await;

    Ok(PreflightObservation {
        connected_database: database,
        search_path,
        current_schema,
        effective_schemas,
        owned_schema_exists: exists,
        owned_objects: OwnedObjectCounts {
            tables: count(tables),
            views: count(views),
            sequences: count(sequences),
            functions: count(functions),
            indexes: count(indexes),
        },
        foreign_schema_dependencies: dependencies.into_iter().map(|row| row.0).collect(),
        extensions_in_owned_schema: extensions.into_iter().map(|row| row.0).collect(),
        ledgers,
        public_ledger_rows: public_ledger_rows
            .into_iter()
            .map(|(version, checksum, success)| LedgerRow {
                version,
                checksum,
                success,
            })
            .collect(),
        shadowed_relations: shadowed.into_iter().map(|row| row.0).collect(),
        preserved_extension_count: count(preserved),
        foreign_backends: backends
            .into_iter()
            .map(
                |(application_name, state, xact_start, details_visible)| ForeignBackend {
                    application_name,
                    state,
                    xact_start,
                    details_visible,
                },
            )
            .collect(),
    })
}

/// The ledgers the runner has to account for.
///
/// Each observed ledger is a namespace, the shape of its table, and whether it
/// sits in the namespace the `cloudiful-tasks` library owns. A ledger the
/// library confirmed is left out: the runner neither reads nor writes that
/// schema, so it is not a ledger the runner may decide about. Every other
/// ledger stays, so an unknown foreign ledger and a schema that only looks
/// like the library's own both remain a refusal.
#[must_use]
fn runner_ledgers(
    observed: Vec<(String, bool, bool)>,
    library_owns_its_ledger: bool,
) -> Vec<LedgerLocation> {
    observed
        .into_iter()
        .filter(|(_, _, library_namespace)| !(*library_namespace && library_owns_its_ledger))
        .map(|(namespace, owned_shape, _)| LedgerLocation {
            namespace,
            owned_shape,
        })
        .collect()
}

/// Return the only namespace in which this observed session may place a ledger.
///
/// Search-path text is intentionally accepted in a narrow set of forms, and its
/// resolved entries must contain exactly one supported schema. Migration
/// connections still set this namespace explicitly after reset.
#[must_use]
pub fn migration_namespace(observation: &PreflightObservation) -> Option<&'static str> {
    let path = observation
        .search_path
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let schema = match observation.effective_schemas.as_slice() {
        [schema] if schema == PUBLIC_SCHEMA => PUBLIC_SCHEMA,
        [schema] if schema == OWNED_SCHEMA => OWNED_SCHEMA,
        _ => return None,
    };
    if observation.current_schema.as_deref() != Some(schema) {
        return None;
    }
    let path_is_unambiguous = match schema {
        PUBLIC_SCHEMA => matches!(
            path.as_str(),
            "public" | "\"public\"" | "$user,public" | "\"$user\",public"
        ),
        OWNED_SCHEMA => matches!(
            path.as_str(),
            "context69" | "\"context69\"" | "$user,context69" | "\"$user\",context69"
        ),
        _ => false,
    };
    path_is_unambiguous.then_some(schema)
}

/// Decide whether the observed target may be reset.
///
/// This is deliberately total and side-effect free: every guard is evaluated
/// and every refusal is reported, so one run surfaces all of them.
#[must_use]
pub fn decide_reset(observation: &PreflightObservation, endpoint: &Endpoint) -> ResetDecision {
    decide_reset_with_policy(observation, endpoint, false)
}

#[must_use]
pub fn decide_reset_with_policy(
    observation: &PreflightObservation,
    endpoint: &Endpoint,
    allow_idle_connections: bool,
) -> ResetDecision {
    let mut refusals = Vec::new();

    if observation.connected_database != endpoint.database() {
        refusals.push(Refusal::ConnectedDatabaseMismatch);
    }
    let migration_namespace = migration_namespace(observation);
    if migration_namespace.is_none() {
        refusals.push(Refusal::UnverifiedMigrationNamespace);
    }
    if !observation.shadowed_relations.is_empty() {
        refusals.push(Refusal::ShadowedOwnedRelation);
    }
    if !observation.foreign_schema_dependencies.is_empty() {
        refusals.push(Refusal::ForeignSchemaDependency);
    }
    if !observation.extensions_in_owned_schema.is_empty() {
        refusals.push(Refusal::ExtensionInsideOwnedSchema);
    }
    if observation.ledgers.len() > 1
        || observation
            .ledgers
            .iter()
            .any(|ledger| Some(ledger.namespace.as_str()) != migration_namespace)
    {
        refusals.push(Refusal::AmbiguousLedgerNamespace);
    }
    let public_ledger_present = observation
        .ledgers
        .iter()
        .any(|ledger| ledger.namespace == PUBLIC_SCHEMA);
    if public_ledger_present
        && observation
            .ledgers
            .iter()
            .any(|ledger| ledger.namespace == PUBLIC_SCHEMA && !ledger.owned_shape)
    {
        refusals.push(Refusal::UnverifiedLedgerShape);
    }
    // Ownership of a shared ledger is decided from its contents. A row this
    // build does not ship belongs to something else, so the reset would have to
    // choose which rows to clear; it refuses instead.
    if public_ledger_present {
        let owned = repository_migrations();
        if observation.public_ledger_rows.iter().any(|row| {
            !owned
                .iter()
                .any(|(version, checksum)| *version == row.version && checksum == &row.checksum)
        }) {
            refusals.push(Refusal::UnverifiedLedgerRows);
        }
    }
    if observation
        .foreign_backends
        .iter()
        .any(|backend| !allow_idle_connections || !backend.is_visible_idle())
    {
        refusals.push(Refusal::ActiveApplicationBackends);
    }

    ResetDecision {
        allowed: refusals.is_empty(),
        refusals,
    }
}

/// Build the runner's connection options for `endpoint`.
///
/// # Errors
///
/// Returns [`ConnectionOptionsUnavailable`] when sqlx rejects the selection.
pub fn connect_options(endpoint: &Endpoint) -> Result<PgConnectOptions, sqlx::Error> {
    endpoint
        .connect_options()
        .map(|options| options.application_name(RUNNER_APPLICATION_NAME))
        .map_err(|_| sqlx::Error::Configuration(Box::new(ConnectionOptionsUnavailable)))
}

/// Connection options that pin SQLx's unqualified migration ledger to the
/// preflight-verified namespace, overriding any URL startup search path.
pub fn migration_connect_options(
    endpoint: &Endpoint,
    namespace: &str,
) -> Result<PgConnectOptions, sqlx::Error> {
    if namespace != PUBLIC_SCHEMA && namespace != OWNED_SCHEMA {
        return Err(sqlx::Error::Configuration(Box::new(
            ConnectionOptionsUnavailable,
        )));
    }
    connect_options(endpoint).map(|options| options.options([("search_path", namespace)]))
}

fn count(value: i64) -> usize {
    usize::try_from(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation_with(ledgers: Vec<LedgerLocation>) -> PreflightObservation {
        PreflightObservation {
            connected_database: "synthetic_db".to_owned(),
            search_path: PUBLIC_SCHEMA.to_owned(),
            current_schema: Some(PUBLIC_SCHEMA.to_owned()),
            effective_schemas: vec![PUBLIC_SCHEMA.to_owned()],
            owned_schema_exists: true,
            owned_objects: OwnedObjectCounts::default(),
            foreign_schema_dependencies: Vec::new(),
            extensions_in_owned_schema: Vec::new(),
            ledgers,
            public_ledger_rows: Vec::new(),
            shadowed_relations: Vec::new(),
            preserved_extension_count: 0,
            foreign_backends: Vec::new(),
        }
    }

    fn endpoint() -> Endpoint {
        Endpoint::parse("postgres://synthetic-user:synthetic-secret@db.test:5433/synthetic_db")
            .expect("synthetic fixture target")
    }

    #[test]
    fn a_ledger_the_library_confirmed_is_not_one_of_the_runners_ledgers() {
        let observed = vec![
            (PUBLIC_SCHEMA.to_owned(), true, false),
            (LIBRARY_SCHEMA.to_owned(), true, true),
        ];

        let ledgers = runner_ledgers(observed, true);

        assert_eq!(
            ledgers,
            vec![LedgerLocation {
                namespace: PUBLIC_SCHEMA.to_owned(),
                owned_shape: true,
            }]
        );
    }

    #[test]
    fn a_schema_that_only_looks_like_the_librarys_own_stays_a_refusal() {
        let observed = vec![
            (PUBLIC_SCHEMA.to_owned(), true, false),
            (LIBRARY_SCHEMA.to_owned(), true, true),
        ];

        let ledgers = runner_ledgers(observed, false);
        let decision = decide_reset(&observation_with(ledgers), &endpoint());

        assert!(!decision.allowed);
        assert_eq!(decision.refusals, vec![Refusal::AmbiguousLedgerNamespace]);
    }

    #[test]
    fn an_unknown_foreign_ledger_stays_a_refusal() {
        let ledgers = runner_ledgers(
            vec![
                (PUBLIC_SCHEMA.to_owned(), true, false),
                ("tenant_a".to_owned(), true, false),
            ],
            false,
        );
        let decision = decide_reset(&observation_with(ledgers), &endpoint());

        assert!(!decision.allowed);
        assert_eq!(decision.refusals, vec![Refusal::AmbiguousLedgerNamespace]);
    }
}
