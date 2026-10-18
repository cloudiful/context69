//! The per-target reset, migrate, and execute sequence.
//!
//! One session advisory lock is held for the whole sequence, every target uses
//! the same resolved database, and each target starts from a freshly recreated
//! owned schema with the real migrator applied once — not once per test
//! function. A failing target stops the run: later targets are not launched on
//! top of an unknown state.

use std::path::{Path, PathBuf};
use std::process::Command;

use sqlx::postgres::PgPool;

use crate::config::{Endpoint, Redactor};

use super::TARGET_TIMEOUT;
use super::manifest::{TestClass, TestManifest};
use super::output::{TargetOutcome, TargetStage, write_log};
use super::preflight::{self, PreflightObservation, ResetDecision};
use super::process::{self, ChildOutcome};
use super::reset::{self, ResetAuthorization};
use super::targets::TestExecutable;

/// One target and the exact test names it must run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTarget {
    pub label: String,
    pub tests: Vec<String>,
    /// Apply only the chain ending immediately before the pinned retirement
    /// migration, so the upgrade-recovery target observes a real pending
    /// migration instead of a fabricated ledger.
    pub initialize_to: Option<i64>,
}

/// Build the ordered plan from a manifest.
///
/// Only classes explicitly requested are selected, so a disabled opt-in is
/// skipped rather than run and counted as a pass.
#[must_use]
pub fn plan(manifest: &TestManifest, opt_ins: &[TestClass]) -> Vec<PlannedTarget> {
    let classes: Vec<TestClass> = std::iter::once(TestClass::Db)
        .chain(opt_ins.iter().copied())
        .collect();
    manifest
        .target_labels()
        .into_iter()
        .filter_map(|label| {
            let tests = manifest.selection(&label, &classes);
            let initialize_to = classes
                .contains(&TestClass::UpgradeRecovery)
                .then_some(reset::UPGRADE_RECOVERY_PRIOR_VERSION)
                .filter(|_| {
                    !manifest
                        .selection(&label, &[TestClass::UpgradeRecovery])
                        .is_empty()
                });
            (!tests.is_empty()).then_some(PlannedTarget {
                label,
                tests,
                initialize_to,
            })
        })
        .collect()
}

/// Build arguments that select exactly the requested ignored tests with libtest.
///
/// Libtest treats each positional filter as one exact test name, while
/// `--exact` is a single boolean option for the whole filter list.
///
/// # Errors
///
/// Returns an error when no tests were selected, preventing an unfiltered run.
pub fn test_arguments(test_names: &[String]) -> Result<Vec<String>, &'static str> {
    if test_names.is_empty() {
        return Err("no tests were selected for this target");
    }

    let mut arguments = Vec::with_capacity(test_names.len() + 3);
    arguments.push("--ignored".to_owned());
    arguments.push("--test-threads=1".to_owned());
    arguments.push("--exact".to_owned());
    arguments.extend(test_names.iter().cloned());
    Ok(arguments)
}

/// Run every planned target in order, stopping at the first failure.
///
/// # Errors
///
/// Returns a redacted message only when the runner's own plumbing fails. A
/// target failure is reported through the returned outcomes.
pub async fn run(
    run_dir: &Path,
    endpoint: &Endpoint,
    observation: &PreflightObservation,
    decision: &ResetDecision,
    executables: &[TestExecutable],
    manifest: &TestManifest,
    opt_ins: &[TestClass],
) -> Result<Vec<TargetOutcome>, String> {
    let redactor = endpoint.redactor();
    let mut outcomes = Vec::new();
    for entry in plan(manifest, opt_ins) {
        let Some(outcome) = execute_target(
            run_dir,
            endpoint,
            observation,
            decision,
            executables,
            &entry,
            &redactor,
        )
        .await?
        else {
            break;
        };
        let should_stop = outcome.failure.is_some();
        outcomes.push(outcome);
        if should_stop {
            break;
        }
    }
    Ok(outcomes)
}

