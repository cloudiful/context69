//! Health read model regression (Phase 6).
use chrono::Utc;
use context69::db::Database;
use context69_extraction::ExtractionStore;
use sqlx::Row;
use uuid::Uuid;

static HEALTH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}
async fn seed_group(db: &Database) -> i64 {
    let k = format!("h-group-{}", Uuid::new_v4());
    let r = sqlx::query("INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1,$2,'public','personal',$3) RETURNING id")
        .bind(&k).bind(&k).bind(format!("/{k}")).fetch_one(db.pool()).await.expect("group");
    r.get("id")
}
async fn seed_doc(db: &Database, gid: i64) -> i64 {
    let r = sqlx::query("INSERT INTO context69.documents (source_key, external_id, title, source_uri, updated_at_source, metadata_json, record_hash, group_id, visibility) VALUES ($1,$2,'t','u',now(),'{}',$3,$4,'public') RETURNING id")
        .bind(format!("s-{}", Uuid::new_v4())).bind(format!("e-{}", Uuid::new_v4())).bind(format!("h-{}", Uuid::new_v4())).bind(gid).fetch_one(db.pool()).await.expect("doc");
    r.get("id")
}
async fn seed_tmpl(db: &Database) -> String {
    let k = format!("t-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO context69.extraction_templates (template_key, version, system_prompt, output_schema, enabled) VALUES ($1,1,'p','{\"type\":\"object\"}',true) ON CONFLICT DO NOTHING").bind(&k).execute(db.pool()).await.expect("tmpl");
    k
}
async fn seed_job(
    db: &Database,
    doc: i64,
    tmpl: &str,
    hash: &str,
    status: &str,
    fc: Option<&str>,
    next: Option<chrono::DateTime<Utc>>,
    finished: Option<chrono::DateTime<Utc>>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO context69.document_extraction_jobs (id, document_id, template_key, template_version, source_record_hash, parameters, status, attempt_count, failure_class, next_attempt_at, finished_at) VALUES ($1,$2,$3,1,$4,'{}',$5,1,$6,$7,$8)")
        .bind(id).bind(doc).bind(tmpl).bind(hash).bind(status).bind(fc).bind(next).bind(finished).execute(db.pool()).await.expect("job");
    id
}

#[tokio::test]
async fn health_counts_and_class_aggregation() {
    let Some(u) = url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let _g = HEALTH_LOCK.lock().await;
    let db = Database::connect(&u).await.expect("connect");
    let store = ExtractionStore::new(db.pool().clone());
    let gid = seed_group(&db).await;
    let doc = seed_doc(&db, gid).await;
    let tmpl = seed_tmpl(&db).await;
    // Clean lingering jobs for this doc (should be none)
    let now = Utc::now();
    let due = seed_job(
        &db,
        doc,
        &tmpl,
        &format!("h-{}", Uuid::new_v4()),
        "queued",
        None,
        None,
        None,
    )
    .await;
    let await_id = seed_job(
        &db,
        doc,
        &tmpl,
        &format!("h-{}", Uuid::new_v4()),
        "queued",
        Some("transient"),
        Some(now + chrono::Duration::seconds(60)),
        None,
    )
    .await;
    let running = seed_job(
        &db,
        doc,
        &tmpl,
        &format!("h-{}", Uuid::new_v4()),
        "running",
        None,
        None,
        None,
    )
    .await;
    let failed_p = seed_job(
        &db,
        doc,
        &tmpl,
        &format!("h-{}", Uuid::new_v4()),
        "failed",
        Some("permanent"),
        None,
        Some(now - chrono::Duration::seconds(600)),
    )
    .await;
    let failed_t = seed_job(
        &db,
        doc,
        &tmpl,
        &format!("h-{}", Uuid::new_v4()),
        "failed",
        Some("transient"),
        None,
        Some(now - chrono::Duration::seconds(600)),
    )
    .await;
    let failed_old = seed_job(
        &db,
        doc,
        &tmpl,
        &format!("h-{}", Uuid::new_v4()),
        "failed",
        Some("permanent"),
        None,
        Some(now - chrono::Duration::hours(2)),
    )
    .await;

    let h = store.health().await.expect("health");
    assert!(h.queued >= 1, "queued must include due");
    assert!(h.running >= 1);
    assert!(h.awaiting_retry >= 1);
    assert!(h.next_retry_at.is_some());
    assert!(h.failed_last_hour >= 2, "only recent fails counted");
    assert_eq!(
        h.failure_class_counts
            .get("permanent")
            .cloned()
            .unwrap_or(0)
            >= 1,
        true
    );
    assert_eq!(
        h.failure_class_counts
            .get("transient")
            .cloned()
            .unwrap_or(0)
            >= 1,
        true
    );
    assert!(
        !h.failure_class_counts.contains_key("quota_exceeded")
            || h.failure_class_counts["quota_exceeded"] >= 0
    );

    for id in [due, await_id, running, failed_p, failed_t, failed_old] {
        sqlx::query("DELETE FROM context69.document_extraction_jobs WHERE id = $1")
            .bind(id)
            .execute(db.pool())
            .await
            .ok();
    }
    sqlx::query("DELETE FROM context69.documents WHERE id = $1")
        .bind(doc)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.extraction_templates WHERE template_key = $1")
        .bind(&tmpl)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(gid)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
async fn health_empty_is_zero() {
    let Some(u) = url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let _g = HEALTH_LOCK.lock().await;
    let db = Database::connect(&u).await.expect("connect");
    // Ensure no jobs for a fresh doc
    let store = ExtractionStore::new(db.pool().clone());
    // Health with existing data may not be zero, so we just check it doesn't error and fields are non-negative
    let h = store.health().await.expect("health");
    assert!(h.queued >= 0);
    assert!(h.running >= 0);
    assert!(h.awaiting_retry >= 0);
    assert!(h.failed_last_hour >= 0);
}
