//! Structural regression coverage for terminal-task maintenance SQL.

fn normalized_sql(path: &str) -> String {
    path.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_active_external_job_guard(sql: &str) {
    assert!(sql.contains("candidate.status IN ('succeeded', 'failed', 'cancelled')"));
    assert!(sql.contains(
        "AND NOT EXISTS ( SELECT 1 FROM context69.task_items item JOIN context69.task_external_jobs job ON job.item_id = item.id WHERE item.task_id = candidate.id AND job.status IN ('submitting', 'pending', 'running') )"
    ));
    assert!(sql.contains("FOR UPDATE SKIP LOCKED"));
}

#[test]
fn terminal_maintenance_excludes_tasks_with_active_external_jobs() {
    let cleanup_sql = normalized_sql(include_str!("../src/sql/db/tasks/cleanup_expired.sql"));
    let purge_sql = normalized_sql(include_str!("../src/sql/db/tasks/purge_terminal.sql"));

    assert_active_external_job_guard(&cleanup_sql);
    assert_active_external_job_guard(&purge_sql);
    assert!(cleanup_sql.contains("COALESCE(candidate.finished_at, candidate.updated_at) < $1"));
}
