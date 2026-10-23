//! Reset-cycle regressions for the runner's own database lane.
//!
//! The reset decision is made once per invocation, but the reset runs once per
//! target — and the first target's migrate can create the shared `public` ledger
//! that a later reset has to clear. These cases drive the production per-target
//! function [`execute::reset_and_migrate`] twice over the owned fixture database,
//! so the wiring under test is the production wiring rather than a rehearsal of
//! it: a case that re-read the ledger itself would mask exactly the defect these
//! cases exist for.
//!
//! They are destructive, so every one of them runs the same admission sequence
//! the ordinary runner does **before its first write**: connect read-only, take
//! the runner's own [`lock::SessionLock`], re-observe the fixture under that lock,
//! and prove the exact owned target, the recorded owner comment, the ledger
//! guards, the verified migration namespace and the library-owned schema. Only
//! then do they create the preserved-object sentinel, drop the fixture state and
//! drive the production cycles. The lock stays held through every write,
//! assertion and the final cleanup, and a refused admission leaves nothing
//! behind because no statement has run yet.
//!
//! Their SQL lives in `src/sql/tests/runner_ledger_refresh/*.sql`. Every read and
//! DML statement there goes through a checked macro, so the query cache carries it
//! and an offline build verifies it; only the genuinely dynamic DDL (creating,
//! dropping and recreating the fixture's own relations and the fixture state) is
//! executed as raw SQL.
//!
//! Because they are destructive they never run on their own: each case returns
//! immediately unless the caller both supplies the owned fixture as
//! `CONTEXT69_TEST_DATABASE_URL` and explicitly opts in with
//! `CONTEXT69_RUNNER_LEDGER_REFRESH=1`:
//!
//! ```sh
//! SQLX_OFFLINE=true CONTEXT69_TEST_DATABASE_URL=… CONTEXT69_RUNNER_LEDGER_REFRESH=1 \
//!   cargo test -p context69-test-support --lib 'runner::reset::tests::'
//! ```
//!
//! `SQLX_OFFLINE=true` is what the runner exports to its own children, and it is
//! required here too: the fixture is deliberately emptied by these cases, so the
//! checked macros must compile from the committed cache rather than describe
//! against a database that is mid-cycle.
//!
//! The explicit opt-in keeps them out of a default suite that happens to export a
//! database URL, and keeps the runner's manifest free of an entry for its own
//! test: an ignored case would have to be classified, and classifying it would
//! make a run select its own regression. Within one invocation the opt-in cases
//! also serialise on an in-process mutex, so two of them can never interleave
//! their setup on the same fixture; that mutex is a convenience for the test
//! process only — the database lock above is what excludes another process.

use sqlx::postgres::PgPool;

use crate::config::Endpoint;
use crate::runner::{execute, lock, preflight};

/// The parent-owned isolated TEST fixture these cases may recreate.
const FIXTURE_DATABASE: &str = "context69_tasks_1020_test";

/// The owner comment the parent recorded on that fixture.
const FIXTURE_COMMENT: &str = "cloudiful-tasks Context69 issue1020 isolated TEST fixture 20261022";

/// The relation the preserved-object sentinel uses, in the ordinary schema.
const SENTINEL_RELATION: &str = "context69_runner_ledger_refresh_sentinel";

/// A ledger version this build does not ship.
const FOREIGN_LEDGER_VERSION: i64 = 9223372036854;

/// The opt-in that lets these destructive cases touch the fixture at all.
const OPT_IN_VAR: &str = "CONTEXT69_RUNNER_LEDGER_REFRESH";

/// Serialises the opt-in cases inside this test process.
///
/// The database lock already excludes another process; this only stops two cases
/// of the same invocation from overlapping their own setup.
static OPT_IN_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What the fixture's own catalog row says about its identity.
struct FixtureIdentity {
    database: String,
    comment: String,
}

