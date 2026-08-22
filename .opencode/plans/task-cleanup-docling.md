# Task Cleanup and Docling Recovery

Status: READY FOR CHECKPOINT
Tracking: LOCAL_PLAN fallback from FORGEJO_ISSUE Redmine issue 2. Redmine issue creation and get worked, but issue update/comment operations did not persist or returned HTTP 404 for comments. This local plan is authoritative from the fallback point.

## Goal

Prevent terminal-task maintenance from deleting task history while an external Docling job may still be recoverable, and document the current production failure disposition without mutating production data.

## Background

The `context69` database has 3,091 tasks, zero active tasks, and 77 records older than the configured one-day retention window. All 77 are successful terminal history, so they are retention-expired history rather than execution timeouts. There are also 2,998 URL tasks created on 2026-08-22 that failed during Docling submission after the connection closed before the response completed. Their external jobs remain in `submitting` with marker IDs, so the submission outcome is uncertain.

## Constraints

- Do not mutate production data or blindly resubmit uncertain Docling requests.
- Do not touch baseline paths: HEAD `1deb913c890c8cad26a0dcb7afb7f3aa2506fd65`; staged, unstaged, and untracked baseline paths were empty.
- Static SQL must remain in SQL files loaded through existing SQLx query-file calls.
- Do not stage or commit `Cargo.lock` or `bun.lock`.

## Acceptance Criteria

1. Expired cleanup and full terminal purge exclude tasks with `submitting`, `pending`, or `running` external jobs.
2. Focused regression coverage protects both maintenance queries.
3. Current 2,998 failures are explained and no unsafe production recovery is performed.
4. Required formatting, tests, diff checks, and independent review pass.

## Phases

1. Investigation: complete. Queried schemas, task status, retention settings, failure groups, external jobs, and deletion constraints.
2. Cleanup protection: in progress. Scope: `src/sql/db/tasks/cleanup_expired.sql`, `src/sql/db/tasks/purge_terminal.sql`, and `tests/task_maintenance.rs`.
3. Independent review and repairs: complete. Reviewer round 1 returned exact `PASS`; no P0-P2 findings. P3 observations only: the structural test is not behavioral, its name overstates coverage, and its single normalized-substring assertion is somewhat brittle. They are non-blocking under the scoped SQL-only change and no repair is required.
4. Checkpoint and final disposition: pending.

## Validation

- `cargo fmt --all -- --check`
- `cargo test --test task_maintenance`
- `git diff --check`
- Broader Cargo checks when feasible.

## Decisions and Findings

- Task-history purge cascades to task items, attempts, and external-job rows, but does not delete library files or documents.
- The current 77 expired records are 76 successful URL tasks and one successful text task.
- The 2,998 current failures are URL tasks, one attempt each, non-retryable Docling submission-uncertain failures; all corresponding files remain in `library_files` with `ingest_status = failed`.
- Do not batch-recover until Docling logs/API confirm whether any uncertain POST requests were accepted. A duplicate submission could create duplicate remote conversions.

## Review History

- Executor Phase 2 attempt 1: DONE. Changed the two purge SQL files and added structural regression coverage. Validation passed: `git diff --check`, `cargo fmt --all -- --check`, `cargo test --test task_maintenance` (1 passed). Redmine audit comment failed with HTTP 404.
- Reviewer round 1: exact `PASS`. Reviewer JSON was recovered after an initial non-JSON wrapper response; final JSON verdict and evidence are valid. No P0-P2 findings. P3 observations recorded above. Validation confirmed `git diff --check`, `cargo fmt --all -- --check`, `cargo test --test task_maintenance` (1 passed), and SQLx call-site compatibility by inspection.
- Tracking fallback: Redmine issue 2 was created, but `comment create` returned HTTP 404 and `issue update-body` did not return the updated body. `.opencode/plans/task-cleanup-docling.md` is authoritative from this point.

## Blocked Questions

- Need Docling-side confirmation for the 2,998 uncertain submissions before any production recovery action.

## Final Status

IN PROGRESS
