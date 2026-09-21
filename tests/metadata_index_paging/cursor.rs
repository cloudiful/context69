//! Cursor case: `list_documents.sql` must walk every matching document by
//! `id > $3` without leaking another `source_key`.

use super::support::*;

#[tokio::test]
async fn metadata_documents_page_walks_documents_by_id_cursor() {
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping cursor paging test");
        return;
    };
    let group_id = seed_group(&db, "metadata-cursor").await;
    seed_documents(&db, group_id, "cursor-source", 7).await;
    seed_documents(&db, group_id, "other-source", 3).await;
    let expected = document_ids(&db, group_id, "cursor-source").await;
    let other = document_ids(&db, group_id, "other-source").await;

    let definition = definition(group_id, "cursor-source");
    let mut paged = Vec::new();
    let mut first_page_len = None;
    let mut cursor = 0_i64;
    loop {
        let page = db
            .metadata_documents_page(&definition, cursor, 3)
            .await
            .expect("page documents");
        if page.is_empty() {
            break;
        }
        first_page_len.get_or_insert(page.len());
        cursor = page.last().expect("non-empty page").document_id;
        paged.extend(page.into_iter().map(|document| document.document_id));
    }

    assert_eq!(
        first_page_len,
        Some(3),
        "the page limit must bound each batch"
    );
    assert_eq!(
        paged, expected,
        "id-cursor paging must visit every matching document exactly once"
    );
    assert!(
        other.iter().all(|id| !paged.contains(id)),
        "documents of another source_key must not leak into the pages"
    );

    cleanup_group(&db, group_id).await;
}
