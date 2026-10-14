//! The reset safety boundary.
//!
//! These tests are pure: they never connect. They pin the guards that must hold
//! before the destructive statement runs and prove the static reset SQL cannot
//! reach a database, a `public` schema, or an installed extension.

use context69_test_support::config::Endpoint;
use context69_test_support::runner::preflight::{
    self, ForeignBackend, LedgerLocation, LedgerRow, OwnedObjectCounts, PreflightObservation,
    Refusal,
};
use context69_test_support::runner::reset::{self, ResetAuthorization};

fn endpoint() -> Endpoint {
    Endpoint::parse("postgres://synthetic-user:synthetic-secret@db.test:5433/synthetic_db")
        .expect("synthetic fixture target")
}

fn observation() -> PreflightObservation {
    PreflightObservation {
        connected_database: "synthetic_db".to_owned(),
        search_path: "context69".to_owned(),
        current_schema: Some(preflight::OWNED_SCHEMA.to_owned()),
        effective_schemas: vec![preflight::OWNED_SCHEMA.to_owned()],
        owned_schema_exists: true,
        owned_objects: OwnedObjectCounts {
            tables: 42,
            views: 2,
            sequences: 3,
            functions: 7,
            indexes: 51,
        },
        foreign_schema_dependencies: Vec::new(),
        extensions_in_owned_schema: Vec::new(),
        ledgers: vec![LedgerLocation {
            namespace: preflight::OWNED_SCHEMA.to_owned(),
            owned_shape: true,
        }],
        public_ledger_rows: Vec::new(),
        shadowed_relations: Vec::new(),
        preserved_extension_count: 3,
        foreign_backends: Vec::new(),
    }
}

#[test]
fn idle_opt_in_requires_visible_idle_without_transaction_and_preserves_other_guards() {
    let idle = ForeignBackend {
        application_name: "diagnostic".into(),
        state: "idle".into(),
        xact_start: None,
        details_visible: true,
    };
    let mut seen = observation();
    seen.foreign_backends = vec![idle.clone()];
    assert!(!preflight::decide_reset(&seen, &endpoint()).allowed);
    let decision = preflight::decide_reset_with_policy(&seen, &endpoint(), true);
    assert!(decision.allowed);
    assert!(!ResetAuthorization::new(decision, false).permits_reset());
    for (state, xact_start, details_visible) in [
        ("active", None, true),
        ("idle in transaction", Some("timestamp"), true),
        ("idle in transaction (aborted)", Some("timestamp"), true),
        ("idle", Some("timestamp"), true),
        ("idle", None, false),
        ("", None, false),
        ("unknown", None, true),
        ("prepared", Some("timestamp"), false),
    ] {
        seen.foreign_backends = vec![
            idle.clone(),
            ForeignBackend {
                application_name: "other".into(),
                state: state.into(),
                xact_start: xact_start.map(str::to_owned),
                details_visible,
            },
        ];
        assert_eq!(
            preflight::decide_reset_with_policy(&seen, &endpoint(), true).refusals,
            vec![Refusal::ActiveApplicationBackends],
            "state={state}"
        );
    }
    seen.foreign_backends = vec![idle];
    seen.foreign_schema_dependencies
        .push("foreign object".into());
    assert_eq!(
        preflight::decide_reset_with_policy(&seen, &endpoint(), true).refusals,
        vec![Refusal::ForeignSchemaDependency]
    );
}

fn refusals(observation: &PreflightObservation) -> Vec<Refusal> {
    preflight::decide_reset(observation, &endpoint()).refusals
}

#[test]
fn an_owned_schema_with_a_verified_ledger_is_resettable() {
    let observation = observation();
    let decision = preflight::decide_reset(&observation, &endpoint());

    assert!(decision.allowed, "{:?}", decision.refusals);
    assert_eq!(decision.refusals, Vec::new());
    assert_eq!(observation.owned_objects.total(), 105);
}

