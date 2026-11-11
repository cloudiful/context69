//! Issue 734 terminal projection regressions for the optimized
//! `project_file_status.sql`.
//!
//! The optimized statement resolves its target by item id through an indexed
//! file-identity join and evaluates the active-sibling guard once. This SQL-text
//! unit pins the indexed join, the single sibling check, and the group fence.

const PROJECTION_SQL: &str = include_str!("../src/sql/db/tasks/project_file_status.sql");

#[test]
fn optimized_projection_keeps_indexed_join_and_single_sibling_check() {
    assert!(
        PROJECTION_SQL.contains("FROM target"),
        "projection must join the file through the indexed target"
    );
    assert!(
        PROJECTION_SQL.contains("JOIN target"),
        "sibling guard must reuse the same target"
    );
    assert!(
        PROJECTION_SQL.contains("has_sibling"),
        "sibling guard must be evaluated once"
    );
    assert_eq!(
        PROJECTION_SQL.matches("sibling.status IN").count(),
        1,
        "the three repeated sibling checks must be combined into one"
    );
    assert!(
        PROJECTION_SQL.contains("file.group_id = target.group_id"),
        "group ownership fence must be preserved"
    );
    assert!(
        !PROJECTION_SQL.contains("WHERE file.id = ("),
        "outer-file correlated scalar subquery must be gone"
    );
}
