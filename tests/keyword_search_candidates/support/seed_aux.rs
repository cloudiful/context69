#[path = "seed_translation.rs"]
mod translation;

use chrono::{Duration, TimeZone, Utc};
use context69::db::Database;
use serde_json::json;

use super::rows::{DocumentOptions, chunk, document};

pub async fn seed_private_translation(db: &Database, group_id: i64, tag: &str) {
    translation::seed_private(db, group_id, tag).await;
}

pub async fn seed(db: &Database, group_id: i64, tag: &str) {
    seed_date_rows(db, group_id, tag).await;
    seed_metadata_rows(db, group_id, tag).await;
    translation::seed(db, group_id, tag).await;
    seed_boundary_rows(db, group_id, tag).await;
}

async fn seed_date_rows(db: &Database, group_id: i64, tag: &str) {
    let start = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
    for (name, offset) in [("before", -1), ("start", 0), ("end", 1)] {
        let row = document(
            db,
            group_id,
            "fixture-source",
            &format!("date-{name}-{tag}"),
            "date test title",
            DocumentOptions {
                summary: None,
                metadata: json!({}),
                published_at: Some(start + Duration::seconds(offset)),
                visibility: "public",
            },
        )
        .await;
        chunk(db, &row, 0, &format!("date{tag}needle"), None).await;
    }
}

async fn seed_metadata_rows(db: &Database, group_id: i64, tag: &str) {
    for (name, metadata) in [
        (
            "gold",
            json!({"tier":"gold","tags":["red","blue"],"score":7,"nullable":null}),
        ),
        ("silver", json!({"tier":"silver","tags":["red"],"score":4})),
        ("missing", json!({"nullable":null})),
    ] {
        let row = document(
            db,
            group_id,
            "fixture-source",
            &format!("metadata-{name}-{tag}"),
            "metadata test title",
            DocumentOptions {
                summary: None,
                metadata,
                published_at: None,
                visibility: "public",
            },
        )
        .await;
        chunk(db, &row, 0, &format!("filter{tag}needle"), None).await;
    }
}

async fn seed_boundary_rows(db: &Database, group_id: i64, tag: &str) {
    let count = sqlx::query_scalar::<_, i64>(include_str!("../fixtures/boundary_seed.sql"))
        .bind(group_id)
        .bind(tag)
        .fetch_one(db.pool())
        .await
        .expect("seed bounded synthetic limit fixture");
    assert_eq!(
        count, 90,
        "limit fixture must contain more than 80 candidates"
    );
}
