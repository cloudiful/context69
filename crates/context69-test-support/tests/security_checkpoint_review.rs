use context69_test_support::config::Endpoint;
use context69_test_support::runner::{configure_child_command, preflight};
use preflight::{LedgerLocation, OwnedObjectCounts, PreflightObservation};
use std::process::Command;

#[test]
fn connection_catalog_projection_keeps_masked_and_prepared_sessions_blocking() {
    let sql = include_str!("../src/sql/foreign_backends.sql");
    assert!(sql.contains("query <> '<insufficient privilege>'"));
    assert!(sql.contains("query IS NOT NULL"));
    assert!(sql.contains("state IS NOT NULL"));
    assert!(sql.contains("xact_start::text"));
    assert!(sql.contains("FROM pg_prepared_xacts"));
    assert!(sql.contains("WHERE database = current_database()"));
    assert!(sql.contains("'prepared', prepared::text, false"));
    // Query content is used only for visibility, not returned or summarized.
    assert!(!sql.contains("SELECT query"));
}

fn observation() -> PreflightObservation {
    PreflightObservation {
        connected_database: "synthetic_db".to_owned(),
        search_path: "public".to_owned(),
        current_schema: Some("public".to_owned()),
        effective_schemas: vec!["public".to_owned()],
        owned_schema_exists: true,
        owned_objects: OwnedObjectCounts::default(),
        foreign_schema_dependencies: Vec::new(),
        extensions_in_owned_schema: Vec::new(),
        ledgers: Vec::new(),
        public_ledger_rows: Vec::new(),
        shadowed_relations: Vec::new(),
        preserved_extension_count: 0,
        foreign_backends: Vec::new(),
    }
}

#[test]
fn selected_identity_matches_effective_database_with_query_options() {
    let endpoint = Endpoint::parse(
        "postgres://synthetic_user:synthetic_password@db.test/synthetic_db?sslmode=disable",
    )
    .unwrap();
    let options = endpoint.connect_options().unwrap();
    assert_eq!(endpoint.database(), options.get_database().unwrap());
}

#[test]
fn database_query_override_is_the_selected_identity() {
    let endpoint = Endpoint::parse(
        "postgres://synthetic_user:synthetic_password@db.test/path_db?dbname=synthetic_db",
    )
    .unwrap();
    assert_eq!(endpoint.database(), "synthetic_db");
}

#[test]
fn sanitized_environment_removes_preexisting_production_variables() {
    let endpoint =
        Endpoint::parse("postgres://synthetic_user:synthetic_password@db.test/synthetic_db")
            .unwrap();
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("test -z \"${DATABASE_URL+x}\" && test -z \"${PGPASSWORD+x}\"")
        .env("DATABASE_URL", "synthetic_production_url")
        .env("PGPASSWORD", "synthetic_production_password");
    configure_child_command(&mut command, &endpoint);
    assert!(command.status().unwrap().success());
}

#[test]
fn unrelated_search_path_cannot_authorize_migration_writes() {
    let endpoint =
        Endpoint::parse("postgres://synthetic_user:synthetic_password@db.test/synthetic_db")
            .unwrap();
    let mut observed = observation();
    observed.search_path = "unrelated_schema, public".to_owned();
    assert!(!preflight::decide_reset(&observed, &endpoint).allowed);
}

#[test]
fn two_ledgers_are_ambiguous() {
    let endpoint =
        Endpoint::parse("postgres://synthetic_user:synthetic_password@db.test/synthetic_db")
            .unwrap();
    let mut observed = observation();
    observed.ledgers = vec![
        LedgerLocation {
            namespace: "context69".to_owned(),
            owned_shape: true,
        },
        LedgerLocation {
            namespace: "public".to_owned(),
            owned_shape: true,
        },
    ];
    assert!(!preflight::decide_reset(&observed, &endpoint).allowed);
}
