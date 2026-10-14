//! Orchestration of the three runner modes.
//!
//! Each mode reports through [`RunSummary`] and writes its artifacts under the
//! repository target directory. The run mode passes the ordinary pure Cargo
//! suite before resolving or admitting any database work; only it can reach the
//! destructive reset, after a passing preflight, an explicitly held advisory
//! lock, and the recorded checkpoint authorization.

use std::path::Path;

use crate::config::{Endpoint, resolve_default};

use super::cancellation;
use super::execute;
use super::list;
use super::lock::{self, Admission, SessionLock};
use super::manifest::TestClass;
use super::output::{RunDir, RunStatus, RunSummary};
use super::preflight::{self, PreflightObservation, ResetDecision};
use super::process_signal;
use super::targets::TestExecutable;
use super::{Invocation, Mode};

/// What the read-only inspection found.
pub struct PreflightReport {
    pub observation: PreflightObservation,
    pub decision: ResetDecision,
    pub owned_objects: usize,
    pub preserved_extension_count: usize,
}

impl PreflightReport {
    #[must_use]
    pub fn allowed(&self) -> bool {
        self.decision.allowed
    }

    #[must_use]
    pub fn refusals(&self) -> &[preflight::Refusal] {
        &self.decision.refusals
    }
}

/// Execute one invocation and return its redacted summary.
///
/// # Errors
///
/// Returns a redacted message when the run cannot be started or its artifacts
/// cannot be written. A refusal is a normal result, not an error.
pub async fn execute(
    repo_root: &Path,
    run_id: &str,
    invocation: Invocation,
) -> Result<RunSummary, String> {
    cancellation::install_signal_handlers()
        .map_err(|error| format!("could not install safe cancellation handlers: {error}"))?;
    let mut summary = RunSummary::new(run_id.to_owned());
    summary.allow_idle_connections = invocation.allow_idle_connections;
    summary.record(format!(
        "policy:allow-idle-connections={}",
        invocation.allow_idle_connections
    ));
    let run_dir = RunDir::create(repo_root, run_id).map_err(|error| error.to_string())?;

    match invocation.mode {
        Mode::List => {
            if let Err(error) = list::report(repo_root, &run_dir, &mut summary) {
                summary.failures.push(error.clone());
                summary.status = RunStatus::Refused;
                summary.record(format!("refused:{error}"));
            }
            finish(&summary, &run_dir)
        }
        Mode::Preflight => {
            inspect(&mut summary, invocation.allow_idle_connections).await?;
            finish(&summary, &run_dir)
        }
        Mode::Run => {
            let run_dir_ref = &run_dir;
            let invocation_ref = &invocation;
            let summary_ref = &mut summary;
            let result = super::pure::run_before_database(
                || super::pure::run_workspace_tests(repo_root, run_dir_ref),
                |outcome| async move {
                    summary_ref.record(super::pure::outcome_action(&outcome));
                    inspect(summary_ref, invocation_ref.allow_idle_connections).await?;
                    if summary_ref.status != RunStatus::Refused {
                        run_targets(repo_root, run_dir_ref, summary_ref, invocation_ref).await?;
                    }
                    Ok(())
                },
            )
            .await;

            match result {
                Ok(()) => {}
                Err(super::pure::GateFailure::PureStart(error)) => {
                    summary.record("pure:workspace-tests=failed-to-start");
                    summary.failures.push(error);
                    summary.status = RunStatus::Failed;
                }
                Err(super::pure::GateFailure::PureFailed(outcome)) => {
                    summary.record(super::pure::outcome_action(&outcome));
                    summary.failures.push(
                        "ordinary workspace tests failed before database admission".to_owned(),
                    );
                    summary.status = RunStatus::Failed;
                }
                Err(super::pure::GateFailure::Database(error)) => {
                    summary.failures.push(error);
                    summary.status = RunStatus::Failed;
                }
            }
            finish(&summary, &run_dir)
        }
    }
}

/// Resolve the target, run the read-only inspection, and record the verdict.
async fn inspect(summary: &mut RunSummary, allow_idle_connections: bool) -> Result<(), String> {
    let target = resolve_default().map_err(|error| error.to_string())?;
    let endpoint = target.endpoint();
    summary.endpoint_fingerprint = Some(endpoint.fingerprint());
    summary.endpoint_source = Some(target.source().label().to_owned());

    let report = preflight_report(endpoint, allow_idle_connections).await?;
    record_idle_policy(summary, &report.observation, allow_idle_connections);
    summary.record(format!("preflight:owned-objects={}", report.owned_objects));
    summary.record(format!(
        "preflight:preserved-extensions={}",
        report.preserved_extension_count
    ));
    if report.allowed() {
        summary.record("preflight:owned-schema-is-resettable");
        summary.status = RunStatus::Passed;
        return Ok(());
    }
    for refusal in report.refusals() {
        summary.record(format!("refused:{refusal:?}"));
        summary.failures.push(refusal.to_string());
    }
    summary.status = RunStatus::Refused;
    Ok(())
}

