use super::*;

use context69_contracts::{TaskResponse, TaskStreamDone, TaskStreamSnapshot, TaskStreamUpdate};

use super::producer::{
    TASK_STREAM_UPDATE_COALESCE_WINDOW, TaskUpdateCoalescer, is_terminal_task_status,
};

#[test]
fn task_stream_query_decodes_comma_ids_and_empty_means_watch_all() {
    // Mirror the search-stream canonical query tests: decoding plus
    // validation happen before the SSE body starts.
    let uri = axum::http::Uri::from_static("/v1/tasks/stream");
    let axum::extract::Query(decoded) =
        axum::extract::Query::<TaskStreamQuery>::try_from_uri(&uri).expect("empty query decodes");
    assert!(decoded.task_ids.is_none());
    assert!(
        parse_task_stream_ids(decoded.task_ids.as_deref())
            .expect("watch-all")
            .is_empty()
    );

    let one = "11111111-1111-4111-8111-111111111111";
    let two = "22222222-2222-4222-8222-222222222222";
    let ids =
        parse_task_stream_ids(Some(&format!("{one},{two},{one} , "))).expect("comma ids parse");
    assert_eq!(ids.len(), 2, "duplicate IDs must dedupe");
    assert_eq!(ids[0].to_string(), one);
    assert_eq!(ids[1].to_string(), two);
    assert!(parse_task_stream_ids(Some("  ")).expect("blank").is_empty());
}

#[test]
fn task_stream_rejects_malformed_and_oversized_id_lists() {
    // Invalid IDs surface as 400 before the 200 stream starts, mirroring
    // the search-stream limit/cursor validation.
    assert!(parse_task_stream_ids(Some("not-a-uuid")).is_err());
    assert!(parse_task_stream_ids(Some("11111111-1111-4111-8111-111111111111, nope")).is_err());
    let many = (0..(TASK_STREAM_IDS_MAX + 1))
        .map(|_| "11111111-1111-4111-8111-111111111111".to_string())
        .collect::<Vec<_>>()
        .join(",");
    // Deduped to one, so this stays valid; build distinct IDs to overflow.
    assert_eq!(parse_task_stream_ids(Some(&many)).expect("dedupe").len(), 1);
    let distinct = (0..(TASK_STREAM_IDS_MAX + 1))
        .map(|i| format!("{:08x}-1111-4111-8111-111111111111", i))
        .collect::<Vec<_>>()
        .join(",");
    assert!(
        parse_task_stream_ids(Some(&distinct)).is_err(),
        "more than {TASK_STREAM_IDS_MAX} distinct IDs must be rejected"
    );
}

#[test]
fn task_stream_terminal_detection_covers_only_terminal_states() {
    use context69_contracts::TaskStatus;
    for terminal in [
        TaskStatus::Succeeded,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ] {
        assert!(
            is_terminal_task_status(&terminal),
            "terminal status must close a filtered stream: {terminal:?}"
        );
    }
    for active in [TaskStatus::Queued, TaskStatus::Running, TaskStatus::Waiting] {
        assert!(
            !is_terminal_task_status(&active),
            "active status must keep the stream open: {active:?}"
        );
    }
}

#[test]
fn task_stream_event_frames_carry_snapshot_update_done_error_shapes() {
    // Contract shapes for the four SSE event names. The transport sends
    // each payload under its own `event:` frame; error mirrors the search
    // stream `{ "message": ... }` envelope.
    let task_id = uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111").expect("uuid");
    let task = TaskResponse {
        task_id,
        kind: context69_contracts::TaskKind::TextBatch,
        status: context69_contracts::TaskStatus::Running,
        origin: context69_contracts::TaskOrigin::Manual,
        group_path: None,
        source_key: None,
        stage: None,
        waiting_reason: None,
        dependency_key: None,
        progress: context69_contracts::TaskProgress {
            total: 1,
            queued: 0,
            running: 1,
            waiting: 0,
            succeeded: 0,
            failed: 0,
            cancelled: 0,
        },
        failure_stage: None,
        error_summary: None,
        eta_seconds: None,
        created_at: chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
            .expect("time")
            .with_timezone(&chrono::Utc),
        started_at: None,
        finished_at: None,
        updated_at: chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
            .expect("time")
            .with_timezone(&chrono::Utc),
        deleted_at: None,
        file_name: None,
        document_title: None,
    };
    let snapshot = TaskStreamSnapshot {
        tasks: vec![task.clone()],
    };
    let snapshot_value = serde_json::to_value(&snapshot).expect("snapshot serializes");
    assert_eq!(
        snapshot_value
            .get("tasks")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(1)
    );
    let update = TaskStreamUpdate { task: task.clone() };
    let update_value = serde_json::to_value(&update).expect("update serializes");
    assert_eq!(
        update_value
            .get("task")
            .and_then(|t| t.get("task_id"))
            .and_then(|v| v.as_str()),
        Some("11111111-1111-4111-8111-111111111111")
    );
    let done = TaskStreamDone {
        tasks: vec![task.clone()],
    };
    assert_eq!(
        serde_json::to_value(&done)
            .expect("done")
            .get("tasks")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(1)
    );
    // The tagged union documents the update/done/error contract; the
    // snapshot variant is the first frame on every connection.
    let event_value =
        serde_json::to_value(TaskStreamEvent::Update(Box::new(update))).expect("event");
    assert_eq!(
        event_value.get("type").and_then(|v| v.as_str()),
        Some("update")
    );
    let error_value = serde_json::to_value(&TaskStreamEvent::Error {
        message: "boom".to_string(),
    })
    .expect("error");
    assert_eq!(
        error_value.get("type").and_then(|v| v.as_str()),
        Some("error")
    );
    assert_eq!(
        error_value.get("message").and_then(|v| v.as_str()),
        Some("boom")
    );
    // SSE mapping must not panic for any frame and must keep the stream
    // shape (mirrors the search-stream generator test structure).
    for event in [
        TaskStreamEvent::Snapshot(snapshot),
        TaskStreamEvent::Update(Box::new(TaskStreamUpdate { task: task.clone() })),
        TaskStreamEvent::Done(done),
        TaskStreamEvent::Error {
            message: "lagged".to_string(),
        },
    ] {
        let _ = sse_task_event(event);
    }
}