/// The fixture target, or `None` when this invocation has not opted in.
///
/// Both conditions are required: the owned fixture as the supplied target, and
/// the explicit opt-in. A default suite that happens to export a database URL
/// therefore runs nothing here.
fn fixture_target() -> Option<Endpoint> {
    if std::env::var(OPT_IN_VAR).as_deref() != Ok("1") {
        println!(
            "skipping: set {OPT_IN_VAR}=1 and CONTEXT69_TEST_DATABASE_URL to the owned \
             {FIXTURE_DATABASE} fixture to run this reset-cycle regression"
        );
        return None;
    }
    let raw = std::env::var("CONTEXT69_TEST_DATABASE_URL").unwrap_or_default();
    let endpoint = Endpoint::parse(&raw).expect("the supplied target is a usable URL");
    assert_eq!(
        endpoint.database(),
        FIXTURE_DATABASE,
        "these cases recreate only the owned fixture database"
    );
    Some(endpoint)
}

async fn connect(endpoint: &Endpoint) -> PgPool {
    let options = preflight::connect_options(endpoint).expect("runner connection options");
    PgPool::connect_with(options)
        .await
        .expect("connect the fixture database")
}

async fn extensions(pool: &PgPool) -> i64 {
    sqlx::query_file_scalar!("src/sql/tests/runner_ledger_refresh/installed_extensions.sql")
        .fetch_one(pool)
        .await
        .expect("count the installed extensions")
}

async fn library_relations(pool: &PgPool) -> i64 {
    sqlx::query_file_scalar!("src/sql/tests/runner_ledger_refresh/library_schema_relations.sql")
        .fetch_one(pool)
        .await
        .expect("count the library relations")
}

async fn owned_relations(pool: &PgPool) -> i64 {
    sqlx::query_file_scalar!("src/sql/tests/runner_ledger_refresh/owned_schema_relations.sql")
        .fetch_one(pool)
        .await
        .expect("count the owned relations")
}

async fn owned_groups_present(pool: &PgPool) -> bool {
    sqlx::query_file_scalar!("src/sql/tests/runner_ledger_refresh/owned_schema_groups_present.sql")
        .fetch_one(pool)
        .await
        .expect("read the owned schema")
}

async fn sentinel_present(pool: &PgPool) -> bool {
    sqlx::query_file_scalar!(
        "src/sql/tests/runner_ledger_refresh/public_relation_present.sql",
        SENTINEL_RELATION
    )
    .fetch_one(pool)
    .await
    .expect("read the sentinel")
}

async fn read_identity(pool: &PgPool) -> FixtureIdentity {
    let row = sqlx::query_file!("src/sql/tests/runner_ledger_refresh/target_database_identity.sql")
        .fetch_one(pool)
        .await
        .expect("read the target identity");
    FixtureIdentity {
        database: row.database,
        comment: row.comment,
    }
}

/// Every read-only refusal that must precede the first fixture write.
///
/// This is the admission contract: the target name, the owner comment the parent
/// recorded, the run-level ledger decision, the verified migration namespace, the
/// accounted ledgers all being the ordinary `public` one, and the library's own
/// schema being present. An empty result is the only state in which a case may
/// write.
fn admission_refusals(
    endpoint: &Endpoint,
    identity: &FixtureIdentity,
    observation: &preflight::PreflightObservation,
    decision: &preflight::ResetDecision,
    library_relation_count: i64,
) -> Vec<String> {
    let mut refusals = Vec::new();
    if identity.database != FIXTURE_DATABASE {
        refusals.push(format!(
            "the connected database is {:?}, not the owned fixture",
            identity.database
        ));
    }
    if identity.comment != FIXTURE_COMMENT {
        refusals.push(format!(
            "the fixture no longer carries its recorded owner comment: {:?}",
            identity.comment
        ));
    }
    if endpoint.database() != FIXTURE_DATABASE {
        refusals.push(format!(
            "the connection options address {:?}, not the owned fixture",
            endpoint.database()
        ));
    }
    if !decision.allowed {
        refusals.push(format!(
            "the run-level ledger decision refuses this target: {:?}",
            decision.refusals
        ));
    }
    if preflight::migration_namespace(observation).is_none() {
        refusals.push("the migration namespace is not verified".to_owned());
    }
    if observation
        .ledgers
        .iter()
        .any(|ledger| ledger.namespace != preflight::PUBLIC_SCHEMA)
    {
        refusals.push(format!(
            "a ledger sits outside the ordinary schema: {:?}",
            observation.ledgers
        ));
    }
    if library_relation_count <= 0 {
        refusals.push("the fixture no longer carries the library's own schema".to_owned());
    }
    refusals
}

