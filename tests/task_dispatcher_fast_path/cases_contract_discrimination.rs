//! Discrimination check for the SQL-contract assertions.
//!
//! A text assertion over SQL is only worth as much as its ability to fail. The
//! attempt-1 fence in `progress_item.sql` was missed because the whole-file
//! predicate `EXISTS (SELECT 1 FROM ` was satisfied by the statement's own final
//! `SELECT EXISTS (SELECT 1 FROM <item CTE>) AS "updated!"`, so it could not
//! distinguish a guarded attempt CTE from an unguarded one.
//!
//! These cases re-derive the slice independently of the contract test's helper
//! and pin the property that matters: the predicate the contract now asserts
//! holds on the real statements and stops holding as soon as the guard is
//! removed, while the old whole-file predicate keeps holding either way.

const FINISH_ITEM: &str = include_str!("../../src/sql/db/tasks/finish_item.sql");
const WAIT_ITEM: &str = include_str!("../../src/sql/db/tasks/wait_item.sql");
const PROGRESS_ITEM: &str = include_str!("../../src/sql/db/tasks/progress_item.sql");

const STATEMENTS: [(&str, &str, &str, &str); 3] = [
    (
        "finish_item.sql",
        FINISH_ITEM,
        "finished",
        "attempt_finished",
    ),
    ("wait_item.sql", WAIT_ITEM, "waiting", "attempt_waited"),
    (
        "progress_item.sql",
        PROGRESS_ITEM,
        "progressed",
        "attempt_progressed",
    ),
];

/// Comment-free copy, so no assertion can be satisfied by prose.
fn code(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Body of one CTE, from its opening parenthesis to the statement's final
/// top-level `SELECT`.
///
/// Requiring a non-identifier character before the CTE name is what keeps
/// `finished` from being found through the tail of `attempt_finished`, and
/// running to the top-level `SELECT` keeps the statement's final
/// `SELECT EXISTS (...)` — the clause that made the old whole-file predicate
/// vacuous — out of the slice.
fn attempt_cte_body(sql: &str, attempt_cte: &str) -> Option<String> {
    let code = code(sql);
    let opening = format!("{attempt_cte} AS (");
    let start = code.match_indices(&opening).find_map(|(index, _)| {
        let starts_a_token = code[..index]
            .chars()
            .next_back()
            .is_none_or(|char| !(char.is_alphanumeric() || char == '_'));
        starts_a_token.then(|| index + opening.len())
    })?;
    let body = &code[start..];
    let end = body.find("\nSELECT ").unwrap_or(body.len());
    Some(body[..end].to_string())
}

/// The same statements with the attempt-side fence removed.
fn without_fence(sql: &str, item_cte: &str) -> String {
    let guard = format!("AND EXISTS (SELECT 1 FROM {item_cte})");
    code(sql)
        .lines()
        .filter(|line| line.trim() != guard)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_attempt_cte_fence_is_asserted_in_every_item_transition() {
    for (name, sql, item_cte, attempt_cte) in STATEMENTS {
        let body = attempt_cte_body(sql, attempt_cte)
            .unwrap_or_else(|| panic!("{name} must contain the {attempt_cte} CTE"));
        assert!(
            body.contains(&format!("AND EXISTS (SELECT 1 FROM {item_cte})")),
            "{name}: {attempt_cte} must gate its attempt close on the item transition"
        );
    }
}

#[test]
fn the_per_cte_predicate_fails_once_the_fence_is_removed() {
    for (name, sql, item_cte, attempt_cte) in STATEMENTS {
        let stripped = without_fence(sql, item_cte);
        assert_ne!(
            stripped,
            code(sql),
            "{name}: the fixture must actually remove the fence"
        );
        let body = attempt_cte_body(&stripped, attempt_cte)
            .unwrap_or_else(|| panic!("{name}: the {attempt_cte} CTE must survive"));
        assert!(
            !body.contains(&format!("AND EXISTS (SELECT 1 FROM {item_cte})")),
            "{name}: the per-CTE predicate must not hold on the unguarded statement"
        );
        // The final `SELECT EXISTS (SELECT 1 FROM <item CTE>) AS "updated!"` is
        // still there, which is exactly why a whole-file match cannot see the
        // difference.
        assert!(
            stripped.contains(&format!(
                "SELECT EXISTS (SELECT 1 FROM {item_cte}) AS \"updated!\""
            )),
            "{name}: the decoy clause must still be present in the stripped copy"
        );
    }
}

#[test]
fn the_contract_helpers_own_predicate_fails_once_the_fence_is_removed() {
    // The contract test slices a CTE with `find("{name} AS (")` and runs to the
    // first top-level `SELECT`. Reproduced here verbatim so the shipped
    // assertion itself is shown to fail on the unguarded statement, not only
    // the equivalent check above.
    fn contract_cte_body(sql: &str, name: &str) -> String {
        let code = code(sql);
        let opening = format!("{name} AS (");
        let start = code
            .find(&opening)
            .unwrap_or_else(|| panic!("{name} CTE must exist"))
            + opening.len();
        let body = &code[start..];
        let end = body.find("\nSELECT ").unwrap_or(body.len());
        body[..end].to_string()
    }

    for (name, sql, item_cte, attempt_cte) in STATEMENTS {
        let guard = format!("AND EXISTS (SELECT 1 FROM {item_cte})");
        let stripped = without_fence(sql, item_cte);
        assert!(
            contract_cte_body(sql, attempt_cte).contains(&guard),
            "{name}: the shipped assertion must hold on the statement as written"
        );
        assert!(
            !contract_cte_body(&stripped, attempt_cte).contains(&guard),
            "{name}: the shipped assertion must fail once the fence is removed"
        );
    }
}

#[test]
fn the_old_whole_file_predicate_cannot_see_the_fence() {
    // Recorded so the vacuous form is never reintroduced as the only check.
    for (name, sql, item_cte, _) in STATEMENTS {
        let stripped = without_fence(sql, item_cte);
        assert!(
            code(&stripped).contains("EXISTS (SELECT 1 FROM "),
            "{name}: the whole-file predicate is satisfied even without the fence"
        );
        assert!(
            code(sql).contains("EXISTS (SELECT 1 FROM "),
            "{name}: and also with it, so it discriminates nothing"
        );
    }
}

#[test]
fn the_item_transition_still_reports_updated_not_the_attempt_rows() {
    // The repair restored the fence; it must not have re-coupled `updated` to
    // the attempt CTE, which is the decoupling this phase exists for.
    for (name, sql, item_cte, attempt_cte) in STATEMENTS {
        let code = code(sql);
        assert!(
            code.contains(&format!(
                "SELECT EXISTS (SELECT 1 FROM {item_cte}) AS \"updated!\""
            )),
            "{name} must report the item transition as `updated`"
        );
        assert!(
            !code.trim_end().ends_with("UPDATE context69.task_attempts"),
            "{name} must not take its row count from the attempt UPDATE"
        );
        assert!(
            !code.contains(&format!("SELECT EXISTS (SELECT 1 FROM {attempt_cte})")),
            "{name}: the reported `updated` must never come from {attempt_cte}"
        );
        let body = attempt_cte_body(sql, attempt_cte)
            .unwrap_or_else(|| panic!("{name} must contain the {attempt_cte} CTE"));
        assert!(
            !body.contains(&format!(
                "SELECT EXISTS (SELECT 1 FROM {item_cte}) AS \"updated!\""
            )),
            "{name}: the reported `updated` must not be produced inside {attempt_cte}"
        );
    }
}
