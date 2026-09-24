//! Seed, cleanup, and payload builders for the issue 592 pipeline tests.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{db::Database, domain::GroupRecord};

pub(super) async fn seed_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, 'unused') RETURNING id",
    )
    .bind(format!("issue-592-{}", Uuid::new_v4()))
    .bind("Issue 592 Test")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
    .get("id")
}

pub(super) async fn seed_group(db: &Database) -> GroupRecord {
    let group_id: i64 = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("issue-592-{}", Uuid::new_v4()))
    .bind("Issue 592 Test Group")
    .bind(format!("test/issue-592-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id");
    db.get_group_by_id(group_id)
        .await
        .expect("load group")
        .expect("group exists")
}

pub(super) async fn cleanup(db: &Database, group_id: i64, user_id: i64) {
    // `library_files`/`library_folders` are group-restricted, so clear them
    // before the group; tasks/items/idempotency keys cascade from the user.
    for sql in [
        "DELETE FROM context69.library_files WHERE group_id = $1",
        "DELETE FROM context69.library_storage_objects WHERE group_id = $1",
        "DELETE FROM context69.library_folders WHERE group_id = $1",
        "DELETE FROM context69.groups WHERE id = $1",
    ] {
        sqlx::query(sql)
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("cleanup group");
    }
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup user");
}

pub(super) fn payload(kind: &str) -> Value {
    match kind {
        "text_batch" => json!({
            "external_id": format!("issue-592-text-{}", Uuid::new_v4()),
            "title": "Issue 592 text",
            "content": "the storage stage creates the file and its sections",
        }),
        "file_batch" => {
            let content = b"issue 592 file body";
            json!({
                "filename": "issue-592.txt",
                "media_type": "text/plain",
                "content_base64": STANDARD.encode(content),
                "options": {
                    "metadata": { "external_id": format!("issue-592-file-{}", Uuid::new_v4()) },
                    "source_policy": "retain"
                }
            })
        }
        _ => {
            let content = b"issue 592 url body";
            let sha256 = Sha256::digest(content)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            json!({
                "url": "https://example.com/issue-592.txt",
                "options": {
                    "metadata": { "external_id": format!("issue-592-url-{}", Uuid::new_v4()) },
                    "source_policy": "retain"
                },
                "download_artifact": {
                    "source_url": "https://example.com/issue-592.txt",
                    "filename": "issue-592.txt",
                    "media_type": "text/plain",
                    "sha256": sha256,
                    "content_base64": STANDARD.encode(content)
                }
            })
        }
    }
}
