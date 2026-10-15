//! Typed extraction/decision template registry regression (issue #89 P4).
//!
//! Proves against a real database (not compilation only) that nested
//! extraction registration preserves the generative path byte-identically,
//! legacy stored rows default to extraction, decision jobs fail unavailable
//! before any provider call with no generative fallback, and a template-kind
//! change cannot reinterpret already-persisted results (row snapshot only).
//! Gated by CONTEXT69_TEST_DATABASE_URL and registered in the static
//! inventory; zero model calls (no provider is ever configured here).

use std::sync::Arc;

use async_trait::async_trait;
use context69::contracts::{
    DecisionDefinition, ExtractionQuestion, ExtractionTemplateDefinition, ExtractionTemplateInput,
    PredicateQuestion,
};
use context69::db::Database;
use context69_extraction::{
    EnqueueExtraction, ExtractionCoordinator, ExtractionDependencies, ExtractionPublication,
    ExtractionPublisher, ExtractionReadiness, ExtractionService, ExtractionStore,
    ExtractionVersionInput,
};
use sqlx::Row;
use uuid::Uuid;

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Ready;
#[async_trait]
impl ExtractionReadiness for Ready {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        Ok(true)
    }
}

struct NoopPublisher;
#[async_trait]
impl ExtractionPublisher for NoopPublisher {
    async fn publish(&self, _publication: &ExtractionPublication<'_>) -> anyhow::Result<()> {
        Ok(())
    }
}

async fn db() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

async fn seed_group(db: &Database) -> i64 {
    let k = format!("g-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1,$2,'public','personal',$3) RETURNING id",
    )
    .bind(&k)
    .bind(&k)
    .bind(format!("/{k}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id")
}

async fn seed_document(db: &Database, group_id: i64) -> i64 {
    let record_hash = format!("h-{}", Uuid::new_v4());
    let document_id: i64 = sqlx::query(
        "INSERT INTO context69.documents (source_key, external_id, title, source_uri, updated_at_source, metadata_json, record_hash, group_id, visibility) VALUES ($1,$2,'t','u',now(),'{}',$3,$4,'public') RETURNING id",
    )
    .bind(format!("s-{}", Uuid::new_v4()))
    .bind(format!("e-{}", Uuid::new_v4()))
    .bind(&record_hash)
    .bind(group_id)
    .fetch_one(db.pool())
    .await
    .expect("seed document")
    .get("id");
    // The extraction document projection joins the matching version snapshot,
    // so the seeded document needs its version row to be convertible.
    sqlx::query(
        "INSERT INTO context69.document_versions (document_id, record_hash, title, summary, body_text, source_uri, metadata_json) VALUES ($1,$2,'t',NULL,'body','u','{}'::jsonb)",
    )
    .bind(document_id)
    .bind(&record_hash)
    .execute(db.pool())
    .await
    .expect("seed document version");
    document_id
}

fn extraction_input(key: &str) -> ExtractionTemplateInput {
    ExtractionTemplateInput {
        template_key: key.to_string(),
        description: None,
        enabled: true,
        definition: ExtractionTemplateDefinition::Extraction(
            context69::contracts::ExtractionDefinition {
                system_prompt: "p".to_string(),
                output_schema: serde_json::json!({"type": "object"}),
                max_output_tokens: None,
            },
        ),
    }
}

fn decision_input(key: &str) -> ExtractionTemplateInput {
    ExtractionTemplateInput {
        template_key: key.to_string(),
        description: Some("decide".to_string()),
        enabled: true,
        definition: ExtractionTemplateDefinition::Decision(DecisionDefinition {
            questions: vec![ExtractionQuestion::Predicate(PredicateQuestion {
                name: "relevant".to_string(),
                instructions: "Is it relevant?".to_string(),
            })],
        }),
    }
}

fn service(db: &Database) -> ExtractionService {
    ExtractionService::new(ExtractionDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopPublisher),
        concurrency: 1,
        readiness: Arc::new(Ready),
    })
}

