//! Discriminating unit tests for the task SQL reference guard.
//!
//! These pin that a macro path argument is accepted, that comments, plain
//! strings, an unrelated namespace, later bind arguments, nested `format!`
//! paths, and same-suffix/different-resolution literals are rejected, and that
//! the collector fails hard instead of shrinking the inventory.

use std::path::Path;

use super::collector::{collect_by_extension, normalize_lexical, references_task_sql};

#[test]
fn task_sql_guard_accepts_macro_invocations_and_rejects_lookalikes() {
    let root = Path::new("/repo");
    let target = normalize_lexical(&root.join("src/sql/db/tasks/items.sql"));

    // Accepted: the path argument of a recognized macro resolves exactly to the
    // target. `query_file*!` is crate-root-relative; `include_str!` is relative
    // to the containing source file.
    let accepted = [
        (
            "/repo/src/db/tasks/probe.rs",
            r#"sqlx::query_file!("src/sql/db/tasks/items.sql", task_id)"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"let rows = sqlx::query_file_as!(Row, "src/sql/db/tasks/items.sql", id)"#,
        ),
        (
            "/repo/src/services/tasks/probe.rs",
            r#"const SQL: &str = include_str!("../../sql/db/tasks/items.sql");"#,
        ),
        (
            "/repo/tests/task_dispatcher_fast_path/probe.rs",
            r#"const SQL: &str = include_str!("../../src/sql/db/tasks/items.sql");"#,
        ),
    ];
    for (source_path, source) in accepted {
        assert!(
            references_task_sql(root, Path::new(source_path), source, &target),
            "a real macro invocation must satisfy the guard: {source}"
        );
    }

    // Rejected: comments, plain strings, an unrelated namespace, a later bind
    // string, a non-literal (nested `format!`) path, and a path that merely
    // shares a suffix but resolves elsewhere are not call-sites.
    let rejected = [
        (
            "/repo/src/db/tasks/probe.rs",
            r#"// see src/sql/db/tasks/items.sql"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"/* src/sql/db/tasks/items.sql */"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"let path = "src/sql/db/tasks/items.sql";"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"println!("src/sql/db/tasks/items.sql");"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"fake::query_file!("src/sql/db/tasks/items.sql")"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"sqlx::query_file_as!(Row, "src/sql/db/tasks/live.sql", "src/sql/db/tasks/items.sql")"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"sqlx::query_file!(format!("src/sql/db/tasks/items.sql"), task_id)"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"sqlx::query_file!("vendor/src/sql/db/tasks/items.sql")"#,
        ),
        (
            "/repo/src/db/tasks/probe.rs",
            r#"sqlx::query_file!("src/sql/db/other/items.sql")"#,
        ),
        // Same suffix, wrong resolution: from a `tests/` source the literal
        // climbs only to the repository root, not back down into `src/`.
        (
            "/repo/tests/task_dispatcher_fast_path/probe.rs",
            r#"const SQL: &str = include_str!("../../sql/db/tasks/items.sql");"#,
        ),
        // A source-relative `src/...` literal resolves under the source file's
        // own directory, which is not the crate's task tree.
        (
            "/repo/tests/task_dispatcher_fast_path/probe.rs",
            r#"const SQL: &str = include_str!("src/sql/db/tasks/items.sql");"#,
        ),
    ];
    for (source_path, source) in rejected {
        assert!(
            !references_task_sql(root, Path::new(source_path), source, &target),
            "text alone must never satisfy the guard: {source}"
        );
    }

    // The sibling named as the actual path argument is still reached.
    let live = normalize_lexical(&root.join("src/sql/db/tasks/live.sql"));
    assert!(references_task_sql(
        root,
        Path::new("/repo/src/db/tasks/probe.rs"),
        r#"sqlx::query_file_as!(Row, "src/sql/db/tasks/live.sql", "src/sql/db/tasks/items.sql")"#,
        &live
    ));
}

#[test]
fn task_sql_guard_collector_fails_hard_on_unreadable_directory() {
    // A missing directory must panic with context, not yield an empty
    // inventory that would make the coverage assertion pass vacuously.
    let mut out = Vec::new();
    let missing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        collect_by_extension(
            Path::new("/nonexistent/context69/task-sql-guard-probe"),
            "rs",
            &mut out,
        );
    }));
    assert!(
        missing.is_err(),
        "a missing directory must fail the collector, not be swallowed"
    );

    // A regular file is not a directory; the explicit `read_dir` must fail too.
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut file_out = Vec::new();
    let not_a_dir = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        collect_by_extension(&file, "rs", &mut file_out);
    }));
    assert!(
        not_a_dir.is_err(),
        "reading a file as a directory must fail the collector"
    );
}
