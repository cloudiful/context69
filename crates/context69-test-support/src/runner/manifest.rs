//! The checked-in classification of database-backed tests.
//!
//! The manifest is the runner's single inventory of which test functions need
//! the configured database and which are opt-ins. It is reconciled against the
//! compiled test list before anything runs, so a stale or partial inventory
//! fails loudly instead of letting an ignored test report as passing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Why a test function is classified the way it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestClass {
    /// Needs the configured PostgreSQL target and an initialized schema.
    Db,
    /// Opt-in keyword benchmark; never part of the ordinary database run.
    Benchmark,
    /// Opt-in application startup service boot; never part of the ordinary
    /// database run.
    AppStartup,
}

impl TestClass {
    /// Whether this class participates in the ordinary database run.
    #[must_use]
    pub fn runs_by_default(self) -> bool {
        matches!(self, Self::Db)
    }
}

/// One classified test function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub package: String,
    pub target: String,
    pub test: String,
    pub class: TestClass,
}

impl ManifestEntry {
    /// `<package>-<target>` label of the owning target.
    #[must_use]
    pub fn target_label(&self) -> String {
        format!("{}-{}", self.package, self.target)
    }
}

/// The full inventory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestManifest {
    #[serde(default)]
    pub entries: Vec<ManifestEntry>,
}

/// One test function reported by a compiled test executable's `--list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedTest {
    pub name: String,
    pub ignored: bool,
}

/// Every test a single compiled test executable reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetListing {
    pub label: String,
    pub tests: Vec<ListedTest>,
}

/// Why a manifest no longer matches the compiled test list.
#[derive(Debug, PartialEq, Eq)]
pub enum CoverageError {
    /// The manifest names the same test function twice.
    DuplicateEntry(String),
    /// A classified test function is absent from its target's compiled list.
    NotCompiled(String),
    /// A classified test function is not marked `#[ignore]`, so the default
    /// `cargo test --workspace` would run it without a configured database.
    NotIgnored(String),
    /// A compiled test is marked `#[ignore]` but has no class, so the manifest
    /// is incomplete.
    Unclassified(String),
    /// The manifest classifies nothing at all.
    EmptyInventory,
}

impl std::fmt::Display for CoverageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateEntry(label) => write!(f, "manifest lists {label} more than once"),
            Self::NotCompiled(label) => write!(f, "manifest entry {label} is not compiled"),
            Self::NotIgnored(label) => write!(
                f,
                "test {label} is classified but not #[ignore], so the default run would \
                 execute it without a database"
            ),
            Self::Unclassified(label) => {
                write!(
                    f,
                    "compiled test {label} is ignored but has no manifest class"
                )
            }
            Self::EmptyInventory => f.write_str("the manifest classifies no database tests"),
        }
    }
}

impl std::error::Error for CoverageError {}

/// Per-class counts derived from a successful reconciliation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CoverageReport {
    pub targets: usize,
    pub database_tests: usize,
    pub benchmark_tests: usize,
    pub app_startup_tests: usize,
    pub ignored_tests: usize,
    pub pure_tests: usize,
    /// Targets whose every compiled test is database-backed.
    ///
    /// Such a target contributes nothing to a default `cargo test --workspace`,
    /// which is a property of the suite rather than a manifest defect. It is
    /// reported so the inventory exception stays visible instead of silently
    /// shrinking the default suite.
    pub targets_without_pure_coverage: Vec<String>,
}

impl TestManifest {
    /// Reconcile the inventory against the compiled per-target listings.
    ///
    /// # Errors
    ///
    /// Returns the first [`CoverageError`] describing why the inventory is
    /// stale, incomplete, or would suppress a target's pure tests.
    pub fn reconcile(&self, listings: &[TargetListing]) -> Result<CoverageReport, CoverageError> {
        if self.entries.is_empty() {
            return Err(CoverageError::EmptyInventory);
        }

        let mut compiled: BTreeMap<&str, BTreeMap<&str, bool>> = BTreeMap::new();
        for listing in listings {
            let tests = listing
                .tests
                .iter()
                .map(|item| (item.name.as_str(), item.ignored))
                .collect();
            compiled.insert(listing.label.as_str(), tests);
        }

        let mut seen: Vec<(String, String)> = Vec::new();
        for entry in &self.entries {
            let key = (entry.target_label(), entry.test.clone());
            if seen.contains(&key) {
                return Err(CoverageError::DuplicateEntry(entry.test.clone()));
            }
            seen.push(key);
        }

        let mut per_target: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        let mut report = CoverageReport {
            targets: listings.len(),
            ..CoverageReport::default()
        };
        for listing in listings {
            report.ignored_tests += listing.tests.iter().filter(|t| t.ignored).count();
            report.pure_tests += listing.tests.iter().filter(|t| !t.ignored).count();
            per_target.insert(listing.label.as_str(), (0, listing.tests.len()));
        }

        for entry in &self.entries {
            let label = entry.target_label();
            let tests = compiled
                .get(label.as_str())
                .ok_or_else(|| CoverageError::NotCompiled(label.clone()))?;
            if !tests.contains_key(entry.test.as_str()) {
                return Err(CoverageError::NotCompiled(format!(
                    "{}::{}",
                    label, entry.test
                )));
            }
            let counts = per_target.get_mut(label.as_str()).expect("listed above");
            match entry.class {
                TestClass::Db => {
                    report.database_tests += 1;
                    counts.0 += 1;
                }
                TestClass::Benchmark => report.benchmark_tests += 1,
                TestClass::AppStartup => report.app_startup_tests += 1,
            }
        }

        // A target whose compiled tests are all database-backed contributes
        // nothing to the default suite. That is a property of the suite, so it
        // is reported as an inventory exception rather than refused.
        report.targets_without_pure_coverage = per_target
            .iter()
            .filter(|(_, (database, total))| *total > 0 && *database == *total)
            .map(|(label, _)| (*label).to_owned())
            .collect();

        for entry in &self.entries {
            let label = entry.target_label();
            let tests = compiled
                .get(label.as_str())
                .expect("validated while collecting counts");
            if !tests[entry.test.as_str()] {
                return Err(CoverageError::NotIgnored(entry.test.clone()));
            }
        }

        for listing in listings {
            for item in &listing.tests {
                if !item.ignored {
                    continue;
                }
                let classified = self
                    .entries
                    .iter()
                    .any(|entry| entry.target_label() == listing.label && entry.test == item.name);
                if !classified {
                    return Err(CoverageError::Unclassified(item.name.clone()));
                }
            }
        }

        Ok(report)
    }

    /// Test names to pass after `--ignored` for `target_label`, restricted to
    /// `classes`. An empty result means the target holds no test of those
    /// classes and must not be selected at all.
    #[must_use]
    pub fn selection(&self, target_label: &str, classes: &[TestClass]) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.target_label() == target_label && classes.contains(&entry.class))
            .map(|entry| entry.test.clone())
            .collect()
    }

    /// Every target label the manifest touches, in sorted order.
    #[must_use]
    pub fn target_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self
            .entries
            .iter()
            .map(ManifestEntry::target_label)
            .collect();
        labels.sort();
        labels.dedup();
        labels
    }

    /// How many tests the ordinary run plus the requested opt-ins select.
    #[must_use]
    pub fn database_test_count(&self, opt_ins: &[TestClass]) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.class.runs_by_default() || opt_ins.contains(&entry.class))
            .count()
    }
}
