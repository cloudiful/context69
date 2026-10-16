//! Issue #910 P3 chunk source-generation cases.
//!
//! Every case runs against the reusable test PostgreSQL target supplied by
//! `./scripts/test-backend.sh`; the harness never resolves a URL itself.

use super::support::*;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn original_read_reports_each_chunk_generation() {
    let db = connect_scratch().await;
    let group = seed_group(&db).await;
    let path = group_path(&db, group).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "gen-1").await;
    seed_chunk(&db, document, 0, "alpha body", "gen-1").await;
    seed_chunk(&db, document, 1, "beta body", "gen-1").await;

    let response = db
        .get_document(document, &scope)
        .await
        .expect("read document")
        .expect("document is visible");
    assert_eq!(response.record_hash, "gen-1", "header generation");
    assert_eq!(response.chunks.len(), 2);
    for (chunk, expected) in response.chunks.iter().zip(["alpha body", "beta body"]) {
        assert_eq!(chunk.text, expected);
        assert_eq!(
            chunk.record_hash, "gen-1",
            "a coherent read stamps the header generation on every chunk"
        );
    }

    cleanup_group(&db, group).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn mixed_generation_read_is_observable() {
    let db = connect_scratch().await;
    let group = seed_group(&db).await;
    let path = group_path(&db, group).await;
    let scope = member_scope(group, &path);
    // The header has already advanced to gen-2 while the chunks still carry the
    // older gen-1 text; the read must report that honestly instead of stamping
    // the header hash onto the stale chunk.
    let document = seed_document(&db, group, "gen-2").await;
    seed_chunk(&db, document, 0, "stale body", "gen-1").await;

    let response = db
        .get_document(document, &scope)
        .await
        .expect("read document")
        .expect("document is visible");
    assert_eq!(
        response.record_hash, "gen-2",
        "header generation is current"
    );
    assert_eq!(response.chunks.len(), 1);
    assert_eq!(response.chunks[0].text, "stale body");
    assert_eq!(
        response.chunks[0].record_hash, "gen-1",
        "the chunk keeps its own generation so the mismatch is observable"
    );

    cleanup_group(&db, group).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn localized_read_reports_translation_source_generation() {
    let db = connect_scratch().await;
    let group = seed_group(&db).await;
    let path = group_path(&db, group).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "gen-1").await;
    seed_chunk(&db, document, 0, "original body", "gen-1").await;
    let version = seed_translation_version(&db, document, "zh-CN", "gen-1", "翻译标题").await;
    seed_translation_chunk(&db, version, document, "zh-CN", 0, "翻译正文").await;

    let response = db
        .get_document_localized(document, Some("zh-CN"), &scope)
        .await
        .expect("read translated document")
        .expect("document is visible");
    assert_eq!(response.title, "翻译标题");
    assert_eq!(response.content_locale.as_deref(), Some("zh-CN"));
    assert_eq!(response.chunks.len(), 1);
    assert_eq!(
        response.chunks[0].text, "翻译正文",
        "the translated text is returned"
    );
    assert_eq!(
        response.chunks[0].record_hash, "gen-1",
        "a translated chunk carries the version's source generation"
    );

    cleanup_group(&db, group).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn original_none_locale_read_keeps_provenance_and_text() {
    let db = connect_scratch().await;
    let group = seed_group(&db).await;
    let path = group_path(&db, group).await;
    let scope = member_scope(group, &path);
    let document = seed_document(&db, group, "gen-1").await;
    seed_chunk(&db, document, 0, "alpha body", "gen-1").await;

    let response = db
        .get_document_localized(document, None, &scope)
        .await
        .expect("read original document")
        .expect("document is visible");
    assert_eq!(response.content_locale.as_deref(), Some("original"));
    assert_eq!(response.chunks[0].text, "alpha body");
    assert_eq!(
        response.chunks[0].record_hash, "gen-1",
        "an original read without a locale still carries chunk provenance"
    );

    cleanup_group(&db, group).await;
}
