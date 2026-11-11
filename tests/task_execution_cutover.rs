//! Pure cutover CLI argument and redaction units.
//!
//! These cases parse and validate arguments and redact a configured database
//! name without opening a connection.

use context69::services::tasks::cutover::{
    CutoverCliErrorKind, CutoverCommand, MAX_HANDOVER_ITEMS, MAX_INVENTORY_PAGE, parse_cutover_args,
};
use uuid::Uuid;

#[test]
fn the_cutover_arguments_are_refused_before_any_connection() {
    let parse = |args: &[&str]| {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        parse_cutover_args(&owned)
    };
    let uuid = Uuid::new_v4().to_string();

    assert!(parse(&[]).is_err());
    assert_eq!(
        parse(&["inventory"]).unwrap_err().kind,
        CutoverCliErrorKind::Usage
    );
    assert!(parse(&["inventory", "--expect-database", "context69"]).is_ok());
    let oversized_page = (MAX_INVENTORY_PAGE + 1).to_string();
    assert!(
        parse(&[
            "inventory",
            "--expect-database",
            "context69",
            "--limit",
            &oversized_page
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "inventory",
            "--expect-database",
            "context69",
            "--offset",
            "-1"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "inventory",
            "--expect-database",
            "context69",
            "--workers-stopped"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "inventory",
            "--expect-database",
            "context69",
            "--item",
            &uuid
        ])
        .is_err()
    );
    assert!(parse(&["adopt", "--expect-database", "context69", "--item", &uuid]).is_err());
    assert!(parse(&["adopt", "--expect-database", "context69"]).is_err());
    assert!(parse(&["adopt", "--workers-stopped", "--item", &uuid]).is_err());
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--item",
            "not-a-uuid"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--item",
            &uuid,
            "--item",
            &uuid
        ])
        .is_err()
    );
    let many: Vec<String> = (0..MAX_HANDOVER_ITEMS + 1)
        .flat_map(|_| vec!["--item".to_string(), Uuid::new_v4().to_string()])
        .collect();
    let mut args = vec![
        "adopt".to_string(),
        "--expect-database".to_string(),
        "context69".to_string(),
        "--workers-stopped".to_string(),
    ];
    args.extend(many);
    assert!(parse_cutover_args(&args).is_err());
    assert!(parse(&["unknown", "--expect-database", "context69"]).is_err());
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--item",
            &uuid,
            "--limit",
            "1"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--all",
            "--item",
            &uuid
        ])
        .is_err()
    );
    assert!(parse(&["adopt", "--expect-database", "context69", "--all"]).is_err());
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--all",
            "--all"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "finalize-interrupted",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--all"
        ])
        .is_err()
    );
    assert!(parse(&["inventory", "--expect-database", "context69", "--all"]).is_err());

    let adopt_all = parse(&[
        "adopt",
        "--expect-database",
        "context69",
        "--workers-stopped",
        "--all",
    ])
    .expect("a valid bounded adopt --all invocation");
    assert_eq!(adopt_all.command, CutoverCommand::AdoptAll);

    let parsed = parse(&[
        "finalize-interrupted",
        "--expect-database",
        " context69 ",
        "--workers-stopped",
        "--item",
        &uuid,
    ])
    .expect("a valid finalize invocation");
    assert_eq!(parsed.expect_database, " context69 ");
    assert!(parsed.workers_stopped);
    assert!(matches!(
        parsed.command,
        CutoverCommand::FinalizeInterrupted { .. }
    ));
}

#[test]
fn the_configured_database_name_is_read_but_never_echoed() {
    use context69::services::tasks::cutover::{Sanitized, configured_database_name};

    assert_eq!(
        configured_database_name("postgres://user:secret@host:5432/context69?sslmode=require")
            .expect("a name"),
        "context69"
    );
    assert!(configured_database_name("postgres://user:secret@host:5432").is_err());
    assert!(configured_database_name("").is_err());

    let redacted =
        Sanitized("connection error: postgres://app:hunter2@db.internal:5432/context69 refused")
            .to_string();
    assert!(!redacted.contains("hunter2"), "{redacted}");
    assert!(
        redacted.contains("postgres://db.internal:5432/context69"),
        "{redacted}"
    );
}
