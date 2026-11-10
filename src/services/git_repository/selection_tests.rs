//! File eligibility and skip-summary tests (issue #28 phase P3).

use super::{
    SelectionDecision, SelectionRules, SkipReason, glob_matches, select_snapshot_files,
    selection_decision,
};
use crate::db::MAX_GIT_FILE_BYTES;
use crate::services::git_repository::model::TreeFileEntry;
use crate::services::git_repository::ref_path_safety::{SafeSha, SafeTreePath};

fn entry(path: &str, size: Option<u64>) -> TreeFileEntry {
    TreeFileEntry {
        path: SafeTreePath::parse(path).expect("valid test path"),
        blob_sha: SafeSha::parse(&"ab".repeat(20)).expect("valid test sha"),
        size,
    }
}

fn decision(path: &str) -> SelectionDecision {
    selection_decision(path, None, &SelectionRules::default())
}

fn included(path: &str) -> bool {
    decision(path) == SelectionDecision::Include
}

fn skip(path: &str) -> SkipReason {
    match decision(path) {
        SelectionDecision::Skip(reason) => reason,
        SelectionDecision::Include => panic!("{path} must be skipped"),
    }
}

#[test]
fn default_allowlist_admits_sources_documents_and_key_manifests() {
    for path in [
        "src/main.rs",
        "web/app.ts",
        "web/app.tsx",
        "web/app.js",
        "web/app.jsx",
        "db/schema.sql",
        "README.md",
        "docs/guide.mdx",
        "docs/guide.rst",
        "docs/notes.txt",
        "Cargo.toml",
        "web/package.json",
    ] {
        assert!(included(path), "{path} must be eligible by default");
    }
}

#[test]
fn default_allowlist_rejects_unlisted_textual_files() {
    assert_eq!(skip("config.yml"), SkipReason::NotIncluded);
    assert_eq!(skip("data/table.json"), SkipReason::NotIncluded);
    assert_eq!(skip("script.py"), SkipReason::NotIncluded);
    assert_eq!(skip("LICENSE"), SkipReason::NotIncluded);
    assert_eq!(skip("Cargo.toml.bak"), SkipReason::NotIncluded);
}

#[test]
fn dependency_build_and_vcs_directories_are_never_admitted() {
    for path in [
        ".git/config",
        "node_modules/pkg/index.js",
        "target/debug/build.rs",
        "dist/bundle.js",
        "build/generated.rs",
        "web/.next/app.ts",
        "service/.venv/lib.py",
        "__pycache__/module.rs",
    ] {
        assert_eq!(skip(path), SkipReason::ExcludedDirectory, "{path}");
    }
}

#[test]
fn binary_minified_map_lock_and_secret_files_are_skipped() {
    assert_eq!(skip("assets/logo.png"), SkipReason::Binary);
    assert_eq!(skip("web/app.min.js"), SkipReason::Minified);
    assert_eq!(skip("web/app.js.map"), SkipReason::SourceMap);
    assert_eq!(skip("Cargo.lock"), SkipReason::Lockfile);
    assert_eq!(skip("web/package-lock.json"), SkipReason::Lockfile);
    assert_eq!(skip("web/pnpm-lock.yaml"), SkipReason::Lockfile);
    assert_eq!(skip(".env"), SkipReason::Secret);
    assert_eq!(skip("config/.env.production"), SkipReason::Secret);
    assert_eq!(skip("certs/server.pem"), SkipReason::Secret);
    assert_eq!(skip("deploy/id_ed25519"), SkipReason::Secret);
}

#[test]
fn declared_size_above_the_storage_ceiling_is_skipped() {
    let oversized = MAX_GIT_FILE_BYTES as u64 + 1;
    assert_eq!(
        selection_decision("src/main.rs", Some(oversized), &SelectionRules::default()),
        SelectionDecision::Skip(SkipReason::TooLarge)
    );
    assert_eq!(
        selection_decision(
            "src/main.rs",
            Some(MAX_GIT_FILE_BYTES as u64),
            &SelectionRules::default()
        ),
        SelectionDecision::Include
    );
}

