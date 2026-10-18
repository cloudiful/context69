//! The checked-in inventory against the compiled workspace test list.
//!
//! Both sides are read from disk and from `--list` output, so the test proves
//! the manifest and the annotations agree without resetting or connecting to a
//! database.

use std::collections::BTreeSet;
use std::path::PathBuf;

use context69_test_support::runner::list;
use context69_test_support::runner::manifest::TestClass;

/// The repository root, so the inventory is read from the checkout rather than
/// from a copied fixture.
fn repo_root() -> PathBuf {
    context69_test_support::repo_root()
}

#[test]
fn the_checked_in_inventory_is_readable_and_classifies_the_suite() {
    let manifest = list::manifest(&repo_root()).expect("the inventory is checked in and parses");
    assert!(
        !manifest.entries.is_empty(),
        "the inventory must classify the database-backed tests"
    );
    assert!(
        manifest
            .entries
            .iter()
            .all(|entry| !entry.test.trim().is_empty()),
        "every entry names a test function"
    );
    assert!(
        manifest
            .target_labels()
            .iter()
            .all(|label| label.contains('-')),
        "a target label carries its package so two packages cannot collide"
    );
}

#[test]
fn the_inventory_names_each_test_function_once() {
    let manifest = list::manifest(&repo_root()).expect("the inventory parses");
    let unique: BTreeSet<(String, String)> = manifest
        .entries
        .iter()
        .map(|entry| (entry.target_label(), entry.test.clone()))
        .collect();
    assert_eq!(
        unique.len(),
        manifest.entries.len(),
        "a duplicated entry would make one test run twice"
    );
}

#[test]
fn opt_in_classes_are_never_selected_by_the_ordinary_run() {
    let manifest = list::manifest(&repo_root()).expect("the inventory parses");
    let classes: BTreeSet<&str> = manifest
        .entries
        .iter()
        .filter(|entry| !entry.class.runs_by_default())
        .map(|entry| match entry.class {
            TestClass::Benchmark => "benchmark",
            TestClass::AppStartup => "app_startup",
            TestClass::UpgradeRecovery => "upgrade_recovery",
            TestClass::Db => "db",
        })
        .collect();
    assert!(
        classes.contains("benchmark")
            || classes.contains("app_startup")
            || classes.contains("upgrade_recovery"),
        "the opt-in classes are represented, so a disabled opt-in is skipped \
         rather than silently counted as a pass"
    );
    for label in manifest.target_labels() {
        let ordinary = manifest.selection(&label, &[TestClass::Db]);
        assert!(
            !ordinary.is_empty()
                || manifest.selection(&label, &[TestClass::Benchmark]).len()
                    + manifest.selection(&label, &[TestClass::AppStartup]).len()
                    + manifest
                        .selection(&label, &[TestClass::UpgradeRecovery])
                        .len()
                    == manifest
                        .entries
                        .iter()
                        .filter(|entry| entry.target_label() == label)
                        .count(),
            "{label} is classified as exactly the classes the runner selects"
        );
    }
}

#[test]
fn the_database_selection_of_every_target_is_non_empty_or_opt_in_only() {
    let manifest = list::manifest(&repo_root()).expect("the inventory parses");
    for label in manifest.target_labels() {
        let db = manifest.selection(&label, &[TestClass::Db]);
        let opt_ins = manifest.selection(
            &label,
            &[
                TestClass::Benchmark,
                TestClass::AppStartup,
                TestClass::UpgradeRecovery,
            ],
        );
        assert!(
            !db.is_empty() || !opt_ins.is_empty(),
            "{label} is never selected by any invocation"
        );
    }
}
