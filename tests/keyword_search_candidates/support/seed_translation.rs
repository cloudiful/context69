use chrono::{Duration, TimeZone, Utc};
use context69::db::Database;
use serde_json::json;
use uuid::Uuid;

use super::super::rows::{
    DocumentOptions, chunk, document, translated_chunk, translation_job, translation_version,
};

pub async fn seed_private(db: &Database, group_id: i64, tag: &str) {
    let private = document(
        db,
        group_id,
        "fixture-source",
        &format!("private-translated-{tag}"),
        "private source title",
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: None,
            visibility: "private",
        },
    )
    .await;
    let version = translation_version(
        db,
        Uuid::new_v4(),
        &private,
        "fr",
        &private.record_hash,
        &format!("private{tag}title"),
        None,
    )
    .await;
    translated_chunk(
        db,
        Uuid::new_v4(),
        version,
        private.id,
        "fr",
        0,
        &format!("private{tag}chunk"),
    )
    .await;
}

pub async fn seed(db: &Database, group_id: i64, tag: &str) {
    let published = Utc.with_ymd_and_hms(2025, 2, 1, 0, 0, 0).unwrap();
    let current = document(
        db,
        group_id,
        "fixture-source",
        &format!("translated-{tag}"),
        "translation source title",
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: Some(published),
            visibility: "public",
        },
    )
    .await;
    seed_current_translation(db, &current, tag, published).await;
    seed_fallback_document(db, group_id, tag, published).await;
}

async fn seed_current_translation(
    db: &Database,
    document: &super::super::rows::Document,
    tag: &str,
    published: chrono::DateTime<Utc>,
) {
    let colliding_chunk_id = Uuid::new_v4();
    chunk(
        db,
        document,
        0,
        &format!("same{tag}needle original"),
        Some(colliding_chunk_id),
    )
    .await;
    let current_translation = translation_version(
        db,
        Uuid::new_v4(),
        document,
        "fr",
        &document.record_hash,
        "translated title",
        Some("translated summary"),
    )
    .await;
    translated_chunk(
        db,
        colliding_chunk_id,
        current_translation,
        document.id,
        "fr",
        1,
        &format!("same{tag}needle traduit"),
    )
    .await;
    let stale_translation = translation_version(
        db,
        Uuid::new_v4(),
        document,
        "fr",
        "stale-source-record-hash",
        "stale translated title",
        None,
    )
    .await;
    translated_chunk(
        db,
        Uuid::new_v4(),
        stale_translation,
        document.id,
        "fr",
        3,
        &format!("stale{tag}needle"),
    )
    .await;
    translation_job(
        db,
        document.id,
        "fr",
        &document.record_hash,
        "failed",
        published,
    )
    .await;
    translation_job(
        db,
        document.id,
        "fr",
        &document.record_hash,
        "succeeded",
        published + Duration::seconds(1),
    )
    .await;
}

async fn seed_fallback_document(
    db: &Database,
    group_id: i64,
    tag: &str,
    published: chrono::DateTime<Utc>,
) {
    let fallback = document(
        db,
        group_id,
        "fixture-source",
        &format!("fallback-{tag}"),
        "fallback source title",
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: Some(published),
            visibility: "public",
        },
    )
    .await;
    chunk(
        db,
        &fallback,
        0,
        &format!("fallback{tag}needle original"),
        None,
    )
    .await;
    translation_job(
        db,
        fallback.id,
        "fr",
        &fallback.record_hash,
        "succeeded",
        published,
    )
    .await;
    translation_job(
        db,
        fallback.id,
        "fr",
        &fallback.record_hash,
        "failed",
        published + Duration::seconds(1),
    )
    .await;
}