#[test]
fn user_include_extends_the_default_allowlist() {
    let rules = SelectionRules::new(vec!["**/*.py".to_string()], Vec::new());
    assert_eq!(
        selection_decision("tools/run.py", None, &rules),
        SelectionDecision::Include
    );
    assert_eq!(
        selection_decision("src/main.rs", None, &rules),
        SelectionDecision::Include,
        "the default allowlist still applies"
    );
    assert_eq!(
        selection_decision("data/table.json", None, &rules),
        SelectionDecision::Skip(SkipReason::NotIncluded)
    );
}

#[test]
fn user_exclude_wins_over_both_defaults_and_includes() {
    let rules = SelectionRules::new(
        vec!["docs/**".to_string()],
        vec!["docs/private/**".to_string()],
    );
    assert_eq!(
        selection_decision("docs/guide.md", None, &rules),
        SelectionDecision::Include
    );
    assert_eq!(
        selection_decision("docs/private/secret.md", None, &rules),
        SelectionDecision::Skip(SkipReason::ExcludedByUser)
    );
    assert_eq!(
        selection_decision("src/main.rs", None, &rules),
        SelectionDecision::Include
    );
}

#[test]
fn user_patterns_cannot_re_enable_safety_exclusions() {
    let rules = SelectionRules::new(vec!["**".to_string()], vec!["src/**".to_string()]);
    assert_eq!(
        selection_decision("node_modules/pkg/index.js", None, &rules),
        SelectionDecision::Skip(SkipReason::ExcludedDirectory)
    );
    assert_eq!(
        selection_decision("assets/logo.png", None, &rules),
        SelectionDecision::Skip(SkipReason::Binary)
    );
    assert_eq!(
        selection_decision("src/main.rs", None, &rules),
        SelectionDecision::Skip(SkipReason::ExcludedByUser)
    );
}

#[test]
fn selection_summary_counts_each_skip_reason() {
    let outcome = select_snapshot_files(
        vec![
            entry("src/main.rs", None),
            entry("README.md", None),
            entry("assets/logo.png", None),
            entry("Cargo.lock", None),
            entry("script.py", None),
        ],
        &SelectionRules::default(),
    );
    assert_eq!(
        outcome
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/main.rs", "README.md"]
    );
    assert_eq!(outcome.skipped.count(SkipReason::Binary), 1);
    assert_eq!(outcome.skipped.count(SkipReason::Lockfile), 1);
    assert_eq!(outcome.skipped.count(SkipReason::NotIncluded), 1);
    assert_eq!(outcome.skipped.total(), 3);
    assert!(!outcome.skipped.is_empty());
    assert_eq!(outcome.skipped.count(SkipReason::Secret), 0);
}

#[test]
fn glob_patterns_match_paths_deterministically() {
    assert!(glob_matches("*.md", "docs/guide.md"));
    assert!(glob_matches("*.md", "README.md"));
    assert!(!glob_matches("*.md", "docs/guide.txt"));
    assert!(glob_matches("src/**/*.rs", "src/main.rs"));
    assert!(glob_matches(
        "src/**/*.rs",
        "src/services/git_repository/selection.rs"
    ));
    assert!(glob_matches("src/**", "src/main.rs"));
    assert!(!glob_matches("src/**", "tests/main.rs"));
    assert!(glob_matches("**/*.test.ts", "web/src/app.test.ts"));
    assert!(glob_matches("**/*.test.ts", "app.test.ts"));
    assert!(glob_matches("web/?.ts", "web/a.ts"));
    assert!(!glob_matches("web/?.ts", "web/ab.ts"));
    assert!(glob_matches("**", "any/deep/path.rs"));
    assert!(!glob_matches("", "src/main.rs"));
}

#[test]
fn skip_reason_tokens_are_stable() {
    assert_eq!(SkipReason::ExcludedDirectory.as_str(), "excluded_directory");
    assert_eq!(SkipReason::Binary.as_str(), "binary");
    assert_eq!(SkipReason::Minified.as_str(), "minified");
    assert_eq!(SkipReason::SourceMap.as_str(), "source_map");
    assert_eq!(SkipReason::Lockfile.as_str(), "lockfile");
    assert_eq!(SkipReason::Secret.as_str(), "secret");
    assert_eq!(SkipReason::TooLarge.as_str(), "too_large");
    assert_eq!(SkipReason::ExcludedByUser.as_str(), "excluded_by_user");
    assert_eq!(SkipReason::NotIncluded.as_str(), "not_included");
}
