//! Issue #910 P1 caller-produced document result cases.
//!
//! Every case runs against the reusable test PostgreSQL target supplied by
//! `./scripts/test-backend.sh`; the harness never resolves a URL itself.

use axum::http::StatusCode;
use context69::contracts::{
    CreateDocumentResultRequest, DOCUMENT_RESULT_REQUEST_MAX_BYTES, ListDocumentResultsQuery,
};
use context69::domain_errors::status_for_error;

use super::support::*;

async fn request(value: serde_json::Value) -> CreateDocumentResultRequest {
    let request: CreateDocumentResultRequest =
        serde_json::from_value(value).expect("result envelope decodes");
    request.validate().expect("result envelope validates");
    request
}

async fn count_rows(db: &context69::db::Database, document_id: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.document_results WHERE document_id = $1")
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("count rows")
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn exact_replay_returns_the_original_immutable_row() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let user = seed_user(&db).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "hash-1", &serde_json::json!({"k": "v"})).await;
    let req = request(result_request("stock.news", "key-1", "hash-1")).await;

    let first = db
        .append_document_result(group, document, &path, user, &scope, &req)
        .await
        .expect("first append");
    let replay = db
        .append_document_result(group, document, &path, user, &scope, &req)
        .await
        .expect("exact replay");
    assert_eq!(first.id, replay.id, "an exact replay returns the same row");
    assert_eq!(replay.created_at, first.created_at);
    assert_eq!(count_rows(&db, document).await, 1);

    cleanup_group(&db, group).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn concurrent_exact_replay_yields_one_immutable_row() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let user = seed_user(&db).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "hash-1", &serde_json::json!({})).await;
    let req = request(result_request("stock.news", "key-race", "hash-1")).await;

    let (a, b) = tokio::join!(
        db.append_document_result(group, document, &path, user, &scope, &req),
        db.append_document_result(group, document, &path, user, &scope, &req),
    );
    let a = a.expect("concurrent append a");
    let b = b.expect("concurrent append b");
    assert_eq!(a.id, b.id, "both racers observe the one immutable row");
    assert_eq!(count_rows(&db, document).await, 1);

    cleanup_group(&db, group).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn changed_envelope_under_used_key_conflicts() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let user = seed_user(&db).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "hash-1", &serde_json::json!({})).await;
    let req = request(result_request("stock.news", "key-1", "hash-1")).await;
    db.append_document_result(group, document, &path, user, &scope, &req)
        .await
        .expect("first append");

    // Same key, different payload.
    let mut changed_value = result_request("stock.news", "key-1", "hash-1");
    changed_value["result"] = serde_json::json!({"direction": "negative"});
    let changed = request(changed_value).await;
    let error = db
        .append_document_result(group, document, &path, user, &scope, &changed)
        .await
        .expect_err("changed replay must conflict");
    assert_eq!(status_for_error(&error), StatusCode::CONFLICT);
    assert_eq!(count_rows(&db, document).await, 1);

    // Same key and payload, different provenance is also a changed envelope.
    let mut provenance_value = result_request("stock.news", "key-1", "hash-1");
    provenance_value["provenance"] = serde_json::json!({"run_id": "run-2"});
    let provenance = request(provenance_value).await;
    let error = db
        .append_document_result(group, document, &path, user, &scope, &provenance)
        .await
        .expect_err("changed provenance must conflict");
    assert_eq!(status_for_error(&error), StatusCode::CONFLICT);

    cleanup_group(&db, group).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn stale_source_hash_rejects_a_new_key() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let user = seed_user(&db).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "hash-2", &serde_json::json!({})).await;
    let stale = request(result_request("stock.news", "key-1", "hash-1")).await;

    let error = db
        .append_document_result(group, document, &path, user, &scope, &stale)
        .await
        .expect_err("stale source hash must reject a new key");
    assert_eq!(status_for_error(&error), StatusCode::CONFLICT);
    assert_eq!(count_rows(&db, document).await, 0);

    cleanup_group(&db, group).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn accepted_delivery_stays_replayable_after_document_update() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let user = seed_user(&db).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "hash-1", &serde_json::json!({})).await;
    let req = request(result_request("stock.news", "key-1", "hash-1")).await;
    let first = db
        .append_document_result(group, document, &path, user, &scope, &req)
        .await
        .expect("first append");

    // A later ingestion moves the document to a new source version.
    update_document_record_hash(&db, document, "hash-2").await;

    // The accepted delivery replays rather than conflicting with the new hash.
    let replay = db
        .append_document_result(group, document, &path, user, &scope, &req)
        .await
        .expect("exact replay after source change");
    assert_eq!(replay.id, first.id);
    assert_eq!(replay.source_record_hash, "hash-1");

    // A new key still has to bind the current version.
    let stale = request(result_request("stock.news", "key-2", "hash-1")).await;
    let error = db
        .append_document_result(group, document, &path, user, &scope, &stale)
        .await
        .expect_err("a new key with the old hash must conflict");
    assert_eq!(status_for_error(&error), StatusCode::CONFLICT);
    let current = request(result_request("stock.news", "key-2", "hash-2")).await;
    db.append_document_result(group, document, &path, user, &scope, &current)
        .await
        .expect("a new key with the current hash appends");
    assert_eq!(count_rows(&db, document).await, 2);

    cleanup_group(&db, group).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn append_history_keeps_distinct_runs_and_two_namespaces() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let user = seed_user(&db).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "hash-1", &serde_json::json!({})).await;

    for key in ["run-1", "run-2"] {
        let req = request(result_request("stock.news", key, "hash-1")).await;
        db.append_document_result(group, document, &path, user, &scope, &req)
            .await
            .expect("append distinct run");
    }
    // A second producer reusing a key is a different identity namespace.
    let other = request(result_request("stock.disclosure", "run-1", "hash-1")).await;
    db.append_document_result(group, document, &path, user, &scope, &other)
        .await
        .expect("append other producer namespace");

    assert_eq!(
        count_rows(&db, document).await,
        3,
        "distinct runs are retained"
    );

    cleanup_group(&db, group).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn list_pages_newest_first_with_bounds_and_current_hash() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let user = seed_user(&db).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "hash-1", &serde_json::json!({})).await;
    for key in ["k1", "k2", "k3"] {
        let req = request(result_request("stock.news", key, "hash-1")).await;
        db.append_document_result(group, document, &path, user, &scope, &req)
            .await
            .expect("append");
    }

    let page = db
        .list_document_results(
            group,
            document,
            &path,
            &scope,
            &ListDocumentResultsQuery {
                limit: 2,
                ..ListDocumentResultsQuery::default()
            },
        )
        .await
        .expect("first page");
    assert_eq!(
        page.source_record_hash, "hash-1",
        "the read reports the current hash"
    );
    assert_eq!(page.items.len(), 2);
    assert!(page.items[0].id > page.items[1].id, "newest first");
    let after = page.next_after_id.expect("a full page has a continuation");

    let next = db
        .list_document_results(
            group,
            document,
            &path,
            &scope,
            &ListDocumentResultsQuery {
                limit: 2,
                after_id: Some(after),
                ..ListDocumentResultsQuery::default()
            },
        )
        .await
        .expect("second page");
    assert_eq!(next.items.len(), 1);
    assert!(
        next.next_after_id.is_none(),
        "terminal page has no continuation"
    );

    // Out-of-bounds limits are rejected before any query.
    for limit in [0, 101] {
        let error = db
            .list_document_results(
                group,
                document,
                &path,
                &scope,
                &ListDocumentResultsQuery {
                    limit,
                    ..ListDocumentResultsQuery::default()
                },
            )
            .await
            .expect_err("limit must be bounded");
        assert_eq!(status_for_error(&error), StatusCode::BAD_REQUEST);
    }

    cleanup_group(&db, group).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn out_of_scope_and_cross_group_document_reads_are_not_found() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "private").await;
    let other = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let other_path = group_path(&db, other).await;
    let user = seed_user(&db).await;
    let document = seed_document(&db, group, "hash-1", &serde_json::json!({})).await;
    let req = request(result_request("stock.news", "key-1", "hash-1")).await;

    // The caller's scope never reaches the private document's group.
    let foreign_scope = member_scope(other, &other_path);
    let error = db
        .append_document_result(group, document, &path, user, &foreign_scope, &req)
        .await
        .expect_err("an invisible document must read as not found");
    assert_eq!(status_for_error(&error), StatusCode::NOT_FOUND);

    // A mounted route group that is not the document's group is also 404.
    let error = db
        .append_document_result(other, document, &other_path, user, &foreign_scope, &req)
        .await
        .expect_err("a cross-group document must read as not found");
    assert_eq!(status_for_error(&error), StatusCode::NOT_FOUND);

    let error = db
        .list_document_results(
            other,
            document,
            &other_path,
            &foreign_scope,
            &ListDocumentResultsQuery::default(),
        )
        .await
        .expect_err("listing a foreign document must read as not found");
    assert_eq!(status_for_error(&error), StatusCode::NOT_FOUND);

    cleanup_group(&db, group).await;
    cleanup_group(&db, other).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn list_read_does_not_mutate_document_metadata() {
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let user = seed_user(&db).await;
    let scope = member_scope(group, &path);
    let metadata = serde_json::json!({"ts_code": "600519.SH", "extraction": {"t": {"a": 1}}});
    let document = seed_document(&db, group, "hash-1", &metadata).await;
    let req = request(result_request("stock.news", "key-1", "hash-1")).await;
    db.append_document_result(group, document, &path, user, &scope, &req)
        .await
        .expect("append");
    db.list_document_results(
        group,
        document,
        &path,
        &scope,
        &ListDocumentResultsQuery::default(),
    )
    .await
    .expect("list");

    assert_eq!(
        document_metadata(&db, document).await,
        metadata,
        "reads and appends must never modify documents.metadata_json"
    );
    assert_eq!(document_record_hash(&db, document).await, "hash-1");

    cleanup_group(&db, group).await;
    cleanup_user(&db, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn source_read_reports_chunk_generation() {
    // The document read path a caller uses to produce results must expose the
    // generation each returned chunk belongs to, not a blank field.
    let db = connect_scratch().await;
    let group = seed_group(&db, "public").await;
    let path = group_path(&db, group).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "hash-2", &serde_json::json!({})).await;
    seed_document_chunk(&db, document, 0, "alpha", "hash-1").await;

    let response = db
        .get_document(document, &scope)
        .await
        .expect("read document")
        .expect("document is visible");
    assert_eq!(response.record_hash, "hash-2", "header generation");
    assert_eq!(response.chunks.len(), 1);
    assert_eq!(
        response.chunks[0].record_hash, "hash-1",
        "the chunk exposes its own generation so a caller can compare"
    );

    cleanup_group(&db, group).await;
}

#[test]
fn malformed_and_oversize_envelopes_are_rejected_before_any_io() {
    // Blank identifier alphabet, non-object result, and an oversize serialized
    // envelope are all refused by the shared contract.
    for bad in [
        serde_json::json!({
            "producer": "stock news", "result_kind": "k", "schema_ref": "s",
            "source_record_hash": "h", "idempotency_key": "i", "result": {}
        }),
        serde_json::json!({
            "producer": "stock.news", "result_kind": "k", "schema_ref": "s",
            "source_record_hash": "h", "idempotency_key": "i", "result": [1]
        }),
    ] {
        let parsed: CreateDocumentResultRequest =
            serde_json::from_value(bad).expect("envelope decodes");
        assert!(
            parsed.validate().is_err(),
            "invalid envelope must be refused"
        );
    }
    let oversize = serde_json::json!({
        "producer": "stock.news", "result_kind": "k", "schema_ref": "s",
        "source_record_hash": "h", "idempotency_key": "i",
        "result": {"blob": "a".repeat(DOCUMENT_RESULT_REQUEST_MAX_BYTES)}
    });
    let parsed: CreateDocumentResultRequest =
        serde_json::from_value(oversize).expect("oversize envelope decodes");
    assert!(
        parsed.validate().is_err(),
        "oversize envelope must be refused"
    );
}