/// Execute one target. `Ok(None)` means the target failed and the run must stop.
///
/// # Errors
///
/// Returns a redacted message when the runner's own plumbing fails.
pub async fn execute_target(
    run_dir: &Path,
    endpoint: &Endpoint,
    observation: &PreflightObservation,
    decision: &ResetDecision,
    executables: &[TestExecutable],
    entry: &PlannedTarget,
    redactor: &Redactor,
) -> Result<Option<TargetOutcome>, String> {
    let Some(executable) = locate(executables, &entry.label) else {
        return Ok(Some(stalled(
            &entry.label,
            TargetStage::Unreachable,
            "no compiled test executable was discovered for this target",
        )));
    };
    if super::cancellation::requested() {
        return Ok(Some(stalled(
            &entry.label,
            TargetStage::Unreachable,
            "the run was interrupted before this target started",
        )));
    }

    if let Err(error) =
        reset_and_migrate(endpoint, observation, decision, entry.initialize_to).await
    {
        return Ok(Some(stalled(&entry.label, error.0, &error.1)));
    }

    let outcome = invoke(executable, endpoint, entry)?;
    let _ = write_log(
        run_dir,
        &format!("{}.log", entry.label),
        &format!("{} --ignored --test-threads=1", entry.label),
        &outcome,
        redactor,
    );

    // A child that outlived its process group, or an orphan the kernel
    // reparented here and could not reap, may still hold the resolved URL. The
    // run must not be reported as clean in that state, and the advisory lock is
    // only released after this function returns, which is deliberately later.
    let failure = if outcome.interrupted {
        Some("the target was interrupted after its process tree was stopped".to_owned())
    } else if outcome.leaked_descendants {
        Some(
            "the target left descendant processes running; they were stopped and reaped".to_owned(),
        )
    } else if outcome.timed_out {
        Some("the target exceeded its execution timeout".to_owned())
    } else {
        unsafe_termination(&outcome).or_else(|| {
            (!outcome.succeeded())
                .then(|| format!("the target exited with {}", outcome.exit_label()))
        })
    };

    Ok(Some(TargetOutcome {
        target: entry.label.clone(),
        exit_code: outcome.exit_code,
        timed_out: outcome.timed_out,
        elapsed_seconds: outcome.elapsed.as_secs_f64(),
        stage: if outcome.succeeded() {
            TargetStage::Complete
        } else {
            TargetStage::Test
        },
        log: format!("{}/{}.log", run_dir.display(), entry.label),
        failure,
    }))
}

/// Describe an un-terminatable process group, if the child left one behind.
fn unsafe_termination(outcome: &ChildOutcome) -> Option<String> {
    let termination = outcome.termination?;
    if termination.is_safe() {
        return None;
    }
    Some(format!(
        "a descendant survived SIGKILL and may still hold the resolved database; \
         the run is blocked until it is gone ({} adopted children still running)",
        termination.adopted_still_running
    ))
}

/// Recreate the owned schema and apply every migration once.
async fn reset_and_migrate(
    endpoint: &Endpoint,
    observation: &PreflightObservation,
    decision: &ResetDecision,
    initialize_to: Option<i64>,
) -> Result<(), (TargetStage, String)> {
    let migration_namespace = preflight::migration_namespace(observation).ok_or((
        TargetStage::Reset,
        "the effective migration namespace was not verified".to_owned(),
    ))?;
    let authorization = ResetAuthorization::new(decision.clone(), true);
    if let Err(error) = reset::reset_owned_schema(endpoint, authorization, observation).await {
        return Err((
            TargetStage::Reset,
            format!("the owned-schema reset did not complete: {error}"),
        ));
    }
    let migration_options = preflight::migration_connect_options(endpoint, migration_namespace)
        .map_err(|error| {
            (
                TargetStage::Migrate,
                format!("could not set the verified migration namespace: {error}"),
            )
        })?;
    let pool = PgPool::connect_with(migration_options)
        .await
        .map_err(|error| {
            (
                TargetStage::Migrate,
                format!("could not connect to the verified migration namespace: {error}"),
            )
        })?;
    let migrated = match initialize_to {
        Some(target) => reset::initialize_schema_to(&pool, target).await,
        None => reset::initialize_schema(&pool).await,
    };
    pool.close().await;
    migrated.map_err(|error| {
        (
            TargetStage::Migrate,
            format!("migrations did not apply: {error}"),
        )
    })
}

/// Build the child command. The URL reaches the test only through the
/// environment; it is never an argument.
fn invoke(
    executable: &Path,
    endpoint: &Endpoint,
    entry: &PlannedTarget,
) -> Result<ChildOutcome, String> {
    let arguments = test_arguments(&entry.tests)
        .map_err(|error| format!("could not select tests for {}: {error}", entry.label))?;
    let mut command = Command::new(executable);
    command.args(arguments);
    super::configure_child_command(&mut command, endpoint);
    process::run_logged(&mut command, TARGET_TIMEOUT)
        .map_err(|error| format!("could not start {}: {}", entry.label, error.kind()))
}

fn locate<'a>(executables: &'a [TestExecutable], label: &str) -> Option<&'a PathBuf> {
    executables
        .iter()
        .find(|candidate| candidate.label() == label)
        .map(|candidate| &candidate.path)
}

fn stalled(label: &str, stage: TargetStage, reason: &str) -> TargetOutcome {
    TargetOutcome {
        target: label.to_owned(),
        exit_code: None,
        timed_out: false,
        elapsed_seconds: 0.0,
        stage,
        log: String::new(),
        failure: Some(reason.to_owned()),
    }
}