/// The gate the first fixture write sits behind.
///
/// The write itself is supplied by the caller, so a pure case can hand in a
/// closure that only records the attempt and prove that a refused admission never
/// reaches it. `Err` carries the refusals and nothing has been written.
fn begin_setup<W: FnOnce()>(refusals: Vec<String>, write: W) -> Result<(), Vec<String>> {
    if refusals.is_empty() {
        write();
        Ok(())
    } else {
        Err(refusals)
    }
}

/// The admitted, locked fixture one destructive case may work on.
///
/// Holding the guard is what makes the destructive phase safe: it excludes the
/// ordinary runner for as long as this value lives, and it is dropped only after
/// the case's cleanup has run.
struct AdmittedFixture {
    endpoint: Endpoint,
    pool: PgPool,
    /// Held for the whole destructive phase of a case and released only when the
    /// value is dropped, after cleanup. The field is deliberately never read: it
    /// exists to be alive.
    _guard: lock::SessionLock,
    observation: preflight::PreflightObservation,
    decision: preflight::ResetDecision,
}

impl AdmittedFixture {
    /// Recreate the fixture's initial state: no owned schema and no shared
    /// ledger. Reachable only from an admitted fixture.
    async fn recreate_empty_state(&self) {
        sqlx::raw_sql(include_str!(
            "../../sql/tests/runner_ledger_refresh/drop_owned_schema_and_shared_ledger.sql"
        ))
        .execute(&self.pool)
        .await
        .expect("recreate the empty fixture state");
    }

    /// One production target cycle through the real per-target function.
    async fn production_cycle(&self, cycle: u32) {
        execute::reset_and_migrate(&self.endpoint, &self.observation, &self.decision, None)
            .await
            .unwrap_or_else(|(stage, message)| {
                panic!("production cycle {cycle} stopped at {stage:?}: {message}")
            });
    }
}

/// Connect read-only, take the runner's own lock, and admit the fixture under it.
///
/// Every check here is a read. The first write of a case happens only after this
/// returns, never before, and the returned guard covers the whole destructive
/// phase including cleanup.
async fn admitted_locked_fixture() -> Option<AdmittedFixture> {
    let endpoint = fixture_target()?;
    // A read-only connection for the identity and preservation reads.
    let pool = connect(&endpoint).await;

    // The same single advisory lock the ordinary runner takes, held from here to
    // the end of the case.
    let guard = lock::SessionLock::acquire(&endpoint)
        .await
        .expect("the runner advisory lock");
    assert!(
        matches!(lock::admit(Some(&guard)), lock::Admission::Proceed),
        "the acquired lock means this case owns the target"
    );

    // Re-observe under the lock, exactly as the runner does.
    let observation = preflight::collect(&endpoint)
        .await
        .expect("read-only inspection under the runner lock");
    // The human-confirmed single-user window admits the fixture's idle pool
    // sessions; every other guard still applies.
    let decision = preflight::decide_reset_with_policy(&observation, &endpoint, true);
    let identity = read_identity(&pool).await;
    let library = library_relations(&pool).await;

    let refusals = admission_refusals(&endpoint, &identity, &observation, &decision, library);
    begin_setup(refusals, || ()).unwrap_or_else(|refusals| {
        panic!(
            "the fixture is refused before any write; nothing was created or dropped: {refusals:?}"
        )
    });

    Some(AdmittedFixture {
        endpoint,
        pool,
        _guard: guard,
        observation,
        decision,
    })
}