#[test]
fn task_stream_update_coalesce_window_is_three_seconds() {
    // Issue 413 Phase 2: aggregation window is user-confirmed 3s. Snapshot
    // stays immediate; only `update` frames wait for this interval.
    assert_eq!(
        TASK_STREAM_UPDATE_COALESCE_WINDOW,
        std::time::Duration::from_secs(3)
    );
}

fn coalescer_test_task(
    task_id: Uuid,
    stage: Option<&str>,
    status: context69_contracts::TaskStatus,
) -> TaskResponse {
    TaskResponse {
        task_id,
        kind: context69_contracts::TaskKind::TextBatch,
        status,
        origin: context69_contracts::TaskOrigin::Manual,
        group_path: None,
        source_key: None,
        stage: stage.map(ToOwned::to_owned),
        waiting_reason: None,
        dependency_key: None,
        progress: context69_contracts::TaskProgress {
            total: 1,
            queued: 0,
            running: 1,
            waiting: 0,
            succeeded: 0,
            failed: 0,
            cancelled: 0,
        },
        failure_stage: None,
        error_summary: None,
        eta_seconds: None,
        created_at: chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
            .expect("time")
            .with_timezone(&chrono::Utc),
        started_at: None,
        finished_at: None,
        updated_at: chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
            .expect("time")
            .with_timezone(&chrono::Utc),
        deleted_at: None,
        file_name: None,
        document_title: None,
    }
}

#[test]
fn task_update_coalescer_keeps_latest_per_task() {
    // Bursty bus events for one task collapse to one `update` per window:
    // latest full state wins, so a 2781-event storm costs one frame.
    let task_id = Uuid::parse_str("11111111-1111-4111-8111-111111111111").expect("uuid");
    let mut buffer = TaskUpdateCoalescer::default();
    assert!(buffer.is_empty());
    for stage in ["stage-0", "stage-1", "stage-49"] {
        buffer.push(coalescer_test_task(
            task_id,
            Some(stage),
            context69_contracts::TaskStatus::Running,
        ));
    }
    assert_eq!(
        buffer.pending.len(),
        1,
        "same task_id must overwrite, not queue"
    );
    let flushed = buffer.drain();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].task_id, task_id);
    assert_eq!(flushed[0].stage.as_deref(), Some("stage-49"));
    assert!(buffer.is_empty(), "drain must clear the window");
    assert!(buffer.drain().is_empty());
}

#[test]
fn task_update_coalescer_flushes_each_task_once_in_stable_order() {
    // Distinct tasks each keep their latest state; flush order is sorted
    // by task_id so the SSE frame order is deterministic.
    let first = Uuid::parse_str("11111111-1111-4111-8111-111111111111").expect("uuid");
    let second = Uuid::parse_str("22222222-2222-4222-8222-222222222222").expect("uuid");
    let mut buffer = TaskUpdateCoalescer::default();
    buffer.push(coalescer_test_task(
        second,
        Some("second-v1"),
        context69_contracts::TaskStatus::Running,
    ));
    buffer.push(coalescer_test_task(
        first,
        Some("first-v1"),
        context69_contracts::TaskStatus::Running,
    ));
    buffer.push(coalescer_test_task(
        second,
        Some("second-v2"),
        context69_contracts::TaskStatus::Running,
    ));
    assert_eq!(buffer.pending.len(), 2);
    let flushed = buffer.drain();
    assert_eq!(flushed.len(), 2);
    assert_eq!(
        flushed.iter().map(|task| task.task_id).collect::<Vec<_>>(),
        vec![first, second],
        "flush must be deterministic across HashMap iteration"
    );
    assert_eq!(flushed[1].stage.as_deref(), Some("second-v2"));
}