#[test]
fn the_ordinary_public_ledger_layout_is_accepted_only_when_verified() {
    let mut observation = observation();
    observation.search_path = "\"$user\", public".to_owned();
    observation.current_schema = Some(preflight::PUBLIC_SCHEMA.to_owned());
    observation.effective_schemas = vec![preflight::PUBLIC_SCHEMA.to_owned()];
    observation.ledgers = vec![LedgerLocation {
        namespace: preflight::PUBLIC_SCHEMA.to_owned(),
        owned_shape: true,
    }];
    assert!(preflight::decide_reset(&observation, &endpoint()).allowed);

    observation.ledgers[0].owned_shape = false;
    assert_eq!(
        refusals(&observation),
        vec![Refusal::UnverifiedLedgerShape],
        "an unverified public ledger must not be treated as Context69's"
    );
}

#[test]
fn migration_namespace_must_be_single_verified_and_match_the_ledger() {
    let mut fresh = observation();
    fresh.ledgers.clear();
    assert!(preflight::decide_reset(&fresh, &endpoint()).allowed);
    assert_eq!(
        preflight::migration_namespace(&fresh),
        Some(preflight::OWNED_SCHEMA)
    );

    fresh.search_path = "unrelated_schema, public".to_owned();
    fresh.current_schema = Some("unrelated_schema".to_owned());
    fresh.effective_schemas = vec!["unrelated_schema".to_owned(), "public".to_owned()];
    assert_eq!(
        refusals(&fresh),
        vec![Refusal::UnverifiedMigrationNamespace]
    );

    let mut dual = observation();
    dual.ledgers.push(LedgerLocation {
        namespace: preflight::PUBLIC_SCHEMA.to_owned(),
        owned_shape: true,
    });
    assert_eq!(refusals(&dual), vec![Refusal::AmbiguousLedgerNamespace]);
}

#[test]
fn migration_connections_append_only_a_verified_search_path() {
    let endpoint = Endpoint::parse(
        "postgres://synthetic-user:synthetic-secret@db.test/synthetic_db?options=-c%20search_path%3Dother",
    )
    .expect("synthetic URL options");
    let options = preflight::migration_connect_options(&endpoint, preflight::OWNED_SCHEMA)
        .expect("owned schema is a verified migration namespace");
    assert!(
        options
            .get_options()
            .expect("URL and pinned startup options")
            .ends_with("-c search_path=context69")
    );
    assert!(preflight::migration_connect_options(&endpoint, "other").is_err());
}

#[test]
fn a_ledger_outside_the_owned_schema_and_public_is_ambiguous() {
    let mut observation = observation();
    observation.ledgers = vec![LedgerLocation {
        namespace: "tenant_a".to_owned(),
        owned_shape: true,
    }];
    assert_eq!(
        refusals(&observation),
        vec![Refusal::AmbiguousLedgerNamespace]
    );
}

#[test]
fn a_connected_database_other_than_the_selection_is_refused() {
    let mut observation = observation();
    observation.connected_database = "somewhere_else".to_owned();
    assert_eq!(
        refusals(&observation),
        vec![Refusal::ConnectedDatabaseMismatch]
    );
}

#[test]
fn objects_outside_the_owned_schema_are_never_cascaded_into() {
    let mut foreign = observation();
    foreign.foreign_schema_dependencies = vec!["reporting".to_owned()];
    assert_eq!(refusals(&foreign), vec![Refusal::ForeignSchemaDependency]);

    let mut extension = observation();
    extension.extensions_in_owned_schema = vec!["vector".to_owned()];
    assert_eq!(
        refusals(&extension),
        vec![Refusal::ExtensionInsideOwnedSchema]
    );

    let mut shadowed = observation();
    shadowed.shadowed_relations = vec!["library_file".to_owned()];
    assert_eq!(refusals(&shadowed), vec![Refusal::ShadowedOwnedRelation]);

    let mut busy = observation();
    busy.foreign_backends = vec![ForeignBackend {
        application_name: String::new(),
        xact_start: None,
        details_visible: true,
        state: "active".to_owned(),
    }];
    assert_eq!(refusals(&busy), vec![Refusal::ActiveApplicationBackends]);
}

