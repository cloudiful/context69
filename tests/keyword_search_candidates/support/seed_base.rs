use context69::db::Database;
use serde_json::json;
use uuid::Uuid;

use super::rows::{DocumentOptions, chunk, document};

pub async fn seed(db: &Database, group_id: i64, private_group_id: i64, tag: &str) {
    seed_phrase_cases(db, group_id, tag).await;
    seed_visibility_cases(db, group_id, private_group_id, tag).await;
}

async fn seed_phrase_cases(db: &Database, group_id: i64, tag: &str) {
    let expansion = document(
        db,
        group_id,
        "fixture-source",
        &format!("expansion-{tag}"),
        &format!("expansion{tag} phrase{tag} title"),
        DocumentOptions {
            summary: Some(" \t "),
            metadata: json!({
                "is_library_file": true,
                "library_file_id": Uuid::new_v4(),
                "library_section_label": "Synthetic section",
                "library_path": "synthetic/path.md"
            }),
            published_at: None,
            visibility: "public",
        },
    )
    .await;
    chunk(db, &expansion, 0, "first neutral chunk", None).await;
    chunk(db, &expansion, 1, "second neutral chunk", None).await;

    let phrase = document(
        db,
        group_id,
        "fixture-source",
        &format!("phrase-{tag}"),
        &format!("ordinary {tag} title"),
        DocumentOptions {
            summary: Some("Meaningful summary"),
            metadata: json!({}),
            published_at: None,
            visibility: "public",
        },
    )
    .await;
    chunk(
        db,
        &phrase,
        0,
        &format!("needle{tag} phrase{tag} in chunk"),
        None,
    )
    .await;

    let terms = document(
        db,
        group_id,
        "fixture-source",
        &format!("terms-{tag}"),
        &format!("alpha{tag}"),
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: None,
            visibility: "public",
        },
    )
    .await;
    chunk(db, &terms, 0, &format!("beta{tag} gamma{tag}"), None).await;

    let duplicate = document(
        db,
        group_id,
        "fixture-source",
        &format!("duplicate-{tag}"),
        "ordinary duplicate title",
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: None,
            visibility: "public",
        },
    )
    .await;
    chunk(db, &duplicate, 0, &format!("repeat{tag}able"), None).await;

    let split = document(
        db,
        group_id,
        "fixture-source",
        &format!("split-{tag}"),
        &format!("left{tag}"),
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: None,
            visibility: "public",
        },
    )
    .await;
    chunk(db, &split, 0, &format!("right{tag}"), None).await;

    seed_wildcard_cases(db, group_id, tag).await;
    let cjk = document(
        db,
        group_id,
        "fixture-source",
        &format!("cjk-{tag}"),
        &format!("中文标题{tag}"),
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: None,
            visibility: "public",
        },
    )
    .await;
    chunk(db, &cjk, 0, "first CJK chunk", None).await;
    chunk(db, &cjk, 1, "second CJK chunk", None).await;
}

async fn seed_wildcard_cases(db: &Database, group_id: i64, tag: &str) {
    for (external_id, title, text) in [
        (
            "wildcard",
            "ordinary wildcard title".to_owned(),
            format!("wild{tag}-anything-card"),
        ),
        (
            "underscore",
            "ordinary underscore title".to_owned(),
            format!("under{tag}Xscore"),
        ),
        (
            "backslash",
            "ordinary backslash title".to_owned(),
            format!("slash{tag}word"),
        ),
    ] {
        let row = document(
            db,
            group_id,
            "fixture-source",
            &format!("{external_id}-{tag}"),
            &title,
            DocumentOptions {
                summary: None,
                metadata: json!({}),
                published_at: None,
                visibility: "public",
            },
        )
        .await;
        chunk(db, &row, 0, &text, None).await;
    }
}

async fn seed_visibility_cases(db: &Database, group_id: i64, private_group_id: i64, tag: &str) {
    let private = document(
        db,
        private_group_id,
        "fixture-source",
        &format!("private-{tag}"),
        "private synthetic title",
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: None,
            visibility: "private",
        },
    )
    .await;
    chunk(db, &private, 0, &format!("secret{tag}needle"), None).await;

    let source_filtered = document(
        db,
        group_id,
        "other-source",
        &format!("source-{tag}"),
        "source filter title",
        DocumentOptions {
            summary: None,
            metadata: json!({}),
            published_at: None,
            visibility: "public",
        },
    )
    .await;
    chunk(db, &source_filtered, 0, &format!("source{tag}needle"), None).await;
}
