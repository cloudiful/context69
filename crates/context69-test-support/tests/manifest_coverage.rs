//! Manifest reconciliation and compiled target discovery.
//!
//! Both readers are pure functions over command output, so the classification
//! contract is verified without building or connecting to anything.

use context69_test_support::runner::manifest::{
    CoverageError, ListedTest, ManifestEntry, TargetListing, TestClass, TestManifest,
};
use context69_test_support::runner::targets;

fn entry(name: &str, class: TestClass) -> ManifestEntry {
    ManifestEntry {
        package: "context69".to_owned(),
        target: "library_storage".to_owned(),
        test: name.to_owned(),
        class,
    }
}

fn listing() -> TargetListing {
    TargetListing {
        label: "context69-library_storage".to_owned(),
        tests: vec![
            ListedTest {
                name: "storage_writes_are_durable".to_owned(),
                ignored: true,
            },
            ListedTest {
                name: "slug_is_normalized".to_owned(),
                ignored: false,
            },
            ListedTest {
                name: "cleanup_is_case_insensitive".to_owned(),
                ignored: true,
            },
            ListedTest {
                name: "keyword_oracle_is_frozen".to_owned(),
                ignored: true,
            },
        ],
    }
}

fn manifest() -> TestManifest {
    TestManifest {
        entries: vec![
            entry("storage_writes_are_durable", TestClass::Db),
            entry("cleanup_is_case_insensitive", TestClass::Db),
            entry("keyword_oracle_is_frozen", TestClass::Benchmark),
        ],
    }
}

#[test]
fn a_current_inventory_reconciles_into_per_class_counts() {
    let report = manifest()
        .reconcile(&[listing()])
        .expect("inventory is current");

    assert_eq!(report.targets, 1);
    assert_eq!(report.database_tests, 2);
    assert_eq!(report.benchmark_tests, 1);
    assert_eq!(report.app_startup_tests, 0);
    assert_eq!(report.ignored_tests, 3);
    assert_eq!(report.pure_tests, 1);
}

#[test]
fn a_duplicate_entry_is_refused() {
    let mut manifest = manifest();
    manifest
        .entries
        .push(entry("storage_writes_are_durable", TestClass::Db));

    assert_eq!(
        manifest.reconcile(&[listing()]),
        Err(CoverageError::DuplicateEntry(
            "storage_writes_are_durable".to_owned()
        ))
    );
}

#[test]
fn a_classified_test_that_is_not_compiled_is_refused() {
    let mut manifest = manifest();
    manifest
        .entries
        .push(entry("never_compiled", TestClass::Db));

    assert!(matches!(
        manifest.reconcile(&[listing()]),
        Err(CoverageError::NotCompiled(_))
    ));
}

#[test]
fn a_classified_test_that_is_not_ignored_is_refused() {
    let mut manifest = manifest();
    manifest
        .entries
        .push(entry("slug_is_normalized", TestClass::Db));

    assert_eq!(
        manifest.reconcile(&[listing()]),
        Err(CoverageError::NotIgnored("slug_is_normalized".to_owned())),
        "a database test without #[ignore] would run in the default workspace suite"
    );
}

#[test]
fn an_ignored_test_without_a_class_is_refused() {
    let manifest = TestManifest {
        entries: vec![entry("storage_writes_are_durable", TestClass::Db)],
    };

    assert_eq!(
        manifest.reconcile(&[listing()]),
        Err(CoverageError::Unclassified(
            "cleanup_is_case_insensitive".to_owned()
        ))
    );
}

#[test]
fn a_target_whose_every_test_is_database_backed_is_reported_as_an_exception() {
    let database_only = TargetListing {
        label: "context69-library_storage".to_owned(),
        tests: vec![
            ListedTest {
                name: "storage_writes_are_durable".to_owned(),
                ignored: true,
            },
            ListedTest {
                name: "cleanup_is_case_insensitive".to_owned(),
                ignored: true,
            },
        ],
    };
    let manifest = TestManifest {
        entries: vec![
            entry("storage_writes_are_durable", TestClass::Db),
            entry("cleanup_is_case_insensitive", TestClass::Db),
        ],
    };

    let report = manifest
        .reconcile(&[database_only])
        .expect("a target without pure tests is not a manifest defect");
    assert_eq!(
        report.targets_without_pure_coverage,
        vec!["context69-library_storage".to_owned()],
        "the target is named so the exception stays visible"
    );
    assert_eq!(report.pure_tests, 0);
}

#[test]
fn an_empty_inventory_is_refused_rather_than_reporting_zero_coverage() {
    assert_eq!(
        TestManifest::default().reconcile(&[listing()]),
        Err(CoverageError::EmptyInventory)
    );
}