#[test]
fn every_guard_is_evaluated_so_one_run_surfaces_all_of_them() {
    let mut observation = observation();
    observation.connected_database = "somewhere_else".to_owned();
    observation.extensions_in_owned_schema = vec!["vector".to_owned()];
    observation.foreign_backends = vec![ForeignBackend {
        application_name: String::new(),
        xact_start: None,
        details_visible: true,
        state: "idle".to_owned(),
    }];

    assert_eq!(
        refusals(&observation),
        vec![
            Refusal::ConnectedDatabaseMismatch,
            Refusal::ExtensionInsideOwnedSchema,
            Refusal::ActiveApplicationBackends,
        ]
    );
}

#[test]
fn reset_needs_both_a_passing_preflight_and_the_checkpoint_approval() {
    let allowed = preflight::ResetDecision {
        allowed: true,
        refusals: Vec::new(),
    };
    let refused = preflight::ResetDecision::refusing(vec![Refusal::ShadowedOwnedRelation]);

    assert!(ResetAuthorization::new(allowed.clone(), true).permits_reset());
    assert!(
        !ResetAuthorization::new(allowed, false).permits_reset(),
        "a passing preflight alone must not make the reset reachable"
    );
    assert!(
        !ResetAuthorization::new(refused, true).permits_reset(),
        "a checkpoint approval alone must not override a refusing preflight"
    );
}

#[test]
fn a_shared_ledger_is_accepted_only_when_every_row_is_this_builds() {
    let migrations = preflight::repository_migrations();
    assert!(
        !migrations.is_empty(),
        "the repository ships migrations, so a ledger can be checked against them"
    );
    let (version, checksum) = migrations.first().expect("a migration is present");

    let mut observation = observation();
    observation.search_path = "public".to_owned();
    observation.current_schema = Some(preflight::PUBLIC_SCHEMA.to_owned());
    observation.effective_schemas = vec![preflight::PUBLIC_SCHEMA.to_owned()];
    observation.ledgers = vec![LedgerLocation {
        namespace: preflight::PUBLIC_SCHEMA.to_owned(),
        owned_shape: true,
    }];
    observation.public_ledger_rows = vec![LedgerRow {
        version: *version,
        checksum: checksum.clone(),
        success: true,
    }];
    assert!(
        preflight::decide_reset(&observation, &endpoint()).allowed,
        "a row that matches a shipped migration is Context69's own"
    );

    // Same version, different contents: the migration was edited after it was
    // applied, so the row is not provably this build's.
    let mut edited = observation.clone();
    edited.public_ledger_rows[0].checksum = vec![0u8; checksum.len()];
    assert_eq!(
        refusals(&edited),
        vec![Refusal::UnverifiedLedgerRows],
        "a checksum mismatch means the row is not this build's migration"
    );

    let mut foreign = observation;
    foreign.public_ledger_rows.push(LedgerRow {
        version: version + 1_000_000,
        checksum: vec![0u8; checksum.len()],
        success: true,
    });
    assert_eq!(
        refusals(&foreign),
        vec![Refusal::UnverifiedLedgerRows],
        "an unrelated row means only part of the shared ledger may be cleared"
    );
}

#[test]
fn a_shared_ledger_is_only_cleared_when_it_is_outside_the_owned_schema() {
    let public = PreflightObservation {
        search_path: "public".to_owned(),
        current_schema: Some(preflight::PUBLIC_SCHEMA.to_owned()),
        effective_schemas: vec![preflight::PUBLIC_SCHEMA.to_owned()],
        ledgers: vec![LedgerLocation {
            namespace: preflight::PUBLIC_SCHEMA.to_owned(),
            owned_shape: true,
        }],
        ..observation()
    };
    assert!(
        reset::clears_ledger_separately(&public),
        "a public ledger keeps its rows when the owned schema is dropped"
    );

    let mut inside = observation();
    inside.ledgers.push(LedgerLocation {
        namespace: preflight::OWNED_SCHEMA.to_owned(),
        owned_shape: true,
    });
    assert!(
        !reset::clears_ledger_separately(&inside),
        "a ledger inside the owned schema goes away with the schema"
    );
}

