//! Redaction of the configured endpoint in every rendering the runner owns.

use context69_test_support::config::{Endpoint, Redactor};
use context69_test_support::runner::output::{RunStatus, RunSummary};

const SECRET: &str = "synthetic-secret";

fn endpoint() -> Endpoint {
    Endpoint::parse("postgres://synthetic-user:synthetic-secret@db.test:5433/synthetic_db")
        .expect("synthetic fixture target")
}

#[test]
fn the_literal_url_is_removed_even_when_it_is_reformatted() {
    let endpoint = endpoint();
    let redactor = endpoint.redactor();
    let raw = endpoint.expose();

    for text in [
        format!("connecting to {raw}"),
        format!("url={raw}&sslmode=require"),
        raw.to_owned(),
    ] {
        let redacted = redactor.redact(&text);
        assert!(!redacted.contains(SECRET), "{redacted}");
        assert!(!redacted.contains(raw), "{redacted}");
    }
}

#[test]
fn a_foreign_url_with_inline_credentials_is_still_masked() {
    let redactor = Redactor::with_secrets(["a-known-secret"]);
    let text = "psql postgresql://other:another-secret@elsewhere:5432/db";

    let redacted = redactor.redact(text);
    assert!(!redacted.contains("another-secret"), "{redacted}");
    assert!(
        redacted.contains("postgresql://other:<redacted>@"),
        "{redacted}"
    );
}

#[test]
fn text_without_credentials_is_returned_unchanged() {
    let redactor = endpoint().redactor();
    let text = "running 3 targets; migrations applied: 81";

    assert_eq!(redactor.redact(text), text);
}

#[test]
fn credential_shaped_text_inside_a_longer_line_is_masked() {
    let redactor = Redactor::default();
    let text = "child failed: (postgres://u:p@h:5433/db) while running target";

    let redacted = redactor.redact(text);
    assert_eq!(
        redacted,
        "child failed: (postgres://u:<redacted>@h:5433/db) while running target"
    );
}

#[test]
fn an_endpoint_without_a_password_is_not_masked_into_something_else() {
    let redactor = Redactor::default();
    let text = "postgres://reader@db.test:5433/synthetic_db";

    assert_eq!(redactor.redact(text), text);
}

#[test]
fn a_run_summary_carries_only_the_fingerprint_and_counts() {
    let endpoint = endpoint();
    let mut summary = RunSummary::new("run-1".to_owned());
    summary.endpoint_fingerprint = Some(endpoint.fingerprint());
    summary.endpoint_source = Some("repository .env DATABASE_URL".to_owned());
    summary.record("preflight:owned-objects=105");
    summary.status = RunStatus::Passed;

    let rendered = serde_json::to_string(&summary).expect("summary serializes");
    for secret in [
        "synthetic-secret",
        "db.test",
        "synthetic_db",
        "synthetic-user",
        "5433",
    ] {
        assert!(
            !rendered.contains(secret),
            "summary leaked {secret}: {rendered}"
        );
    }
    assert!(
        rendered.contains("105"),
        "action counts stay in the summary"
    );
}
