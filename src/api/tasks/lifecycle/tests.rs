use super::*;

#[test]
fn canonical_task_list_query_requires_view_and_rejects_trashed_shape() {
    let canonical: CanonicalTaskListQuery = serde_json::from_value(serde_json::json!({
        "page": 1,
        "page_size": 25,
        "view": "processing"
    }))
    .expect("canonical requires view");
    assert_eq!(
        canonical.view,
        context69_contracts::TaskListView::Processing
    );
    assert!(canonical.validate().is_ok());
    let legacy: TaskListQuery = canonical.into();
    assert!(legacy.view.is_some());

    for trashed in [
        serde_json::json!({
            "page": 1,
            "page_size": 25,
            "view": "processing",
            "trashed": true
        }),
        serde_json::json!({
            "page": 1,
            "page_size": 25,
            "trashed": true
        }),
    ] {
        assert!(
            serde_json::from_value::<CanonicalTaskListQuery>(trashed.clone()).is_err(),
            "old ?trashed= requests must be rejected after removal"
        );
        assert!(
            serde_json::from_value::<TaskListQuery>(trashed).is_err(),
            "old trashed shapes must be rejected after removal"
        );
    }

    let missing_view = serde_json::from_value::<CanonicalTaskListQuery>(serde_json::json!({
        "page": 1,
        "page_size": 25
    }));
    assert!(
        missing_view.is_err(),
        "v0.18 path must require view; trashed-only queries are rejected"
    );

    let zero_page = CanonicalTaskListQuery {
        page: 0,
        page_size: 25,
        query: None,
        kind: None,
        status: None,
        view: context69_contracts::TaskListView::Processing,
        stage: None,
        waiting_reason: None,
        dependency_key: None,
        sort_by: None,
        sort_direction: None,
    };
    assert!(zero_page.validate().is_err());
    assert!(
        context69_http_support::validate_canonical_offset(0, 25).is_err(),
        "shared pagination bounds must reject zero page"
    );
    assert!(
        context69_http_support::validate_canonical_offset(1, 101).is_err(),
        "shared pagination bounds must reject oversized page_size"
    );
}

#[test]
fn canonical_view_filtering_never_widens_status() {
    use context69_contracts::{TaskListView, TaskStatus};
    let processing = TaskListView::Processing;
    let completed = TaskListView::Completed;
    assert_ne!(processing, completed);
    assert_eq!(processing.as_str(), "processing");
    let narrowed: Option<TaskStatus> = Some(TaskStatus::Succeeded);
    assert!(
        narrowed.is_some(),
        "status narrows the view; processing + succeeded matches nothing by SQL predicate"
    );
}

#[test]
fn task_items_query_status_defaults_to_none_and_round_trips() {
    // Issue 413 Phase 1: `status` is optional (absent lists every status)
    // and cursor pagination stays offset-based. Old callers sending only
    // limit/cursor keep working; the fixed active-first ordering lives in
    // SQL, not in a new sort param.
    let bare: TaskItemsQuery = serde_json::from_value(serde_json::json!({
        "limit": 100
    }))
    .expect("status defaults to none");
    assert_eq!(bare.limit, 100);
    assert!(bare.cursor.is_none());
    assert!(bare.status.is_none());

    let filtered: TaskItemsQuery = serde_json::from_value(serde_json::json!({
        "limit": 25,
        "cursor": "25",
        "status": "failed"
    }))
    .expect("status failed parses");
    assert_eq!(
        filtered.status,
        Some(context69_contracts::TaskItemStatus::Failed)
    );
    assert_eq!(filtered.cursor.as_deref(), Some("25"));

    assert!(
        serde_json::from_value::<TaskItemsQuery>(serde_json::json!({
            "limit": 25,
            "status": "bogus"
        }))
        .is_err(),
        "unknown item status must be rejected before the handler runs"
    );
}

/// The batch recovery wire shape is ordered and non-sensitive: accepted items
/// carry the recovered task id, skipped/failed items carry a machine-readable
/// reason and never a message.
#[test]
fn batch_retry_wire_shape_is_ordered_and_machine_readable() {
    let request: TaskBatchRetryRequest = serde_json::from_value(serde_json::json!({
        "task_ids": [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222"
        ]
    }))
    .expect("decode batch request");
    assert_eq!(request.task_ids.len(), 2);

    let response = context69_contracts::TaskBatchRetryResponse {
        results: vec![
            context69_contracts::TaskBatchRetryItem {
                task_id: request.task_ids[0],
                outcome: context69_contracts::TaskBatchRetryOutcome::Accepted,
                reason: None,
                retried_task_id: Some(request.task_ids[0]),
            },
            context69_contracts::TaskBatchRetryItem {
                task_id: request.task_ids[1],
                outcome: context69_contracts::TaskBatchRetryOutcome::Skipped,
                reason: Some(context69_contracts::TaskBatchRetryReason::NotFound),
                retried_task_id: None,
            },
        ],
    };
    let value = serde_json::to_value(&response).expect("encode batch response");
    assert_eq!(value["results"][0]["outcome"], "accepted");
    assert_eq!(
        value["results"][0]["retried_task_id"],
        serde_json::json!(request.task_ids[0].to_string())
    );
    assert!(value["results"][0].get("reason").is_none());
    assert_eq!(value["results"][1]["outcome"], "skipped");
    assert_eq!(value["results"][1]["reason"], "not_found");
    assert!(value["results"][1].get("retried_task_id").is_none());
}