#[test]
fn cascade_guard_covers_owned_catalogs_and_fails_closed_for_unknown_classes() {
    let sql = include_str!("../src/sql/foreign_schema_dependencies.sql").to_ascii_lowercase();

    for (catalog, namespace_column) in [
        ("pg_catalog.pg_class", "c.relnamespace"),
        ("pg_catalog.pg_constraint", "constraint_row.connamespace"),
        ("pg_catalog.pg_type", "t.typnamespace"),
        ("pg_catalog.pg_proc", "p.pronamespace"),
        ("pg_catalog.pg_rewrite", "rel.relnamespace"),
    ] {
        assert!(
            sql.contains(catalog) && sql.contains(namespace_column),
            "missing cascade ownership mapping for {catalog}"
        );
    }
    assert!(sql.contains("<unclassified dependent object>"));
    assert!(sql.contains("<unclassified owned object>"));
    assert!(sql.contains("own.schema_oid = referenced.namespace_oid"));
    assert!(sql.contains("dependent.namespace_oid <> own.schema_oid"));

    let mut unknown = observation();
    unknown.foreign_schema_dependencies = vec!["<unclassified dependent object>".to_owned()];
    assert_eq!(refusals(&unknown), vec![Refusal::ForeignSchemaDependency]);
}

#[test]
fn the_static_reset_sql_touches_only_the_owned_schema() {
    let sql = reset::RESET_SQL.to_ascii_uppercase();

    for forbidden in [
        "CREATE DATABASE",
        "DROP DATABASE",
        "DROP ROLE",
        "DROP EXTENSION",
        "CREATE EXTENSION",
        "DROP SCHEMA PUBLIC",
        "DROP SCHEMA IF EXISTS PUBLIC",
    ] {
        assert!(
            !sql.contains(forbidden),
            "reset SQL must not contain {forbidden}"
        );
    }
    assert_eq!(sql.matches("DROP SCHEMA").count(), 1);
    assert!(sql.contains(&format!(
        "DROP SCHEMA IF EXISTS {}",
        preflight::OWNED_SCHEMA.to_ascii_uppercase()
    )));
    assert!(sql.contains(&format!(
        "CREATE SCHEMA {}",
        preflight::OWNED_SCHEMA.to_ascii_uppercase()
    )));
}

#[test]
fn the_ledger_clear_can_only_delete_rows_this_build_shipped() {
    let sql = statements_only(reset::CLEAR_VERIFIED_LEDGER_SQL);

    assert!(
        !sql.contains("DROP"),
        "the shared ledger table must survive the reset"
    );
    assert!(
        !sql.contains("TRUNCATE"),
        "a shared ledger is never emptied wholesale"
    );
    assert!(
        !sql.replace(" ", "")
            .contains("DELETEFROMPUBLIC._SQLX_MIGRATIONS;"),
        "an unqualified delete would reach rows belonging to other applications"
    );
    // The statement is filtered by bound version/checksum pairs, so the
    // repository's own migration set decides what is removed.
    assert_eq!(sql.matches("DELETE").count(), 1);
    assert!(
        sql.contains("$1::BIGINT[]"),
        "versions are bound, not interpolated"
    );
    assert!(
        sql.contains("$2::BYTEA[]"),
        "checksums are bound, not interpolated"
    );
    assert!(sql.contains("EXPECTED.VERSION = LEDGER.VERSION"));
    assert!(sql.contains("EXPECTED.CHECKSUM = LEDGER.CHECKSUM"));
}

/// The upper-cased executable text of a checked-in statement file, with its
/// comments removed so prose naming a verb is never read as that verb.
fn statements_only(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_uppercase()
}