/// Hold the advisory lock, then reset, migrate, and execute every planned
/// target serially against the one resolved database.
async fn run_targets(
    repo_root: &Path,
    run_dir: &RunDir,
    summary: &mut RunSummary,
    invocation: &Invocation,
) -> Result<(), String> {
    let allow_idle_connections = invocation.allow_idle_connections;
    if cancellation::requested() {
        summary
            .failures
            .push("the run was interrupted before acquiring the database lock".to_owned());
        summary.status = RunStatus::Failed;
        return Ok(());
    }
    let target = resolve_default().map_err(|error| error.to_string())?;
    let endpoint = target.endpoint();

    // Orphans of every child are reparented here, so a descendant that left the
    // child's process group is still waitable and cannot outlive the lock.
    process_signal::enable_child_subreaper()
        .map_err(|error| format!("could not adopt child orphans: {error}"))?;

    if !invocation.reset_authorized {
        summary
            .failures
            .push("reset is not authorized by a recorded security checkpoint".to_owned());
        summary.status = RunStatus::Refused;
        return Ok(());
    }

    let guard: SessionLock = lock::SessionLock::acquire(endpoint)
        .await
        .map_err(|error| format!("could not take the target database lock: {error}"))?;
    if matches!(
        lock::admit(Some(&guard)),
        Admission::RefuseAnotherRunnerActive
    ) {
        summary
            .failures
            .push(Admission::RefuseAnotherRunnerActive.to_string());
        summary.status = RunStatus::Refused;
        return Ok(());
    }
    summary.record("lock:acquired");

    let manifest = list::manifest(repo_root)?;
    let executables = list::executables(repo_root, run_dir)?;
    summary.test_executables = executables.len();
    summary.doctest_packages = list::doctest_packages(repo_root, run_dir)?;
    summary.database_tests = manifest.entries.len();

    let opt_ins = requested_opt_ins(invocation);
    let planned = execute::plan(&manifest, &opt_ins);
    summary.record(format!("plan:targets={}", planned.len()));
    summary.record(format!(
        "plan:tests={}",
        manifest.database_test_count(&opt_ins)
    ));

    let report = preflight_report(endpoint, allow_idle_connections).await?;
    record_idle_policy(summary, &report.observation, allow_idle_connections);
    let outcomes = execute::run(
        run_dir.path(),
        endpoint,
        &report.observation,
        &report.decision,
        &executables,
        &manifest,
        &opt_ins,
    )
    .await?;

    for outcome in &outcomes {
        summary.record(format!(
            "target:{}:{:?}:exit={}",
            outcome.target,
            outcome.stage,
            outcome.exit_label()
        ));
        if let Some(failure) = &outcome.failure {
            summary
                .failures
                .push(format!("{}: {failure}", outcome.target));
        }
    }
    summary.outcomes = outcomes;
    if cancellation::requested() && summary.failures.is_empty() {
        summary
            .failures
            .push("the run was interrupted after its active child exited".to_owned());
    }
    summary.status = if summary.failures.is_empty() {
        summary.record("sequence:complete");
        RunStatus::Passed
    } else {
        RunStatus::Failed
    };
    Ok(())
}

/// Map the opt-in flags to the manifest classes they select.
fn requested_opt_ins(invocation: &Invocation) -> Vec<TestClass> {
    let mut classes = Vec::new();
    if invocation.keyword_benchmark {
        classes.push(TestClass::Benchmark);
    }
    if invocation.app_startup {
        classes.push(TestClass::AppStartup);
    }
    classes
}

/// Inspect the target catalog without writing anything.
async fn preflight_report(
    endpoint: &Endpoint,
    allow_idle_connections: bool,
) -> Result<PreflightReport, String> {
    let observation = preflight::collect(endpoint)
        .await
        .map_err(|error| format!("preflight failed: {error}"))?;
    let decision =
        preflight::decide_reset_with_policy(&observation, endpoint, allow_idle_connections);
    Ok(PreflightReport {
        owned_objects: observation.owned_objects.total(),
        preserved_extension_count: observation.preserved_extension_count,
        observation,
        decision,
    })
}

fn record_idle_policy(summary: &mut RunSummary, observation: &PreflightObservation, enabled: bool) {
    summary.permitted_idle_connections = if enabled {
        observation
            .foreign_backends
            .iter()
            .filter(|backend| backend.is_visible_idle())
            .count()
    } else {
        0
    };
    summary.record(format!(
        "preflight:permitted-idle-connections={}",
        summary.permitted_idle_connections
    ));
}

fn finish(summary: &RunSummary, run_dir: &RunDir) -> Result<RunSummary, String> {
    summary
        .write(run_dir.path())
        .map_err(|error| error.to_string())?;
    Ok(summary.clone())
}

/// Re-exported so the binary reports the reset boundary from one place.
pub use super::reset::RESET_SQL;

/// Keeps the discovery type visible to the runner's callers.
pub type Executables = Vec<TestExecutable>;