/// Two production target cycles from a fixture with no owned schema and no
/// shared ledger must leave a migrated schema behind.
///
/// This is the regression for the empty initial ledger: the run level
/// observation is taken once, so a reset that decided from it alone would clear
/// nothing on the second target, migrate nothing, and leave the owned schema
/// empty.
#[tokio::test]
async fn two_production_cycles_from_an_empty_fixture_migrate_every_target() {
    let _serial = OPT_IN_SERIAL.lock().await;
    let Some(fixture) = admitted_locked_fixture().await else {
        return;
    };
    let pool = &fixture.pool;

    let extensions_before = extensions(pool).await;
    let library_before = library_relations(pool).await;
    let sentinel_before = sentinel_present(pool).await;
    sqlx::raw_sql(include_str!(
        "../../sql/tests/runner_ledger_refresh/create_public_sentinel.sql"
    ))
    .execute(pool)
    .await
    .expect("create the preserved-object sentinel");

    fixture.recreate_empty_state().await;
    assert!(
        !owned_groups_present(pool).await,
        "the fixture starts with no owned schema"
    );

    for cycle in 1..=2 {
        fixture.production_cycle(cycle).await;
        assert!(
            owned_groups_present(pool).await,
            "cycle {cycle} left a migrated owned schema"
        );
        let relations = owned_relations(pool).await;
        assert!(
            relations > 0,
            "cycle {cycle} left {relations} relations in the owned schema"
        );
    }

    let rows = sqlx::query_file!("src/sql/tests/runner_ledger_refresh/applied_ledger_rows.sql")
        .fetch_all(pool)
        .await
        .expect("read the shared ledger");
    let shipped = preflight::repository_migrations();
    assert_eq!(
        rows.len(),
        shipped.len(),
        "the shared ledger holds exactly this build's migrations"
    );
    for row in &rows {
        assert!(row.success, "migration {} applied", row.version);
        let owned = shipped.iter().any(|(shipped_version, shipped_checksum)| {
            *shipped_version == row.version
                && shipped_checksum
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
                    == row.checksum
        });
        assert!(
            owned,
            "migration {} is one this build ships, not a foreign row",
            row.version
        );
    }

    assert!(
        sentinel_present(pool).await,
        "the preserved relation in the ordinary schema survives every reset"
    );
    assert_eq!(
        extensions(pool).await,
        extensions_before,
        "the installed extensions survive every reset"
    );
    assert_eq!(
        library_relations(pool).await,
        library_before,
        "the library's own schema survives every reset"
    );

    sqlx::raw_sql(include_str!(
        "../../sql/tests/runner_ledger_refresh/drop_public_sentinel.sql"
    ))
    .execute(pool)
    .await
    .expect("remove the sentinel");
    assert_eq!(
        sentinel_present(pool).await,
        sentinel_before,
        "cleanup leaves the ordinary schema as it was found"
    );
}

