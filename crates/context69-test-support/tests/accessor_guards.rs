//! The shared database configuration accessor, with no database in reach.
//!
//! Each environment-sensitive case runs in its own subprocess with a synthetic,
//! explicitly constructed environment. Nothing in this file opens a connection.

use std::process::Command;

use context69_test_support::TEST_DATABASE_URL_VAR;
use context69_test_support::accessor::{
    self, APP_STARTUP_VAR, DB_IGNORE_REASON, KEYWORD_BENCHMARK_VAR,
};

const CHILD_CASE_VAR: &str = "CONTEXT69_ACCESSOR_GUARD_CHILD_CASE";

fn child_case() -> Option<String> {
    std::env::var(CHILD_CASE_VAR).ok()
}

fn launch_isolated(test_name: &str, case: &str, environment: &[(&str, &str)]) {
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", test_name, "--quiet"])
        .env_clear()
        .env(CHILD_CASE_VAR, case)
        .envs(environment.iter().copied())
        .output()
        .expect("launch isolated accessor guard");
    assert!(
        output.status.success(),
        "isolated accessor guard failed: {case}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn an_absent_target_fails_loudly_instead_of_skipping() {
    if child_case().as_deref() != Some("absent-target") {
        launch_isolated(
            "an_absent_target_fails_loudly_instead_of_skipping",
            "absent-target",
            &[],
        );
        return;
    }

    assert!(!accessor::is_configured());

    let failure = std::panic::catch_unwind(accessor::database_url)
        .expect_err("an explicitly invoked database test must not proceed");
    let message = failure
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(
        message.contains(TEST_DATABASE_URL_VAR),
        "the failure names the missing configuration: {message}"
    );
    assert!(
        !message.contains("postgres://") && !message.contains("postgresql://"),
        "the failure carries no connection value"
    );
}

#[test]
fn a_malformed_target_fails_loudly_instead_of_connecting() {
    if child_case().as_deref() != Some("malformed-target") {
        launch_isolated(
            "a_malformed_target_fails_loudly_instead_of_connecting",
            "malformed-target",
            &[(TEST_DATABASE_URL_VAR, "mysql://user:pass@example/db")],
        );
        return;
    }

    let failure = std::panic::catch_unwind(accessor::database_url)
        .expect_err("a target that is not PostgreSQL must be refused");
    let message = failure
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(message.contains(TEST_DATABASE_URL_VAR), "{message}");
    assert!(
        !message.contains("pass"),
        "the refusal carries no credential from the value: {message}"
    );
}

#[test]
fn a_blank_target_counts_as_absent() {
    if child_case().as_deref() != Some("blank-target") {
        launch_isolated(
            "a_blank_target_counts_as_absent",
            "blank-target",
            &[(TEST_DATABASE_URL_VAR, "   ")],
        );
        return;
    }

    assert!(
        !accessor::is_configured(),
        "whitespace is not a configured target"
    );
    assert!(std::panic::catch_unwind(accessor::database_url).is_err());
}

#[test]
fn a_disabled_opt_in_is_reported_as_disabled_rather_than_passed() {
    match child_case().as_deref() {
        Some("opt-ins-absent") => {
            assert!(
                !accessor::app_startup_enabled(),
                "the application-startup boot stays off until it is asked for"
            );
            assert!(
                !accessor::keyword_benchmark_enabled(),
                "the keyword benchmark stays off until it is asked for"
            );
        }
        Some("opt-ins-configured") => {
            assert!(
                !accessor::app_startup_enabled(),
                "an explicitly disabled value does not enable the opt-in"
            );
            assert!(accessor::keyword_benchmark_enabled());
        }
        _ => {
            launch_isolated(
                "a_disabled_opt_in_is_reported_as_disabled_rather_than_passed",
                "opt-ins-absent",
                &[],
            );
            launch_isolated(
                "a_disabled_opt_in_is_reported_as_disabled_rather_than_passed",
                "opt-ins-configured",
                &[(APP_STARTUP_VAR, "0"), (KEYWORD_BENCHMARK_VAR, "yes")],
            );
        }
    }
}

#[test]
fn the_ignore_reason_names_the_requirement_without_a_value() {
    assert!(DB_IGNORE_REASON.contains("PostgreSQL"));
    assert!(!DB_IGNORE_REASON.contains("://"));
}
