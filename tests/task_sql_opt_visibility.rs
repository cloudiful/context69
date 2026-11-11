//! Issue 734 visibility regressions for the optimized `list.sql`/`count.sql`.
//!
//! These SQL-text units pin the hoisted accessible-groups CTE, the page-first
//! list shape, and the metadata-free count.

const LIST_SQL: &str = include_str!("../src/sql/db/tasks/list.sql");
const COUNT_SQL: &str = include_str!("../src/sql/db/tasks/count.sql");

#[test]
fn optimized_visibility_computes_groups_once_without_duplicates() {
    for sql in [LIST_SQL, COUNT_SQL] {
        assert!(
            sql.contains("WITH RECURSIVE accessible_groups"),
            "visibility must compute accessible groups once"
        );
        assert!(
            sql.contains("FROM accessible_groups ag"),
            "tasks must filter against the hoisted set"
        );
        assert!(
            !sql.contains("inherited_groups"),
            "per-row inherited_groups CTE must be gone"
        );
    }
    assert!(
        LIST_SQL.contains("WITH RECURSIVE accessible_groups") && LIST_SQL.contains("page AS"),
        "list must keep the page-first shape"
    );
    assert!(
        !COUNT_SQL.contains("LATERAL"),
        "count must stay metadata-free"
    );
}
