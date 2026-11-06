//! The catalog projection the destructive reset admits connections from.
//!
//! `crates/context69-test-support/src/sql/foreign_backends.sql` is the
//! observation boundary of a reset: the admission can only classify a session
//! from what this query returns. `reset_guards` exercises the decision with
//! constructed `ForeignBackend` values, so this is the only guard that the SQL
//! actually produces them: a masked or incomplete session and a prepared
//! transaction must never look like an idle one. The guard is static, so it
//! needs no database.

fn projection() -> String {
    include_str!("../src/sql/foreign_backends.sql")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A session is visible only when the catalog fully describes it: a missing or
/// masked query, a missing backend start, a non-client backend and a null state
/// all collapse the row to `details_visible = false`, so a masked backend can
/// never be taken as idle. Prepared transactions are projected as `prepared`
/// rows that are never idle, scoped to the database being reset.
#[test]
fn connection_catalog_projection_keeps_masked_and_prepared_sessions_blocking() {
    let sql = projection();

    // Masked visibility: every condition the admission reads is required, and
    // an unknown value falls back to "not visible".
    assert!(
        sql.contains("query <> '<insufficient privilege>'"),
        "a masked query must not count as visible"
    );
    assert!(
        sql.contains("query IS NOT NULL"),
        "an absent query must not count as visible"
    );
    assert!(
        sql.contains("backend_start IS NOT NULL"),
        "an absent backend start must not count as visible"
    );
    assert!(
        sql.contains("backend_type = 'client backend'"),
        "only a client backend is a resettable session"
    );
    assert!(
        sql.contains("state IS NOT NULL"),
        "an unknown state must not count as visible"
    );
    assert!(
        sql.contains(", false) AS details_visible"),
        "the visibility default must stay false"
    );
    // The transaction start is projected as text, so the admission can see an
    // open transaction on a visible session.
    assert!(
        sql.contains("xact_start::text AS xact_start"),
        "the projection must expose the transaction start"
    );

    // Prepared transactions are synthesized as non-idle rows of this database.
    assert!(
        sql.contains("FROM pg_prepared_xacts"),
        "prepared transactions must be projected"
    );
    assert!(
        sql.contains("WHERE database = current_database()"),
        "only prepared transactions of the reset database are in scope"
    );
    assert!(
        sql.contains("'prepared', prepared::text, false"),
        "a prepared transaction is never visible or idle"
    );

    // The query content is used only for visibility, never returned.
    assert!(
        !sql.contains("SELECT query"),
        "query text must not be projected"
    );
}