#[test]
fn opt_in_classes_never_join_the_ordinary_database_selection() {
    let manifest = manifest();

    assert_eq!(
        manifest.selection("context69-library_storage", &[TestClass::Db]),
        vec![
            "storage_writes_are_durable".to_owned(),
            "cleanup_is_case_insensitive".to_owned()
        ]
    );
    assert_eq!(
        manifest.selection("context69-library_storage", &[TestClass::Benchmark]),
        vec!["keyword_oracle_is_frozen".to_owned()]
    );
    assert!(
        manifest
            .selection("context69-library_storage", &[TestClass::AppStartup])
            .is_empty(),
        "an absent opt-in must yield an empty selection, never a pass"
    );
}

const METADATA: &str = r#"{
  "workspace_members": ["path+file:///repo#root@0.1.0"],
  "packages": [
    {"id": "path+file:///repo#root@0.1.0", "name": "context69",
     "targets": [
       {"name": "context69", "kind": ["bin", "lib"], "test": true, "doctest": true},
       {"name": "library_storage", "kind": ["test"], "test": true, "doctest": false}
     ]},
    {"id": "path+file:///dep#thirdparty@0.1.0", "name": "thirdparty",
     "targets": [{"name": "thirdparty", "kind": ["lib"], "test": true, "doctest": true}]}
  ]
}"#;

fn artifacts(executable: &str) -> String {
    [
        r#"{"reason":"compiler-artifact","package_id":"path+file:///repo#root@0.1.0","target":{"name":"context69","kind":["lib"],"test":true},"profile":{"test":true},"executable":"/tmp/context69"}"#,
        &format!(
            r#"{{"reason":"compiler-artifact","package_id":"path+file:///repo#root@0.1.0","target":{{"name":"library_storage","kind":["test"],"test":true}},"profile":{{"test":true}},"executable":"{executable}"}}"#
        ),
        r#"{"reason":"compiler-artifact","package_id":"path+file:///dep#thirdparty@0.1.0","target":{"name":"thirdparty","kind":["lib"],"test":true},"profile":{"test":true},"executable":"/elsewhere/thirdparty"}"#,
        "this line is not json",
    ]
    .join("\n")
}

#[test]
fn workspace_packages_exclude_dependencies_and_keep_doctest_targets() {
    let packages = targets::workspace_packages(METADATA).expect("metadata parses");

    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "context69");
    assert_eq!(targets::doctest_packages(&packages), vec!["context69"]);
}

#[test]
fn artifact_discovery_is_workspace_scoped_deduplicated_and_complete() {
    let packages = targets::workspace_packages(METADATA).expect("metadata parses");
    let stream = format!(
        "{}\n{}",
        artifacts("/tmp/library_storage"),
        artifacts("/tmp/library_storage")
    );

    let found = targets::parse_test_executables(&stream, &packages).expect("discovery succeeds");

    assert_eq!(found.len(), 2);
    assert_eq!(found[0].label(), "context69-context69");
    assert_eq!(found[1].label(), "context69-library_storage");
    assert!(found.iter().all(|item| item.path.starts_with("/tmp")));
}

#[test]
fn a_declared_test_target_without_an_artifact_is_never_a_short_list() {
    let packages = targets::workspace_packages(METADATA).expect("metadata parses");

    let error = targets::parse_test_executables("", &packages)
        .expect_err("a missing artifact must not look like full coverage");
    assert!(matches!(
        error,
        targets::DiscoveryError::IncompleteDiscovery(_)
    ));
    assert!(error.to_string().contains("context69:library_storage"));
}

#[test]
fn a_test_artifact_without_an_executable_path_is_refused() {
    let packages = targets::workspace_packages(METADATA).expect("metadata parses");

    let error = targets::parse_test_executables(&artifacts(""), &packages)
        .expect_err("an artifact without an executable is unusable");
    assert!(matches!(
        error,
        targets::DiscoveryError::MissingExecutable(_)
    ));
}

#[test]
fn libtest_listing_output_becomes_bare_test_names() {
    let stream = "storage_writes_are_durable: test\nslug_is_normalized: test\n: benchmark\n";

    assert_eq!(
        targets::parse_terse_listing(stream),
        vec![
            "storage_writes_are_durable".to_owned(),
            "slug_is_normalized".to_owned()
        ]
    );
    assert!(targets::parse_terse_listing("").is_empty());
}

#[test]
fn the_ignored_subset_of_a_listing_marks_exactly_those_tests() {
    let all = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    let ignored = vec!["b".to_owned()];

    let paired = targets::pair_listing(&all, &ignored);

    assert_eq!(
        paired
            .iter()
            .map(|item| (item.name.as_str(), item.ignored))
            .collect::<Vec<_>>(),
        vec![("a", false), ("b", true), ("c", false)]
    );
}
