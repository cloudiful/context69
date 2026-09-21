//! Build case: `DocumentStoreService::build_index` must consume every page of
//! the 500-row id-cursor walk before it finishes an index.

use context69::contracts::{CreateMetadataIndexRequest, MetadataDataType, MetadataValueKind};

use super::support::*;

#[tokio::test]
async fn metadata_index_build_processes_every_document_page() {
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping build paging test");
        return;
    };
    // 500 documents per page: this needs one full page plus a partial second
    // page, so a build that stops after the first page fails the counts below.
    let document_count = 520_i64;
    let source_key = "build-source";
    let group_id = seed_group(&db, "metadata-build").await;
    seed_documents(&db, group_id, source_key, document_count).await;

    let service = build_document_store_service(&db).await;
    let created = service
        .create_index(
            group_id,
            "test/metadata-build",
            source_key,
            &CreateMetadataIndexRequest {
                path: "year".to_string(),
                data_type: MetadataDataType::Integer,
                value_kind: MetadataValueKind::Scalar,
                sortable: true,
            },
        )
        .await
        .expect("create metadata index");

    let ready = wait_for_ready(&service, group_id, source_key, created.index_id).await;
    assert_eq!(
        ready.processed_documents, document_count,
        "the rebuild must process every page"
    );
    let value_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.document_metadata_values WHERE index_id = $1",
    )
    .bind(created.index_id)
    .fetch_one(db.pool())
    .await
    .expect("count metadata values");
    assert_eq!(
        value_rows, document_count,
        "every document on every page must keep its metadata value row"
    );

    cleanup_group(&db, group_id).await;
}
