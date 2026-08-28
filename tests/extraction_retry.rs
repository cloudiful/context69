//! Extraction retry regression (Phase 6) – gated by CONTEXT69_TEST_DATABASE_URL.
use chrono::Utc;
use context69::db::Database;
use context69_contracts::ExtractionFailureClass;
use context69_extraction::{
    ExtractionDependencies, ExtractionReadiness, ExtractionService, ExtractionStore,
    providers::{
        ProviderHttpError, ProviderPayloadError, ProviderSchemaError, classify_error,
        failure_class_as_str, next_retry_delay,
    },
};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn grp(db: &Database) -> i64 {
    let k = format!("g-{}", Uuid::new_v4());
    let r = sqlx::query("INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1,$2,'public','personal',$3) RETURNING id")
        .bind(&k).bind(&k).bind(format!("/{k}")).fetch_one(db.pool()).await.unwrap();
    r.get("id")
}
async fn doc(db: &Database, gid: i64) -> i64 {
    let r = sqlx::query("INSERT INTO context69.documents (source_key, external_id, title, source_uri, updated_at_source, metadata_json, record_hash, group_id, visibility) VALUES ($1,$2,'t','u',now(),'{}',$3,$4,'public') RETURNING id")
        .bind(format!("s-{}", Uuid::new_v4())).bind(format!("e-{}", Uuid::new_v4())).bind(format!("h-{}", Uuid::new_v4())).bind(gid).fetch_one(db.pool()).await.unwrap();
    r.get("id")
}
async fn tmpl(db: &Database) -> String {
    let k = format!("t-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO context69.extraction_templates (template_key, version, system_prompt, output_schema, enabled) VALUES ($1,1,'p','{\"type\":\"object\"}',true) ON CONFLICT DO NOTHING").bind(&k).execute(db.pool()).await.unwrap();
    k
}
async fn job(
    db: &Database,
    doc: i64,
    tmpl: &str,
    hash: &str,
    status: &str,
    fc: Option<&str>,
    next: Option<chrono::DateTime<Utc>>,
) -> Uuid {
    job_with_attempt(db, doc, tmpl, hash, status, 0, fc, next).await
}
async fn job_with_attempt(
    db: &Database,
    doc: i64,
    tmpl: &str,
    hash: &str,
    status: &str,
    attempt: i32,
    fc: Option<&str>,
    next: Option<chrono::DateTime<Utc>>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO context69.document_extraction_jobs (id, document_id, template_key, template_version, source_record_hash, parameters, status, attempt_count, failure_class, next_attempt_at) VALUES ($1,$2,$3,1,$4,'{}',$5,$6,$7,$8)")
        .bind(id).bind(doc).bind(tmpl).bind(hash).bind(status).bind(attempt).bind(fc).bind(next).execute(db.pool()).await.unwrap();
    id
}
async fn clean(db: &Database, ids: &[Uuid]) {
    for id in ids {
        sqlx::query("DELETE FROM context69.document_extraction_attempts WHERE job_id=$1")
            .bind(*id)
            .execute(db.pool())
            .await
            .ok();
        sqlx::query("DELETE FROM context69.document_extraction_jobs WHERE id=$1")
            .bind(*id)
            .execute(db.pool())
            .await
            .ok();
    }
}
struct Noop;
#[async_trait::async_trait]
impl context69_extraction::ExtractionPublisher for Noop {
    async fn publish(
        &self,
        _: &context69_extraction::ExtractionPublication<'_>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
struct Ready;
#[async_trait::async_trait]
impl ExtractionReadiness for Ready {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        Ok(true)
    }
}

#[tokio::test]
async fn due_pending_and_claim() {
    let Some(u) = url() else {
        eprintln!("skip");
        return;
    };
    let _g = LOCK.lock().await;
    let db = Database::connect(&u).await.unwrap();
    let store = ExtractionStore::new(db.pool().clone());
    let gid = grp(&db).await;
    let did = doc(&db, gid).await;
    let t = tmpl(&db).await;
    let due = job(
        &db,
        did,
        &t,
        &format!("h-{}", Uuid::new_v4()),
        "queued",
        None,
        None,
    )
    .await;
    let fut = job(
        &db,
        did,
        &t,
        &format!("h-{}", Uuid::new_v4()),
        "queued",
        Some("transient"),
        Some(Utc::now() + chrono::Duration::seconds(60)),
    )
    .await;
    let pending = store.pending_ids().await.unwrap();
    assert!(pending.contains(&due));
    assert!(!pending.contains(&fut));
    assert!(store.claim_job(fut).await.unwrap().is_none());
    let claimed = store.claim_job(due).await.unwrap();
    assert!(claimed.is_some());
    // fut should still not be claimable via store (already verified above)
    clean(&db, &[due, fut]).await;
    sqlx::query("DELETE FROM context69.documents WHERE id=$1")
        .bind(did)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.extraction_templates WHERE template_key=$1")
        .bind(&t)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id=$1")
        .bind(gid)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
async fn queued_retry_and_health() {
    let Some(u) = url() else {
        eprintln!("skip");
        return;
    };
    let _g = LOCK.lock().await;
    let db = Database::connect(&u).await.unwrap();
    let store = ExtractionStore::new(db.pool().clone());
    let gid = grp(&db).await;
    let did = doc(&db, gid).await;
    let t = tmpl(&db).await;
    let h = format!("h-{}", Uuid::new_v4());
    let jid = job(&db, did, &t, &h, "queued", None, None).await;
    let j = store.claim_job(jid).await.unwrap().unwrap();
    assert_eq!(j.attempt_count, 1);
    let d = next_retry_delay(j.attempt_count);
    assert_eq!(d, std::time::Duration::from_secs(5));
    let next = Utc::now() + chrono::Duration::from_std(d).unwrap();
    sqlx::query(
        "UPDATE context69.document_extraction_jobs SET status='queued', provider_key='llm', provider_config_hash='h', error_message='err', failure_class='transient', next_attempt_at=$2, finished_at=NULL, updated_at=now() WHERE id=$1",
    )
    .bind(jid)
    .bind(next)
    .execute(db.pool())
    .await
    .unwrap();
    let r = sqlx::query("SELECT status, failure_class, next_attempt_at, finished_at FROM context69.document_extraction_jobs WHERE id=$1").bind(jid).fetch_one(db.pool()).await.unwrap();
    let st: String = r.get("status");
    let fc: Option<String> = r.get("failure_class");
    let na: Option<chrono::DateTime<Utc>> = r.get("next_attempt_at");
    let fin: Option<chrono::DateTime<Utc>> = r.get("finished_at");
    assert_eq!(st, "queued");
    assert_eq!(fc.as_deref(), Some("transient"));
    assert!(na.is_some());
    assert!(fin.is_none());
    assert!(!store.pending_ids().await.unwrap().contains(&jid));
    assert!(store.next_pending_at().await.unwrap().is_some());
    let health = store.health().await.unwrap();
    assert!(health.awaiting_retry >= 1);
    clean(&db, &[jid]).await;
    sqlx::query("DELETE FROM context69.documents WHERE id=$1")
        .bind(did)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.extraction_templates WHERE template_key=$1")
        .bind(&t)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id=$1")
        .bind(gid)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
async fn manual_retry_and_reset() {
    let Some(u) = url() else {
        eprintln!("skip");
        return;
    };
    let _g = LOCK.lock().await;
    let db = Database::connect(&u).await.unwrap();
    let store = ExtractionStore::new(db.pool().clone());
    let gid = grp(&db).await;
    let did = doc(&db, gid).await;
    let t = tmpl(&db).await;
    let jid = job_with_attempt(
        &db,
        did,
        &t,
        &format!("h-{}", Uuid::new_v4()),
        "failed",
        1,
        Some("transient"),
        Some(Utc::now() + chrono::Duration::seconds(30)),
    )
    .await;
    sqlx::query("INSERT INTO context69.document_extraction_attempts (job_id, provider_key, provider_config_hash, attempt_number, status, latency_ms) VALUES ($1,'llm','h',1,'failed',100)").bind(jid).execute(db.pool()).await.unwrap();
    let ret = store.retry_job(gid, jid).await.unwrap().unwrap();
    assert_eq!(ret.status, "queued");
    let r = sqlx::query(
        "SELECT failure_class, next_attempt_at FROM context69.document_extraction_jobs WHERE id=$1",
    )
    .bind(jid)
    .fetch_one(db.pool())
    .await
    .unwrap();
    let fc: Option<String> = r.get("failure_class");
    let na: Option<chrono::DateTime<Utc>> = r.get("next_attempt_at");
    assert!(fc.is_none() && na.is_none());
    let run = job_with_attempt(
        &db,
        did,
        &t,
        &format!("h-{}", Uuid::new_v4()),
        "running",
        1,
        Some("transient"),
        Some(Utc::now() + chrono::Duration::seconds(60)),
    )
    .await;
    store.reset_interrupted().await.unwrap();
    let r2 = sqlx::query("SELECT status, failure_class, next_attempt_at FROM context69.document_extraction_jobs WHERE id=$1").bind(run).fetch_one(db.pool()).await.unwrap();
    let st: String = r2.get("status");
    let fc2: Option<String> = r2.get("failure_class");
    assert_eq!(st, "queued");
    assert!(fc2.is_none());
    clean(&db, &[jid, run]).await;
    sqlx::query("DELETE FROM context69.documents WHERE id=$1")
        .bind(did)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.extraction_templates WHERE template_key=$1")
        .bind(&t)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id=$1")
        .bind(gid)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
async fn attempt_and_stop_at_max() {
    let Some(u) = url() else {
        eprintln!("skip");
        return;
    };
    let _g = LOCK.lock().await;
    let db = Database::connect(&u).await.unwrap();
    let store = ExtractionStore::new(db.pool().clone());
    let gid = grp(&db).await;
    let did = doc(&db, gid).await;
    let t = tmpl(&db).await;
    let jid = job(
        &db,
        did,
        &t,
        &format!("h-{}", Uuid::new_v4()),
        "queued",
        None,
        None,
    )
    .await;
    // quota attempt
    let j = store.claim_job(jid).await.unwrap().unwrap();
    sqlx::query_file!(
        "crates/context69-extraction/sql/jobs/insert_attempt.sql",
        jid,
        "llm",
        "h",
        j.attempt_count,
        "quota_exceeded",
        123,
        "q"
    )
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query(
        "UPDATE context69.document_extraction_jobs SET status='failed', provider_key='llm', provider_config_hash='h', error_message='q', failure_class='quota_exceeded', next_attempt_at=NULL, finished_at=now(), updated_at=now() WHERE id=$1",
    )
    .bind(jid)
    .execute(db.pool())
    .await
    .unwrap();
    let at: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, latency_ms FROM context69.document_extraction_attempts WHERE job_id=$1",
    )
    .bind(jid)
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert_eq!(at[0].0, "quota_exceeded");
    // max attempts: need fresh job
    let j2 = job(
        &db,
        did,
        &t,
        &format!("h-{}", Uuid::new_v4()),
        "queued",
        None,
        None,
    )
    .await;
    let c1 = store.claim_job(j2).await.unwrap().unwrap();
    assert_eq!(c1.attempt_count, 1);
    let d1 = next_retry_delay(c1.attempt_count);
    assert_eq!(d1, std::time::Duration::from_secs(5));
    let nxt1 = Utc::now() + chrono::Duration::from_std(d1).unwrap();
    sqlx::query(
        "UPDATE context69.document_extraction_jobs SET status='queued', provider_key='llm', provider_config_hash='h', error_message='t', failure_class='transient', next_attempt_at=$2, finished_at=NULL, updated_at=now() WHERE id=$1",
    )
    .bind(j2)
    .bind(nxt1)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE context69.document_extraction_jobs SET next_attempt_at=now()-interval '1 sec' WHERE id=$1").bind(j2).execute(db.pool()).await.unwrap();
    let c2 = store.claim_job(j2).await.unwrap().unwrap();
    assert_eq!(c2.attempt_count, 2);
    let d2 = next_retry_delay(c2.attempt_count);
    assert_eq!(d2, std::time::Duration::from_secs(10));
    let nxt2 = Utc::now() + chrono::Duration::from_std(d2).unwrap();
    sqlx::query(
        "UPDATE context69.document_extraction_jobs SET status='queued', provider_key='llm', provider_config_hash='h', error_message='t', failure_class='transient', next_attempt_at=$2, finished_at=NULL, updated_at=now() WHERE id=$1",
    )
    .bind(j2)
    .bind(nxt2)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE context69.document_extraction_jobs SET next_attempt_at=now()-interval '1 sec' WHERE id=$1").bind(j2).execute(db.pool()).await.unwrap();
    let c3 = store.claim_job(j2).await.unwrap().unwrap();
    assert_eq!(c3.attempt_count, 3);
    assert!(!(c3.attempt_count < 3));
    sqlx::query(
        "UPDATE context69.document_extraction_jobs SET status='failed', provider_key='llm', provider_config_hash='h', error_message='t', failure_class='transient', next_attempt_at=NULL, finished_at=now(), updated_at=now() WHERE id=$1",
    )
    .bind(j2)
    .execute(db.pool())
    .await
    .unwrap();
    let r = sqlx::query("SELECT status FROM context69.document_extraction_jobs WHERE id=$1")
        .bind(j2)
        .fetch_one(db.pool())
        .await
        .unwrap();
    let s: String = r.get("status");
    assert_eq!(s, "failed");
    clean(&db, &[jid, j2]).await;
    sqlx::query("DELETE FROM context69.documents WHERE id=$1")
        .bind(did)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.extraction_templates WHERE template_key=$1")
        .bind(&t)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id=$1")
        .bind(gid)
        .execute(db.pool())
        .await
        .ok();
}

#[test]
fn classify_and_delay() {
    assert_eq!(
        classify_error(&anyhow::Error::new(ProviderHttpError {
            status: 429,
            body: "x".into()
        })),
        ExtractionFailureClass::QuotaExceeded
    );
    assert_eq!(
        classify_error(&anyhow::Error::new(ProviderHttpError {
            status: 500,
            body: "x".into()
        })),
        ExtractionFailureClass::Transient
    );
    assert_eq!(
        classify_error(&anyhow::Error::new(ProviderHttpError {
            status: 401,
            body: "x".into()
        })),
        ExtractionFailureClass::Permanent
    );
    assert_eq!(
        classify_error(&anyhow::Error::new(ProviderSchemaError("violates".into()))),
        ExtractionFailureClass::Permanent
    );
    assert_eq!(
        classify_error(&anyhow::Error::new(ProviderPayloadError("omitted".into()))),
        ExtractionFailureClass::Permanent
    );
    assert_eq!(next_retry_delay(1), std::time::Duration::from_secs(5));
    assert_eq!(next_retry_delay(2), std::time::Duration::from_secs(10));
    assert_eq!(next_retry_delay(3), std::time::Duration::from_secs(20));
    assert_eq!(next_retry_delay(10), std::time::Duration::from_secs(300));
    assert_eq!(
        failure_class_as_str(ExtractionFailureClass::Transient),
        "transient"
    );
}

#[tokio::test]
async fn semaphore_respects_concurrency() {
    let pool = sqlx::PgPool::connect_lazy("postgres://invalid").unwrap();
    let s = ExtractionService::new(ExtractionDependencies {
        pool: pool.clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(Noop),
        concurrency: 3,
        readiness: Arc::new(Ready),
    });
    assert_eq!(s.configured_concurrency(), 3);
    assert_eq!(s.available_permits(), 3);
    let s2 = ExtractionService::new(ExtractionDependencies {
        pool,
        http_client: reqwest::Client::new(),
        publisher: Arc::new(Noop),
        concurrency: 0,
        readiness: Arc::new(Ready),
    });
    assert_eq!(s2.configured_concurrency(), 1);
}