/// A shared-ledger row this build does not ship refuses the next reset before it
/// writes anything.
///
/// The advisory lock excludes other runners, not other applications, so a ledger
/// that changes between two targets is the case the per-target decision has to
/// catch on its own.
#[tokio::test]
async fn a_foreign_shared_ledger_row_refuses_the_next_reset_before_it_writes() {
    let _serial = OPT_IN_SERIAL.lock().await;
    let Some(fixture) = admitted_locked_fixture().await else {
        return;
    };
    let pool = &fixture.pool;

    let extensions_before = extensions(pool).await;
    let library_before = library_relations(pool).await;
    sqlx::raw_sql(include_str!(
        "../../sql/tests/runner_ledger_refresh/create_public_sentinel.sql"
    ))
    .execute(pool)
    .await
    .expect("create the preserved-object sentinel");

    fixture.recreate_empty_state().await;
    fixture.production_cycle(1).await;
    assert!(
        owned_groups_present(pool).await,
        "the first cycle migrated the owned schema"
    );

    sqlx::query_file!(
        "src/sql/tests/runner_ledger_refresh/insert_foreign_ledger_row.sql",
        FOREIGN_LEDGER_VERSION
    )
    .execute(pool)
    .await
    .expect("inject one shared-ledger row this build does not ship");

    // The fresh per-target view must refuse, which is what keeps the next reset
    // from dropping the owned schema on the strength of a ledger it cannot own.
    match preflight::refresh_ledger_view(&fixture.endpoint, &fixture.observation).await {
        Err(preflight::LedgerRefreshError::Refused(refusals)) => assert!(
            refusals.contains(&preflight::Refusal::UnverifiedLedgerRows),
            "a foreign row refuses through the ledger-row guard, got {refusals:?}"
        ),
        other => panic!("a foreign shared-ledger row must refuse the reset: {other:?}"),
    }
    // The production sequence refuses at the same point, before the reset write.
    assert!(
        execute::reset_and_migrate(
            &fixture.endpoint,
            &fixture.observation,
            &fixture.decision,
            None
        )
        .await
        .is_err(),
        "the production per-target sequence refuses a ledger it cannot own"
    );

    assert!(
        owned_groups_present(pool).await,
        "the refused reset wrote nothing: the owned schema is untouched"
    );
    assert!(
        sentinel_present(pool).await,
        "the refused reset left the ordinary schema alone"
    );
    assert_eq!(
        extensions(pool).await,
        extensions_before,
        "the refused reset left the extensions alone"
    );
    assert_eq!(
        library_relations(pool).await,
        library_before,
        "the refused reset left the library's schema alone"
    );
    let rows = sqlx::query_file!("src/sql/tests/runner_ledger_refresh/applied_ledger_rows.sql")
        .fetch_all(pool)
        .await
        .expect("read the shared ledger");
    assert!(
        rows.iter().any(|row| row.version == FOREIGN_LEDGER_VERSION),
        "the injected foreign row is still there"
    );

    sqlx::query_file!(
        "src/sql/tests/runner_ledger_refresh/delete_foreign_ledger_row.sql",
        FOREIGN_LEDGER_VERSION
    )
    .execute(pool)
    .await
    .expect("remove the injected foreign row");
    sqlx::raw_sql(include_str!(
        "../../sql/tests/runner_ledger_refresh/drop_public_sentinel.sql"
    ))
    .execute(pool)
    .await
    .expect("remove the sentinel");
}

/// The ordinary runner's lock excludes a second destructive case on the same
/// fixture, and the refused contender writes nothing.
#[tokio::test]
async fn the_owned_fixture_lock_denies_a_second_destructive_holder() {
    let _serial = OPT_IN_SERIAL.lock().await;
    let Some(endpoint) = fixture_target() else {
        return;
    };
    let pool = connect(&endpoint).await;

    // The normal runner's own lock, taken first.
    let held = lock::SessionLock::acquire(&endpoint)
        .await
        .expect("the runner advisory lock");
    assert!(matches!(lock::admit(Some(&held)), lock::Admission::Proceed));

    let sentinel_before = sentinel_present(&pool).await;
    let relations_before = owned_relations(&pool).await;

    // A second holder is denied by the existing mechanism, without waiting.
    let second = lock::SessionLock::try_acquire(&endpoint)
        .await
        .expect("a non-blocking lock attempt");
    assert!(
        second.is_none(),
        "the held runner lock excludes a second holder"
    );
    assert_eq!(
        lock::admit(second.as_ref()),
        lock::Admission::RefuseAnotherRunnerActive,
        "a denied contender is refused, never admitted"
    );

    // The denied contender performed no fixture write.
    assert_eq!(
        sentinel_present(&pool).await,
        sentinel_before,
        "a denied contender creates nothing"
    );
    assert_eq!(
        owned_relations(&pool).await,
        relations_before,
        "a denied contender drops nothing"
    );
}

