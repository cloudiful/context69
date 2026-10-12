//! The ordinary workspace pure-test gate before any database admission.

use std::future::Future;
use std::path::Path;
use std::process::Command;

use crate::config::Redactor;

use super::COMPILE_TIMEOUT;
use super::output::{RunDir, write_log};
use super::process::{self, ChildOutcome};

const WORKSPACE_TEST_LOG: &str = "workspace-pure-tests.log";
const WORKSPACE_TEST_COMMAND: &str = "cargo test --workspace --no-fail-fast --offline";
const WORKSPACE_OUTPUT_LIMIT: usize = 256 * 1024;

/// The pure stage failed before the database stage could begin.
#[derive(Debug)]
pub enum GateFailure {
    PureStart(String),
    PureFailed(ChildOutcome),
    Database(String),
}

/// Run the ordinary Cargo test suite before invoking any database work.
///
/// The callback is reached only after Cargo exits successfully. The test suite
/// keeps Cargo's default parallel harness, includes doctests, and leaves ignored
/// database and opt-in cases unselected.
pub async fn run_before_database<T, Pure, Database, DatabaseFuture>(
    pure_stage: Pure,
    database_stage: Database,
) -> Result<T, GateFailure>
where
    Pure: FnOnce() -> Result<ChildOutcome, String>,
    Database: FnOnce(ChildOutcome) -> DatabaseFuture,
    DatabaseFuture: Future<Output = Result<T, String>>,
{
    let outcome = pure_stage().map_err(GateFailure::PureStart)?;
    if !outcome.succeeded() {
        return Err(GateFailure::PureFailed(outcome));
    }
    database_stage(outcome).await.map_err(GateFailure::Database)
}

/// Execute the workspace's ordinary pure tests with bounded child lifetime and
/// redacted output before the runner resolves or connects to a database.
///
/// # Errors
///
/// Returns a credential-free message if Cargo cannot start or the log cannot be
/// written.
pub fn run_workspace_tests(repo_root: &Path, run_dir: &RunDir) -> Result<ChildOutcome, String> {
    let mut command = Command::new("cargo");
    command
        .current_dir(repo_root)
        .args(["test", "--workspace", "--no-fail-fast", "--offline"]);
    super::configure_workspace_test_command(&mut command);

    let outcome =
        process::run_logged_bounded(&mut command, COMPILE_TIMEOUT, WORKSPACE_OUTPUT_LIMIT)
            .map_err(|error| {
                format!(
                    "the ordinary workspace test suite could not start ({})",
                    error.kind()
                )
            })?;
    write_log(
        run_dir.path(),
        WORKSPACE_TEST_LOG,
        WORKSPACE_TEST_COMMAND,
        &outcome,
        &Redactor::default(),
    )
    .map_err(|_| "the ordinary workspace test log could not be written".to_owned())?;
    Ok(outcome)
}

/// Compact, credential-free action text for the run summary.
#[must_use]
pub fn outcome_action(outcome: &ChildOutcome) -> String {
    let status = if outcome.succeeded() {
        "passed"
    } else if outcome.timed_out {
        "timed-out"
    } else if outcome.interrupted {
        "interrupted"
    } else {
        "failed"
    };
    format!(
        "pure:workspace-tests={status}:exit={}",
        outcome.exit_label()
    )
}
