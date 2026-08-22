# Task Cleanup and Docling Recovery

Status: COMPLETE
Tracking: LOCAL_PLAN fallback from FORGEJO_ISSUE Redmine issue 2. Redmine issue creation and get worked, but issue update/comment operations did not persist or returned HTTP 404 for comments. This archived local plan is the final authoritative record.

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
4. Required formatting, tests, diff checks, offline build, and independent review pass.

## Phases

1. Investigation: complete. Queried schemas, task status, retention settings, failure groups, external jobs, and deletion constraints.
2. Cleanup protection: complete. Changed `src/sql/db/tasks/cleanup_expired.sql`, `src/sql/db/tasks/purge_terminal.sql`, and added `tests/task_maintenance.rs`.
3. Independent review and repairs: complete. Round 1 passed the SQL logic; aggregate round 1 found and repaired the missing SQLx offline caches.
4. Checkpoint and final disposition: complete. Checkpoints are `180173d` and `adea599`; final aggregate round 3 returned exact `PASS`.

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo test --test task_maintenance`: pass, 1 test
- `SQLX_OFFLINE=true cargo check --bin context69`: pass
- `git diff --check`: pass
- Final review confirmed 302 baseline `.sqlx` caches unchanged and exactly two intended cache files added.

## Decisions and Findings

- Task-history purge cascades to task items, attempts, and external-job rows, but does not delete library files or documents.
- The current 77 expired records are 76 successful URL tasks and one successful text task.
- The 2,998 current failures are URL tasks, one attempt each, non-retryable Docling submission-uncertain failures; all corresponding files remain in `library_files` with `ingest_status = failed`.
- Do not batch-recover until Docling logs/API confirm whether any uncertain POST requests were accepted. A duplicate submission could create duplicate remote conversions.
- The cleanup SQL now preserves any terminal task linked to an external job in `submitting`, `pending`, or `running`, so the recovery path cannot be removed by retention cleanup.

## Review History

- Executor Phase 2 attempt 1: DONE. Changed the two purge SQL files and added structural regression coverage. Validation passed: `git diff --check`, `cargo fmt --all -- --check`, `cargo test --test task_maintenance` (1 passed). Redmine audit comment failed with HTTP 404.
- Reviewer round 1: exact `PASS`; no P0-P2 findings. P3 observations: structural rather than behavioral coverage, test name overstates coverage, and one normalized assertion is brittle.
- Final aggregate reviewer round 1: exact `FAIL`. Confirmed P1: changed SQL queries had no matching committed `.sqlx` offline cache entries.
- Repair: restored the 45 unrelated cache deletions attempted by SQLx prepare and added only the two required generated cache files.
- Final aggregate reviewer round 2: exact `PASS`; cache hashes, query text, parameters, result shape, and no unrelated deletions confirmed.
- Final committed-range reviewer round 3: exact `PASS`; no P0-P2 findings.
- Redmine fallback: issue 2 comment creation returned HTTP 404 and issue body updates did not persist; local plan was used and archived here.

## Blocked Questions

- Production disposition remains pending Docling-side confirmation for the 2,998 uncertain submissions. This is an operational follow-up, not a blocker for the code change.

## Final Status

COMPLETE
