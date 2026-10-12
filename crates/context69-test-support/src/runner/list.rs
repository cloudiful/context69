//! `--list`: reconcile the checked-in database-test inventory against the
//! compiled test binaries.
//!
//! Nothing here opens a database connection. The mode answers one question:
//! does the inventory still describe the tests that actually compile?

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::COMPILE_TIMEOUT;
use super::manifest::{TargetListing, TestManifest};
use super::output::{RunDir, RunStatus, RunSummary};
use super::process;
use super::targets::{self, TestExecutable};

/// Where the checked-in inventory lives inside the support crate.
pub const MANIFEST_PATH: &str = "crates/context69-test-support/tests/database-tests.json";

/// How long one test binary may spend listing its tests.
const LIST_TIMEOUT: Duration = Duration::from_secs(120);

/// Build the target inventory and reconcile it against the manifest.
///
/// # Errors
///
/// Returns a redacted message when discovery or reconciliation fails.
pub fn report(repo_root: &Path, run_dir: &RunDir, summary: &mut RunSummary) -> Result<(), String> {
    // The workspace build is itself a child tree; adopting its orphans keeps the
    // invariant that nothing the runner started outlives the run directory.
    super::process_signal::enable_child_subreaper()
        .map_err(|error| format!("could not adopt child orphans: {error}"))?;
    let manifest = manifest(repo_root)?;
    let packages = workspace_packages(repo_root, run_dir)?;
    let executables = compile_and_discover(repo_root, run_dir, &packages)?;

    summary.test_executables = executables.len();
    summary.doctest_packages = targets::doctest_packages(&packages).len();
    summary.record(format!("discovery:test-executables={}", executables.len()));
    summary.record(format!(
        "discovery:doctest-packages={}",
        summary.doctest_packages
    ));

    let mut listings = Vec::with_capacity(executables.len());
    for executable in &executables {
        listings.push(listing_for(executable)?);
    }

    match manifest.reconcile(&listings) {
        Ok(report) => {
            summary.database_tests = report.database_tests;
            summary.record(format!("manifest:database-tests={}", report.database_tests));
            summary.record(format!(
                "manifest:benchmark-tests={}",
                report.benchmark_tests
            ));
            summary.record(format!(
                "manifest:app-startup-tests={}",
                report.app_startup_tests
            ));
            summary.record(format!("manifest:ignored-tests={}", report.ignored_tests));
            summary.record(format!("manifest:pure-tests={}", report.pure_tests));
            summary.status = RunStatus::Passed;
            Ok(())
        }
        Err(error) => {
            summary.record(format!("refused:manifest:{error}"));
            summary.failures.push(error.to_string());
            summary.status = RunStatus::Refused;
            Ok(())
        }
    }
}

/// Read the checked-in inventory.
///
/// # Errors
///
/// Returns a message when the file is missing or malformed.
pub fn manifest(repo_root: &Path) -> Result<TestManifest, String> {
    let path = repo_root.join(MANIFEST_PATH);
    let body = std::fs::read_to_string(&path)
        .map_err(|_| format!("the database-test inventory at {MANIFEST_PATH} could not be read"))?;
    serde_json::from_str(&body)
        .map_err(|error| format!("the database-test inventory is malformed: {error}"))
}

/// Discover the compiled test executables of the workspace.
///
/// # Errors
///
/// Returns a redacted message when metadata or the test build fails.
pub fn executables(repo_root: &Path, run_dir: &RunDir) -> Result<Vec<TestExecutable>, String> {
    let packages = workspace_packages(repo_root, run_dir)?;
    compile_and_discover(repo_root, run_dir, &packages)
}

/// Count the workspace packages that contribute doctests.
///
/// # Errors
///
/// Returns a redacted message when metadata fails.
pub fn doctest_packages(repo_root: &Path, run_dir: &RunDir) -> Result<usize, String> {
    Ok(targets::doctest_packages(&workspace_packages(repo_root, run_dir)?).len())
}

fn workspace_packages(
    repo_root: &Path,
    run_dir: &RunDir,
) -> Result<Vec<targets::WorkspacePackage>, String> {
    let mut command = Command::new("cargo");
    command
        .current_dir(repo_root)
        .args(["metadata", "--format-version", "1", "--no-deps"]);
    let outcome = process::run_logged(&mut command, Duration::from_secs(300))
        .map_err(|error| format!("cargo metadata could not be run: {error}"))?;
    if !outcome.succeeded() {
        let _ = std::fs::write(run_dir.path().join("metadata.log"), &outcome.stderr);
        return Err("cargo metadata failed".to_owned());
    }
    targets::workspace_packages(&outcome.stdout)
        .map_err(|error| format!("cargo metadata output was unreadable: {error}"))
}

fn compile_and_discover(
    repo_root: &Path,
    run_dir: &RunDir,
    packages: &[targets::WorkspacePackage],
) -> Result<Vec<TestExecutable>, String> {
    let mut command = Command::new("cargo");
    command.current_dir(repo_root).args([
        "test",
        "--workspace",
        "--all-targets",
        "--features",
        "integration-test-helpers",
        "--no-run",
        "--message-format=json",
    ]);
    let outcome = process::run_logged(&mut command, COMPILE_TIMEOUT)
        .map_err(|error| format!("the workspace test build could not be run: {error}"))?;
    if !outcome.succeeded() {
        let _ = std::fs::write(run_dir.path().join("compile-tests.log"), &outcome.stderr);
        return Err("workspace test compilation failed".to_owned());
    }
    targets::parse_test_executables(&outcome.stdout, packages).map_err(|error| error.to_string())
}

fn listing_for(executable: &TestExecutable) -> Result<TargetListing, String> {
    let all = terse_listing(executable, false)?;
    let ignored = terse_listing(executable, true)?;
    Ok(TargetListing {
        label: executable.label(),
        tests: targets::pair_listing(&all, &ignored),
    })
}

fn terse_listing(executable: &TestExecutable, only_ignored: bool) -> Result<Vec<String>, String> {
    let mut command = Command::new(&executable.path);
    command.arg("--list").arg("--format").arg("terse");
    if only_ignored {
        command.arg("--ignored");
    }
    let outcome =
        process::run_logged(&mut command, LIST_TIMEOUT).map_err(|error| error.to_string())?;
    if !outcome.succeeded() {
        return Err(format!("{} could not list its tests", executable.label()));
    }
    Ok(targets::parse_terse_listing(&outcome.stdout))
}