async fn cleanup(db: &Database, group_id: i64, document_id: i64, template_key: &str) {
    for job_id in sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM context69.document_extraction_jobs WHERE document_id = $1",
    )
    .bind(document_id)
    .fetch_all(db.pool())
    .await
    .unwrap_or_default()
    {
        sqlx::query("DELETE FROM context69.document_extraction_attempts WHERE job_id = $1")
            .bind(job_id)
            .execute(db.pool())
            .await
            .ok();
    }
    sqlx::query("DELETE FROM context69.document_extraction_jobs WHERE document_id = $1")
        .bind(document_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.document_extraction_versions WHERE document_id = $1")
        .bind(document_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.documents WHERE id = $1")
        .bind(document_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.extraction_templates WHERE template_key = $1")
        .bind(template_key)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn nested_extraction_register_preserves_generative_shape() {
    let _guard = LOCK.lock().await;
    let db = db().await;
    let store = ExtractionStore::new(db.pool().clone());
    let group_id = seed_group(&db).await;
    let key = format!("t-{}", Uuid::new_v4());

    let first = store
        .register_template(&extraction_input(&key))
        .await
        .expect("register v1");
    assert_eq!(first.version, 1);
    match first.definition {
        ExtractionTemplateDefinition::Extraction(definition) => {
            assert_eq!(definition.system_prompt, "p");
            assert_eq!(
                definition.output_schema,
                serde_json::json!({"type": "object"})
            );
            assert_eq!(definition.max_output_tokens, Some(8192));
        }
        other => panic!("nested extraction must stay generative, got {other:?}"),
    }
    // Stored branch columns carry the extraction kind with no decision blob.
    let stored = sqlx::query_file!("tests/sql/extraction_templates/get_template_kind.sql", key,)
        .fetch_one(db.pool())
        .await
        .expect("stored branch columns");
    assert_eq!(stored.kind, "extraction");
    assert!(stored.decision_definition.is_none());

    let second = store
        .register_template(&extraction_input(&key))
        .await
        .expect("register v2");
    assert_eq!(second.version, 2);
    let listed = service(&db).templates().await.expect("list");
    assert!(
        listed
            .iter()
            .any(|row| row.template_key == key && row.version == 2)
    );

    sqlx::query("DELETE FROM context69.extraction_templates WHERE template_key = $1")
        .bind(&key)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn legacy_stored_row_defaults_to_extraction() {
    let _guard = LOCK.lock().await;
    let db = db().await;
    let store = ExtractionStore::new(db.pool().clone());
    let group_id = seed_group(&db).await;
    let key = format!("legacy-{}", Uuid::new_v4());

    // A pre-P4 row carries only the legacy generative columns; the kind
    // default and codec keep its exact prompt/schema/cap and execute it
    // generatively.
    sqlx::query(
        "INSERT INTO context69.extraction_templates (template_key, version, system_prompt, output_schema, enabled) VALUES ($1,1,'legacy-p','{\"type\":\"object\"}',true)",
    )
    .bind(&key)
    .execute(db.pool())
    .await
    .expect("legacy row");
    let template = store.template(&key).await.expect("read").expect("row");
    assert_eq!(template.kind, "extraction");
    let listed = service(&db).templates().await.expect("list");
    let response = listed
        .into_iter()
        .find(|row| row.template_key == key)
        .expect("legacy template lists");
    match response.definition {
        ExtractionTemplateDefinition::Extraction(definition) => {
            assert_eq!(definition.system_prompt, "legacy-p");
            assert_eq!(
                definition.output_schema,
                serde_json::json!({"type": "object"})
            );
        }
        other => panic!("legacy row must decode generative, got {other:?}"),
    }

    sqlx::query("DELETE FROM context69.extraction_templates WHERE template_key = $1")
        .bind(&key)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn decision_job_fails_unavailable_before_provider_call() {
    let _guard = LOCK.lock().await;
    let db = db().await;
    let group_id = seed_group(&db).await;
    let document_id = seed_document(&db, group_id).await;
    let key = format!("d-{}", Uuid::new_v4());
    let svc = service(&db);
    let store = ExtractionStore::new(db.pool().clone());

    let registered = store
        .register_template(&decision_input(&key))
        .await
        .expect("register decision");
    assert!(matches!(
        registered.definition,
        ExtractionTemplateDefinition::Decision(_)
    ));
    let stored = sqlx::query_file!("tests/sql/extraction_templates/get_template_kind.sql", key,)
        .fetch_one(db.pool())
        .await
        .expect("stored branch columns");
    assert_eq!(stored.kind, "decision");
    assert!(stored.decision_definition.is_some());

    // No provider is configured in this database, yet the failure must name
    // the decision provider (branch before the generative lookup), carry the
    // typed permanent class, and perform zero HTTP: the branch returns before
    // any provider fetch or transport use.
    let responses = svc
        .convert(EnqueueExtraction {
            document_id,
            directive: context69::contracts::ExtractionDirective {
                template_key: key.clone(),
                parameters: serde_json::json!({}),
                required: true,
            },
        })
        .await
        .expect("convert returns terminal data");
    assert_eq!(responses.len(), 1);
    let job = &responses[0];
    assert_eq!(
        job.status,
        context69::contracts::ExtractionJobStatus::Failed
    );
    assert_eq!(
        job.failure_class,
        Some(context69::contracts::ExtractionFailureClass::Permanent)
    );
    assert_eq!(
        job.error_message.as_deref(),
        Some("decision provider is not configured")
    );

    cleanup(&db, group_id, document_id, &key).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn template_kind_change_cannot_reinterpret_history() {
    let _guard = LOCK.lock().await;
    let db = db().await;
    let group_id = seed_group(&db).await;
    let document_id = seed_document(&db, group_id).await;
    let key = format!("k-{}", Uuid::new_v4());
    let store = ExtractionStore::new(db.pool().clone());

    store
        .register_template(&extraction_input(&key))
        .await
        .expect("register extraction v1");
    // Publish a generative version directly: no model call, only the row the
    // codec must keep decoding generatively after the kind change.
    let stored_value = serde_json::json!({"direction": "positive"});
    store
        .publish_version(&ExtractionVersionInput {
            id: Uuid::new_v4(),
            document_id,
            template_key: &key,
            template_version: 1,
            source_record_hash: "h",
            provider_key: "llm",
            provider_config_hash: "c",
            model_name: None,
            result_json: &stored_value,
            template_kind: "extraction",
        })
        .await
        .expect("publish generative version");

    // Change the template kind: the live row becomes decision v2 while the
    // persisted version row keeps its extraction snapshot.
    let second = store
        .register_template(&decision_input(&key))
        .await
        .expect("register decision v2");
    assert_eq!(second.version, 2);

    let rows = store
        .results_for_document(document_id)
        .await
        .expect("results");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].template_kind, "extraction");

    cleanup(&db, group_id, document_id, &key).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn decision_register_rejects_invalid_questions() {
    let _guard = LOCK.lock().await;
    let db = db().await;
    let store = ExtractionStore::new(db.pool().clone());
    let group_id = seed_group(&db).await;
    let key = format!("bad-{}", Uuid::new_v4());

    let mut invalid = decision_input(&key);
    if let ExtractionTemplateDefinition::Decision(definition) = &mut invalid.definition {
        definition.questions.clear();
    }
    assert!(
        store.register_template(&invalid).await.is_err(),
        "empty questions must not register"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.extraction_templates WHERE template_key = $1",
    )
    .bind(&key)
    .fetch_one(db.pool())
    .await
    .expect("count");
    assert_eq!(count, 0);

    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}
