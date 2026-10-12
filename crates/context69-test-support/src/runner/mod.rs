//! The backend database-test runner.
//!
//! The runner owns the sequence that turns a configured target into a verified
//! test run: the run mode first passes the ordinary pure Cargo suite, then
//! resolves privately, inspects read-only, holds one advisory lock, and only
//! then resets, migrates, and executes. Every mode reports through
//! [`output::RunSummary`], which carries the opaque endpoint fingerprint and
//! action counts rather than any configuration value.

pub mod cancellation;
pub mod execute;
pub mod list;
pub mod lock;
pub mod manifest;
pub mod output;
pub mod preflight;
pub mod process;
pub mod process_group;
pub mod process_signal;
pub mod process_tree;
pub mod pure;
pub mod reset;
pub mod run;
pub mod targets;

use std::process::Command;
use std::time::Duration;

use crate::config::Endpoint;

/// How the runner was invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Read-only catalog inspection; never writes and never resets.
    Preflight,
    /// Reconcile the manifest against the compiled test list; no database.
    List,
    /// Reset, migrate, and execute the database-backed tests.
    Run,
}

/// One parsed invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invocation {
    pub mode: Mode,
    /// Set only after the recorded security checkpoint approved a real reset.
    /// Without it the runner stops after preflight and reports the refusal.
    pub reset_authorized: bool,
    /// Run the opt-in keyword benchmark in addition to the ordinary set.
    pub keyword_benchmark: bool,
    /// Run the opt-in application-startup service boot in addition to the
    /// ordinary set.
    pub app_startup: bool,
}

/// Compile-time budget for the workspace test build.
pub const COMPILE_TIMEOUT: Duration = Duration::from_secs(2400);

/// Per-target execution budget.
pub const TARGET_TIMEOUT: Duration = Duration::from_secs(1800);

/// Exit codes the runner uses so a shell caller can branch without parsing
/// output.
pub mod exit {
    /// The requested work completed.
    pub const OK: i32 = 0;
    /// A target failed, timed out, or could not be started.
    pub const FAILED: i32 = 1;
    /// The invocation was malformed.
    pub const USAGE: i32 = 2;
    /// Preflight or lock admission refused the run before any write.
    pub const REFUSED: i32 = 3;
}

/// Parse the runner's own flags.
///
/// # Errors
///
/// Returns a usage message for an unknown flag or two conflicting modes, so a
/// mistyped authorization flag can never silently proceed.
pub fn parse_invocation(args: &[String]) -> Result<Invocation, String> {
    let mut mode = Mode::Run;
    let mut selected = false;
    let mut reset_authorized = false;
    let mut keyword_benchmark = false;
    let mut app_startup = false;
    for arg in args {
        match arg.as_str() {
            "--preflight" | "--list" => {
                let requested = if arg == "--preflight" {
                    Mode::Preflight
                } else {
                    Mode::List
                };
                if selected && mode != requested {
                    return Err("choose one of --preflight or --list".to_owned());
                }
                mode = requested;
                selected = true;
            }
            // Passed only by the orchestrator once the recorded security
            // checkpoint has approved the reset boundary.
            "--authorize-reset" => reset_authorized = true,
            "--with-keyword-benchmark" => keyword_benchmark = true,
            "--with-app-startup" => app_startup = true,
            other => return Err(format!("unsupported argument: {other}")),
        }
    }
    if (keyword_benchmark || app_startup) && mode != Mode::Run {
        return Err("opt-in classes only apply to a run".to_owned());
    }
    Ok(Invocation {
        mode,
        reset_authorized,
        keyword_benchmark,
        app_startup,
    })
}

/// Child environment for one test target.
///
/// The selected URL is passed only through the environment. Production and
/// libpq variables are removed first so an ambient production endpoint can
/// never reach a test process, and the repository root is exported so a test can
/// locate its own fixtures without depending on the runner's working
/// directory.
#[must_use]
pub fn child_environment(endpoint: &Endpoint) -> Vec<(String, String)> {
    let mut environment = filtered_child_environment();
    environment.push((
        crate::TEST_DATABASE_URL_VAR.to_owned(),
        endpoint.expose().to_owned(),
    ));
    add_workspace_test_environment(&mut environment);
    environment
}

/// Replace a command's inherited environment with the filtered test environment.
pub fn configure_child_command(command: &mut Command, endpoint: &Endpoint) {
    command.env_clear().envs(child_environment(endpoint));
}

fn filtered_child_environment() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(key, _)| {
            key != crate::TEST_DATABASE_URL_VAR && !is_production_or_libpq_variable(key)
        })
        .collect()
}

fn add_workspace_test_environment(environment: &mut Vec<(String, String)>) {
    let repo_root = crate::repo_root();
    environment.push(("SQLX_OFFLINE".to_owned(), "true".to_owned()));
    if let Some(value) = repo_root.to_str() {
        environment.push(("CONTEXT69_TEST_REPO_ROOT".to_owned(), value.to_owned()));
    }
}

fn configure_workspace_test_command(command: &mut Command) {
    let mut environment = filtered_child_environment();
    add_workspace_test_environment(&mut environment);
    command.env_clear().envs(environment);
}

fn is_production_or_libpq_variable(key: &str) -> bool {
    key == "DATABASE_URL"
        || key.starts_with("PG")
        || key.starts_with("CONTEXT69_APP__")
        || key.starts_with("CLOUDIFUL_")
}
