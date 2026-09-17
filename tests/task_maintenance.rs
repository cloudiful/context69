//! Structural regression coverage for queue-only recovery statements
//! (issue 391 Task 1: automatic task-history cleanup removed, so no
//! retention/purge SQL remains to guard; issue 446 P3: stale `submitting`
//! quarantine chain removed, so no quarantine SQL remains to guard).

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


