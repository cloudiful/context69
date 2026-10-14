//! Run artifacts and the redacted summary the runner reports.
//!
//! Everything written under the run directory passes through
//! [`crate::config::Redactor`] first, and the summary itself only carries the
//! opaque endpoint fingerprint plus action counts.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Redactor;

/// Terminal state of one runner invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Preflight or lock admission refused the run before any reset.
    Refused,
    /// A test target failed, timed out, or could not be started.
    Failed,
    /// Every selected target completed.
    Passed,
}

/// Where a target stopped. Reported instead of a raw error string so a failure
/// never has to be parsed back out of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetStage {
    /// The owned-schema reset did not complete.
    Reset,
    /// The migrator did not apply.
    Migrate,
    /// The test executable failed or timed out.
    Test,
    /// The target finished successfully.
    Complete,
    /// The runner could not start the target at all.
    Unreachable,
}

/// One executed target, reduced to counts and a redacted failure note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetOutcome {
    pub target: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub elapsed_seconds: f64,
    pub stage: TargetStage,
    pub log: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

impl TargetOutcome {
    /// Whether the target completed without a failure.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.failure.is_none() && self.stage == TargetStage::Complete
    }

    /// Exit status label, or the stage when the target never ran.
    #[must_use]
    pub fn exit_label(&self) -> String {
        self.exit_code
            .map_or_else(|| "none".to_owned(), |code| code.to_string())
    }

    /// Attach a redacted failure reason.
    #[must_use]
    pub fn with_failure(mut self, reason: String) -> Self {
        self.failure = Some(reason);
        self
    }
}

/// Redacted, machine-readable record of a runner invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub status: RunStatus,
    pub endpoint_fingerprint: Option<String>,
    pub endpoint_source: Option<String>,
    pub test_executables: usize,
    pub doctest_packages: usize,
    pub database_tests: usize,
    pub allow_idle_connections: bool,
    pub permitted_idle_connections: usize,
    pub actions: Vec<String>,
    pub outcomes: Vec<TargetOutcome>,
    pub failures: Vec<String>,
}

impl RunSummary {
    /// Start a summary for `run_id` with no endpoint resolved yet.
    #[must_use]
    pub fn new(run_id: String) -> Self {
        Self {
            run_id,
            status: RunStatus::Refused,
            endpoint_fingerprint: None,
            endpoint_source: None,
            test_executables: 0,
            doctest_packages: 0,
            database_tests: 0,
            allow_idle_connections: false,
            permitted_idle_connections: 0,
            actions: Vec::new(),
            outcomes: Vec::new(),
            failures: Vec::new(),
        }
    }

    /// Record one completed action. The text must already be secret-free.
    pub fn record(&mut self, action: impl Into<String>) {
        self.actions.push(action.into());
    }

    /// Serialize the summary next to the run's logs.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem or serialization error.
    pub fn write(&self, run_dir: &Path) -> io::Result<()> {
        let rendered = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(run_dir.join("summary.json"), format!("{rendered}\n"))
    }
}

/// A run's artifact directory under the repository target directory.
#[derive(Debug, Clone)]
pub struct RunDir {
    path: PathBuf,
}

impl RunDir {
    /// Create a fresh directory named after `run_id`.
    ///
    /// Run directories older than [`RETENTION`] are removed first, so the
    /// generated artifact tree stays bounded. A concurrent run's directory is
    /// younger than the window and is never touched.
    ///
    /// # Errors
    ///
    /// Returns the underlying filesystem error.
    pub fn create(repo_root: &Path, run_id: &str) -> io::Result<Self> {
        let root = repo_root.join("target").join("backend-test-results");
        fs::create_dir_all(&root)?;
        prune_expired(&root, run_id);
        let path = root.join(run_id);
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Repository-relative path, used in log references and summaries.
    #[must_use]
    pub fn relative_label(&self, repo_root: &Path) -> String {
        self.path
            .strip_prefix(repo_root)
            .unwrap_or(&self.path)
            .display()
            .to_string()
    }
}

/// Write one child process transcript after redaction.
pub fn write_log(
    run_dir: &Path,
    file_name: &str,
    command_label: &str,
    outcome: &super::process::ChildOutcome,
    redactor: &Redactor,
) -> io::Result<()> {
    let body = format!(
        "$ {command_label}\nexit: {}\nelapsed: {:.3}s\n--- stdout ---\n{}\n--- stderr ---\n{}\n",
        outcome.exit_label(),
        outcome.elapsed.as_secs_f64(),
        outcome.stdout,
        outcome.stderr,
    );
    fs::write(run_dir.join(file_name), redactor.redact(&body))
}

/// How long a finished run's artifacts are kept.
const RETENTION: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

fn prune_expired(root: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(RETENTION)
        .unwrap_or(std::time::UNIX_EPOCH);
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == keep {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        if metadata.modified().is_ok_and(|modified| modified < cutoff) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}
