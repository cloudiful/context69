//! Structural regression coverage for terminal-task maintenance SQL and the
//! phase 4 queue-only recovery / quarantine statements.

fn normalized_sql(path: &str) -> String {
    // Strip `--` line comments so assertions only observe executable SQL.
    path.lines()
        .map(|line| line.split_once("--").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

#[test]
fn orphaned_jobs_do_not_block_terminal_cleanup() {
    // `orphaned` is a real non-active status: it must never appear in the
    // blocking set, otherwise quarantined history would wedge cleanup again.
    for sql in [
        normalized_sql(include_str!("../src/sql/db/tasks/cleanup_expired.sql")),
        normalized_sql(include_str!("../src/sql/db/tasks/purge_terminal.sql")),
    ] {
        assert!(
            !sql.contains("orphaned"),
            "cleanup guards must not mention orphaned; only submitting/pending/running block"
        );
    }
}

#[test]
fn queue_recovery_requeues_without_attempt_or_remote_job() {
    let sql = normalized_sql(include_str!(
        "../src/sql/db/tasks/queue_docling_recovery.sql"
    ));
    // Persists back to the scheduling queue.
    assert!(sql.contains("SET status = 'queued'"));
    assert!(sql.contains("stage = 'docling'"));
    // Never mints an attempt or a remote job. The validation may READ the
    // latest external job to enforce the active-job boundary, but no
    // statement may write attempts or jobs.
    assert!(
        !sql.contains("INSERT INTO context69.task_attempts"),
        "queue-only recovery must not insert an attempt row"
    );
    assert!(
        !sql.contains("attempt_count + 1") && !sql.contains("attempt_count= attempt_count + 1"),
        "queue-only recovery must not bump attempt_count"
    );
    assert!(
        !sql.contains("UPDATE context69.task_external_jobs")
            && !sql.contains("INSERT INTO context69.task_external_jobs"),
        "queue-only recovery must not write external jobs"
    );
    // Repeat calls are idempotent.
    assert!(sql.contains("THEN 'already_queued'"));
    // Shares the immediate-recovery safety boundaries.
    for reason in [
        "'task_terminal'",
        "'lease_active'",
        "'item_terminal'",
        "'active_external_job'",
        "'uncertain_submission'",
        "'dependency_waiting'",
        "'missing_file'",
        "'no_docling_item'",
    ] {
        assert!(
            sql.contains(reason),
            "queue SQL must keep boundary {reason}"
        );
    }
}

#[test]
fn immediate_recovery_and_supersede_never_cancel_uncertain_submissions() {
    let recover_sql = normalized_sql(include_str!("../src/sql/db/tasks/recover_docling_item.sql"));
    assert!(
        recover_sql.contains("THEN 'uncertain_submission'"),
        "immediate recovery must reject submitting rows instead of claiming them"
    );
    let supersede_sql = normalized_sql(include_str!(
        "../src/sql/library_store/external_jobs/mark_external_job_superseded.sql"
    ));
    assert!(
        supersede_sql.contains("WHEN job.status IN ('pending', 'running') THEN 'cancelled'"),
        "supersede must only cancel live remote states"
    );
    assert!(
        !supersede_sql.contains("'submitting', 'pending', 'running'"),
        "supersede must never mark an uncertain submitting row as cancelled"
    );
}

#[test]
fn quarantine_only_isolates_eligible_stale_submitting_rows() {
    let sql = normalized_sql(include_str!(
        "../src/sql/library_store/external_jobs/quarantine_stale_submitting.sql"
    ));
    assert!(sql.contains("SET status = 'orphaned'"));
    assert!(sql.contains("AND job.status = 'submitting'"));
    assert!(sql.contains("AND job.remote_task_id LIKE $4"));
    assert!(sql.contains("AND job.submitted_at < $3"));
    assert!(sql.contains("AND item.status IN ('succeeded', 'failed', 'cancelled')"));
    assert!(sql.contains("AND task.status IN ('succeeded', 'failed', 'cancelled')"));
    // Live remote jobs are never candidates.
    assert!(
        !sql.contains("'pending'") && !sql.contains("'running'"),
        "quarantine must not reference live remote states"
    );
    // History is preserved and audited, never overwritten with a fake cancel.
    assert!(sql.contains("|| ' | quarantined: ' || $1"));
    assert!(sql.contains("INSERT INTO context69.task_external_job_quarantine_audit"));
    assert!(sql.contains("FOR UPDATE OF job SKIP LOCKED"));
}

#[test]
fn quarantine_stats_partition_every_submitting_row() {
    let sql = normalized_sql(include_str!(
        "../src/sql/library_store/external_jobs/quarantine_submitting_stats.sql"
    ));
    for column in [
        "\"uncertain_submitting_count!\"",
        "\"quarantinable_count!\"",
        "\"skipped_non_terminal_count!\"",
        "\"skipped_fresh_count!\"",
        "\"skipped_real_remote_count!\"",
        "\"orphaned_count!\"",
    ] {
        assert!(
            sql.contains(column),
            "quarantine stats must report {column}"
        );
    }
}

#[test]
fn maintenance_stats_reports_quarantine_counters() {
    let sql = normalized_sql(include_str!("../src/sql/db/tasks/maintenance_stats.sql"));
    for column in [
        "\"uncertain_submitting_count!\"",
        "\"quarantinable_submitting_count!\"",
        "\"orphaned_external_job_count!\"",
    ] {
        assert!(
            sql.contains(column),
            "maintenance stats must report {column}"
        );
    }
}
