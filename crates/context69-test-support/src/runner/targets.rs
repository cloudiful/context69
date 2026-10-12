//! Discovery of the workspace's compiled test executables.
//!
//! Discovery mirrors the compile-first flow the backend suite already uses:
//! `cargo metadata` supplies the workspace packages and `cargo test --no-run`
//! with `--message-format=json` supplies the executable of every test target.
//! Both readers are pure functions over the command output, so the discovery
//! contract is unit-testable without a database.

use std::path::PathBuf;

use serde::Deserialize;

use super::manifest::ListedTest;

/// A workspace package as far as target discovery needs to know.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspacePackage {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub targets: Vec<PackageTarget>,
}

/// One Cargo target inside a [`WorkspacePackage`].
#[derive(Debug, Clone, Deserialize)]
pub struct PackageTarget {
    pub name: String,
    #[serde(default)]
    pub kind: Vec<String>,
    #[serde(default)]
    pub test: bool,
    #[serde(default)]
    pub doctest: bool,
}

/// A compiled test binary produced by `cargo test --no-run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestExecutable {
    pub package: String,
    pub target: String,
    pub kinds: Vec<String>,
    pub path: PathBuf,
}

impl TestExecutable {
    /// Stable `<package>-<target>` label used in manifests and summaries.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}-{}", self.package, self.target)
    }
}

/// Why the compiled artifact stream could not be turned into a target list.
#[derive(Debug)]
pub enum DiscoveryError {
    /// The artifact stream named a test target without an executable path.
    MissingExecutable(String),
    /// A test target in the workspace never appeared in the artifact stream.
    IncompleteDiscovery(Vec<String>),
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingExecutable(label) => {
                write!(f, "test target has no executable: {label}")
            }
            Self::IncompleteDiscovery(labels) => {
                write!(
                    f,
                    "test artifact discovery was incomplete: {}",
                    labels.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    #[serde(default)]
    package_id: Option<String>,
    #[serde(default)]
    target: Option<ArtifactTarget>,
    #[serde(default)]
    profile: Option<ArtifactProfile>,
    #[serde(default)]
    executable: Option<String>,
}

#[derive(Deserialize)]
struct ArtifactTarget {
    name: String,
    #[serde(default)]
    kind: Vec<String>,
}

#[derive(Deserialize)]
struct ArtifactProfile {
    #[serde(default)]
    test: bool,
}

#[derive(Deserialize)]
struct CargoMetadata {
    workspace_members: Vec<String>,
    packages: Vec<WorkspacePackage>,
}

/// Keep only the workspace's own packages.
///
/// # Errors
///
/// Returns the JSON error when `cargo metadata` output cannot be decoded.
pub fn workspace_packages(metadata_json: &str) -> Result<Vec<WorkspacePackage>, serde_json::Error> {
    let metadata: CargoMetadata = serde_json::from_str(metadata_json)?;
    let members: std::collections::HashSet<&str> = metadata
        .workspace_members
        .iter()
        .map(String::as_str)
        .collect();
    Ok(metadata
        .packages
        .into_iter()
        .filter(|package| members.contains(package.id.as_str()))
        .collect())
}

/// Packages that contribute doctests.
#[must_use]
pub fn doctest_packages(packages: &[WorkspacePackage]) -> Vec<String> {
    let mut names: Vec<String> = packages
        .iter()
        .filter(|package| {
            package
                .targets
                .iter()
                .any(|target| target.doctest && target.kind.iter().any(|kind| kind == "lib"))
        })
        .map(|package| package.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Parse `cargo test --no-run --message-format=json` output into the compiled
/// test executables of `packages`, sorted by label.
///
/// Binary targets are left out. A package may have a `lib` and a `bin` of the
/// same name, and an integration test may share a name with a `bin`; both would
/// otherwise collide on one `<package>-<target>` label and one of the two
/// listings would be silently discarded. No bin test binary in this workspace
/// declares a test function.
///
/// # Errors
///
/// Returns [`DiscoveryError`] when an artifact is unusable or when a declared
/// test target never produced an executable. A silently short list would let a
/// target's tests be reported as covered when they never ran.
pub fn parse_test_executables(
    cargo_output: &str,
    packages: &[WorkspacePackage],
) -> Result<Vec<TestExecutable>, DiscoveryError> {
    let expected: Vec<(String, String)> = packages
        .iter()
        .flat_map(|package| {
            package
                .targets
                .iter()
                .filter(|target| is_test_suite(&target.kind))
                .map(move |target| (package.name.clone(), target.name.clone()))
        })
        .collect();

    let mut found: Vec<TestExecutable> = Vec::new();
    for line in cargo_output.lines() {
        let Some(message) = parse_message(line) else {
            continue;
        };
        if message.reason != "compiler-artifact" {
            continue;
        }
        let (Some(target), Some(profile)) = (message.target, message.profile) else {
            continue;
        };
        if !is_test_suite(&target.kind) || !profile.test {
            continue;
        }
        let Some(package) = packages
            .iter()
            .find(|package| Some(package.id.as_str()) == message.package_id.as_deref())
        else {
            continue;
        };
        let Some(executable) = message.executable.filter(|path| !path.is_empty()) else {
            return Err(DiscoveryError::MissingExecutable(format!(
                "{}-{}",
                package.name, target.name
            )));
        };
        found.push(TestExecutable {
            package: package.name.clone(),
            target: target.name.clone(),
            kinds: target.kind,
            path: PathBuf::from(executable),
        });
    }

    found.sort_by(|left, right| {
        left.package
            .cmp(&right.package)
            .then(left.target.cmp(&right.target))
    });
    found.dedup_by(|left, right| left.package == right.package && left.target == right.target);

    let missing: Vec<String> = expected
        .iter()
        .filter(|(package, target)| {
            !found
                .iter()
                .any(|item| item.package == *package && item.target == *target)
        })
        .map(|(package, target)| format!("{package}:{target}"))
        .collect();
    if !missing.is_empty() {
        return Err(DiscoveryError::IncompleteDiscovery(missing));
    }
    Ok(found)
}

/// Whether a target kind names a library or integration test suite.
///
/// A `bin` is excluded because it shares its name with the package's library
/// and, in this workspace, with integration tests of the same name.
#[must_use]
fn is_test_suite(kind: &[String]) -> bool {
    kind.iter().any(|entry| entry == "lib" || entry == "test")
}

fn parse_message(line: &str) -> Option<CargoMessage> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

/// Parse one `--list --format terse` stream into bare test names.
///
/// libtest emits `<name>: test` per line, so the separator is the only
/// structure available.
#[must_use]
pub fn parse_terse_listing(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            line.split_once(": ")
                .map(|(name, _)| name.trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// Pair the names a target lists with the subset libtest reports under
/// `--ignored`, which is how a `#[ignore]` attribute becomes observable without
/// executing the target.
#[must_use]
pub fn pair_listing(all_names: &[String], ignored_names: &[String]) -> Vec<ListedTest> {
    all_names
        .iter()
        .map(|name| ListedTest {
            name: name.clone(),
            ignored: ignored_names.contains(name),
        })
        .collect()
}
