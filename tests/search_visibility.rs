//! Integration tests for search/document authorization.
//!
//! Authorization must read visibility from the live `groups` table, never from
//! the `documents.visibility` copy: the copy is only a search pre-filter and is
//! not updated when a group changes visibility or is moved.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::contracts::SearchRequest;
use context69::db::Database;
use context69::domain::AccessScope;
use sqlx::Row;
use uuid::Uuid;

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

fn anonymous_public_scope() -> AccessScope {
    AccessScope {
        user_id: None,
        include_public: true,
        private_group_ids: Vec::new(),
        group_path: None,
        scoped_group_id: None,
    }
}

/// Seeds a public group containing one document whose chunk matches the
/// unique search token. Tests search by that token so parallel tests in the
/// same scratch database never observe each other's documents.
async fn seed_public_group_with_document(db: &Database) -> (i64, i64, String) {
    let token = format!("needle-{}", Uuid::new_v4());
    let group_key = format!("visibility-{}", Uuid::new_v4());
    let group_id = sqlx::query(
        "INSERT INTO context69.groups \
         (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(&group_key)
    .bind("Visibility Test Group")
    .bind(format!("test/{group_key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id");

    let document_id = sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, \
          updated_at_source, record_hash, metadata_json, visibility) \
         VALUES ($1, 'test-source', $2, $3, NULL, 'https://example.test/1', \
          now(), 'record-hash', '{}'::jsonb, 'public') RETURNING id",
    )
    .bind(group_id)
    .bind(format!("external-{group_key}"))
    .bind(&token)
    .fetch_one(db.pool())
    .await
    .expect("seed test document")
    .get("id");

    sqlx::query(
        "INSERT INTO context69.document_chunks \
         (id, document_id, chunk_index, chunk_text, record_hash) \
         VALUES ($1, $2, 0, $3, 'chunk-hash')",
    )
    .bind(Uuid::new_v4())
    .bind(document_id)
    .bind(&token)
    .execute(db.pool())
    .await
    .expect("seed test chunk");

    (group_id, document_id, token)
}

async fn cleanup_group_and_document(db: &Database, group_id: i64) {
    sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up documents");
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

fn search_request(query: &str, group_path: Option<String>) -> SearchRequest {
    SearchRequest {
        query: query.to_string(),
        locale: None,
        limit: 10,
        page: 1,
        source_key: None,
        group_path,
        published_after: None,
        published_before: None,
        metadata_filters: Vec::new(),
    }
}

async fn search_hits(db: &Database, query: &str, scope: &AccessScope) -> Vec<String> {
    db.keyword_search(&search_request(query, None), scope, 10)
        .await
        .expect("keyword search")
        .into_iter()
        .map(|hit| hit.external_id)
        .collect()
}

#[tokio::test]
async fn group_visibility_change_immediately_gates_document_search() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping search visibility test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let (group_id, document_id, token) = seed_public_group_with_document(&db).await;
    let scope = anonymous_public_scope();

    let hits = search_hits(&db, &token, &scope).await;
    assert!(!hits.is_empty(), "public document must be searchable");

    let fetched = db
        .get_document(document_id, &scope)
        .await
        .expect("get document");
    assert!(fetched.is_some(), "public document must be readable");

    // Flip the group to private while leaving the stale documents.visibility
    // copy untouched, exactly like update_group does in production.
    let rows_updated =
        sqlx::query("UPDATE context69.groups SET visibility = 'private' WHERE id = $1")
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("make group private")
            .rows_affected();
    assert_eq!(rows_updated, 1);
    let stale: String = sqlx::query("SELECT visibility FROM context69.documents WHERE id = $1")
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("read stale document visibility")
        .get("visibility");
    assert_eq!(
        stale, "public",
        "precondition: document copy must stay stale"
    );

    let hits = search_hits(&db, &token, &scope).await;
    assert!(
        hits.is_empty(),
        "document must vanish from search as soon as the group is private"
    );

    let fetched = db
        .get_document(document_id, &scope)
        .await
        .expect("get document");
    assert!(
        fetched.is_none(),
        "document must be unreadable as soon as the group is private"
    );

    sqlx::query("UPDATE context69.groups SET visibility = 'public' WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("make group public again");
    let hits = search_hits(&db, &token, &scope).await;
    assert!(
        !hits.is_empty(),
        "document must become searchable again after the group returns to public"
    );

    cleanup_group_and_document(&db, group_id).await;
}

#[tokio::test]
async fn scoped_search_follows_group_path_changes() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping scoped search test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let (group_id, _document_id, token) = seed_public_group_with_document(&db).await;
    let old_path: String = sqlx::query("SELECT full_path FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .fetch_one(db.pool())
        .await
        .expect("read group path")
        .get("full_path");

    // "Move" the group by rewriting its path, as move_group does in production.
    let new_path = format!("moved/{old_path}");
    sqlx::query("UPDATE context69.groups SET full_path = $1 WHERE id = $2")
        .bind(&new_path)
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("move group");

    let scope = AccessScope {
        user_id: None,
        include_public: true,
        private_group_ids: Vec::new(),
        group_path: Some(new_path.clone()),
        scoped_group_id: Some(group_id),
    };
    let hits = db
        .keyword_search(&search_request(&token, Some(new_path.clone())), &scope, 10)
        .await
        .expect("scoped keyword search");
    assert!(
        !hits.is_empty(),
        "scoped search must find the document at the new group path"
    );

    let hits = db
        .keyword_search(&search_request(&token, Some(old_path)), &scope, 10)
        .await
        .expect("scoped keyword search at old path");
    assert!(
        hits.is_empty(),
        "scoped search must not match the stale group path"
    );

    cleanup_group_and_document(&db, group_id).await;
}
