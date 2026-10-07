//! Coverage guard for task SQL files (issue 702 P5).
//!
//! Every statement under `src/sql/db/tasks` must be reached by a
//! `sqlx::query_file*!` or `include_str!` call-site, so an unreferenced task SQL
//! file cannot accumulate. The guard is deliberately read-only and independent
//! of the generated `.sqlx` cache: it lexes the Rust sources themselves, not
//! `query-<hash>.json` names, so a deleted file with a leftover cache entry
//! still fails here.
//!
//! `parser` recognizes a macro invocation and reads only its dedicated path
//! argument; `collector` walks the tree and resolves that argument against its
//! real base (crate root or containing source file); `cases` unit-tests both,
//! including the hard-failure collector path.

#[path = "task_sql_reference_guard/parser.rs"]
mod parser;

#[path = "task_sql_reference_guard/collector.rs"]
mod collector;

#[path = "task_sql_reference_guard/cases.rs"]
mod cases;

use std::path::Path;

use collector::{collect_by_extension, normalize_lexical, references_task_sql};

/// Task SQL files intentionally kept without a Rust call-site; each entry must
/// name why it is a test, migration, or build artifact. Empty because every
/// statement under `src/sql/db/tasks` is reached by a macro.
const UNREFERENCED_TASK_SQL: &[&str] = &[];

#[test]
fn every_task_sql_file_is_referenced_by_a_macro() {
    // The `.sqlx` cache is generated from `query_file*!`, so a stale
    // `query-<hash>.json` entry can outlive the call-site it was generated for.
    // Resolving each macro path against its real base keeps this check
    // independent of that cache, so a deleted file with a leftover entry still
    // fails here. The collector and every source read fail hard instead of
    // shrinking the inventory.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_dir = root.join("src");
    let mut source_paths = Vec::new();
    collect_by_extension(&src_dir, "rs", &mut source_paths);
    collect_by_extension(&root.join("tests"), "rs", &mut source_paths);
    // The source path is kept beside its contents: `include_str!` resolves
    // relative to the file, `query_file*!` relative to the crate root.
    let sources = source_paths
        .into_iter()
        .map(|path| {
            let contents = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read source {}: {error}", path.display()));
            (path, contents)
        })
        .collect::<Vec<_>>();

    let mut sql_files = Vec::new();
    collect_by_extension(&root.join("src/sql/db/tasks"), "sql", &mut sql_files);
    let mut missing = Vec::new();
    for file in sql_files {
        let target = normalize_lexical(&file);
        let rel = file
            .strip_prefix(&src_dir)
            .expect("task SQL lives under src")
            .to_string_lossy()
            .replace('\\', "/");
        if UNREFERENCED_TASK_SQL.contains(&rel.as_str()) {
            continue;
        }
        let reached = sources
            .iter()
            .any(|(path, contents)| references_task_sql(root, path, contents, &target));
        if !reached {
            missing.push(file.to_string_lossy().into_owned());
        }
    }
    assert!(
        missing.is_empty(),
        "task SQL files with no `query_file*!`/`include_str!` call-site: {missing:#?}"
    );
}

/// Negative control for `every_task_sql_file_is_referenced_by_a_macro`.
///
/// The live inventory is fully referenced, so a guard that never reported
/// anything would look identical to a working one. This drives the same
/// collector and resolver over a synthetic tree holding one reachable and one
/// orphaned statement, plus a `tests/` comment that only names the orphan. No
/// repository path is written; the tree lives in the process temporary
/// directory and is removed before the assertions run.
#[test]
fn the_task_sql_guard_reports_a_genuinely_orphaned_statement() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the unix epoch")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "context69-sql-guard-{}-{nonce}",
        std::process::id()
    ));
    let sql_dir = base.join("src/sql/db/tasks");
    let probe_dir = base.join("src/db/tasks");
    let noise_dir = base.join("tests");
    std::fs::create_dir_all(&sql_dir).unwrap();
    std::fs::create_dir_all(&probe_dir).unwrap();
    std::fs::create_dir_all(&noise_dir).unwrap();
    std::fs::write(sql_dir.join("referenced.sql"), "SELECT 1\n").unwrap();
    std::fs::write(sql_dir.join("orphan.sql"), "SELECT 2\n").unwrap();
    std::fs::write(
        probe_dir.join("probe.rs"),
        "let rows = sqlx::query_file!(\"src/sql/db/tasks/referenced.sql\", id);\n",
    )
    .unwrap();
    std::fs::write(
        noise_dir.join("noise.rs"),
        "// src/sql/db/tasks/orphan.sql is named here but never called\n",
    )
    .unwrap();

    let mut source_paths = Vec::new();
    collect_by_extension(&base.join("src"), "rs", &mut source_paths);
    collect_by_extension(&noise_dir, "rs", &mut source_paths);
    let sources = source_paths
        .into_iter()
        .map(|path| {
            let contents = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read source {}: {error}", path.display()));
            (path, contents)
        })
        .collect::<Vec<_>>();
    let mut sql_files = Vec::new();
    collect_by_extension(&sql_dir, "sql", &mut sql_files);
    let unreferenced = sql_files
        .iter()
        .filter(|file| {
            let target = normalize_lexical(file);
            !sources
                .iter()
                .any(|(path, contents)| references_task_sql(&base, path, contents, &target))
        })
        .map(|file| file.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let inventory = sql_files.len();
    std::fs::remove_dir_all(&base).unwrap();

    assert_eq!(inventory, 2, "the synthetic inventory must not be empty");
    assert_eq!(
        unreferenced,
        vec!["orphan.sql".to_string()],
        "the guard must flag the unreferenced statement and only that one"
    );

    let mut live = Vec::new();
    collect_by_extension(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sql/db/tasks"),
        "sql",
        &mut live,
    );
    assert!(
        !live.is_empty(),
        "the live task SQL inventory must not be empty, or the coverage assertion passes vacuously"
    );
}