/// The admission gate itself: every unproven precondition stops the first write.
///
/// Pure — no database, no live fixture. It drives the same
/// [`admission_refusals`] and [`begin_setup`] the live cases use, with a closure
/// that records the attempted write instead of performing one.
#[test]
fn a_refused_admission_never_reaches_the_first_fixture_write() {
    let endpoint = Endpoint::parse(&format!(
        "postgres://user:secret@example.invalid:5432/{FIXTURE_DATABASE}"
    ))
    .expect("a synthetic target");
    // A synthetic observation the gate accepts; no database is contacted.
    let observation = preflight::PreflightObservation {
        connected_database: FIXTURE_DATABASE.to_owned(),
        search_path: "public".to_owned(),
        current_schema: Some("public".to_owned()),
        effective_schemas: vec!["public".to_owned()],
        owned_schema_exists: true,
        owned_objects: preflight::OwnedObjectCounts::default(),
        foreign_schema_dependencies: Vec::new(),
        extensions_in_owned_schema: Vec::new(),
        ledgers: vec![preflight::LedgerLocation {
            namespace: "public".to_owned(),
            owned_shape: true,
        }],
        public_ledger_rows: Vec::new(),
        shadowed_relations: Vec::new(),
        preserved_extension_count: 0,
        foreign_backends: Vec::new(),
    };
    let allowed = preflight::ResetDecision {
        allowed: true,
        refusals: Vec::new(),
    };
    let identity = || FixtureIdentity {
        database: FIXTURE_DATABASE.to_owned(),
        comment: FIXTURE_COMMENT.to_owned(),
    };

    let refused = |name: &str, refusals: Vec<String>| {
        let mut wrote = false;
        let outcome = begin_setup(refusals, || wrote = true);
        assert!(
            outcome.is_err(),
            "{name}: a refused admission must not report success"
        );
        assert!(
            !wrote,
            "{name}: the first fixture write must not be reached"
        );
    };

    refused(
        "a foreign database name",
        admission_refusals(
            &endpoint,
            &FixtureIdentity {
                database: "some_other_database".to_owned(),
                comment: FIXTURE_COMMENT.to_owned(),
            },
            &observation,
            &allowed,
            1,
        ),
    );
    refused(
        "a missing original owner comment",
        admission_refusals(
            &endpoint,
            &FixtureIdentity {
                database: FIXTURE_DATABASE.to_owned(),
                comment: String::new(),
            },
            &observation,
            &allowed,
            1,
        ),
    );
    refused(
        "a changed owner comment",
        admission_refusals(
            &endpoint,
            &FixtureIdentity {
                database: FIXTURE_DATABASE.to_owned(),
                comment: "somebody else's fixture".to_owned(),
            },
            &observation,
            &allowed,
            1,
        ),
    );
    refused(
        "a denied run-level decision",
        admission_refusals(
            &endpoint,
            &identity(),
            &observation,
            &preflight::ResetDecision::refusing(vec![preflight::Refusal::UnverifiedLedgerRows]),
            1,
        ),
    );
    refused(
        "an unverified migration namespace",
        admission_refusals(
            &endpoint,
            &identity(),
            &preflight::PreflightObservation {
                effective_schemas: vec!["other".to_owned()],
                ..observation.clone()
            },
            &allowed,
            1,
        ),
    );
    refused(
        "a ledger outside the ordinary schema",
        admission_refusals(
            &endpoint,
            &identity(),
            &preflight::PreflightObservation {
                ledgers: vec![preflight::LedgerLocation {
                    namespace: "context69".to_owned(),
                    owned_shape: true,
                }],
                ..observation.clone()
            },
            &allowed,
            1,
        ),
    );
    refused(
        "the library schema missing",
        admission_refusals(&endpoint, &identity(), &observation, &allowed, 0),
    );

    // The accepted state is the only one that reaches the write, and it does.
    let mut wrote = false;
    begin_setup(
        admission_refusals(&endpoint, &identity(), &observation, &allowed, 1),
        || wrote = true,
    )
    .expect("a fully admitted fixture proceeds");
    assert!(wrote, "the admitted fixture reaches its first write");
}
