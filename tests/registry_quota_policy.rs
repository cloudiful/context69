//! The persisted registry capacity policy (issue #1138 P27).
//!
//! The policy is one persisted capacity of the search registry: three caps
//! that bound how many vector point ids the registry admits for the whole
//! deployment, for one file and for one ingestion run. The policy is
//! cumulative, so it counts the retained entries and their tombstones, and it
//! is independent of the size of an upload.
//!
//! The cases prove the properties the deployment depends on: a boot seeds the
//! policy, an upgraded installation keeps the same policy, a client that does
//! not know the setting never resets it, an invalid policy is refused before
//! anything is written, a caller can only narrow the policy, and a service
//! that holds an older policy cannot exceed the current one.

#[path = "registry_quota_policy/support.rs"]
mod support;

#[cfg(feature = "integration-test-helpers")]
#[path = "library_ingest_switch/harness.rs"]
mod harness;

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

use context69::db::{AdmissionOutcome, Database, RegistryQuotaOverride};
use context69_search::SearchRegistryEntry;
use uuid::Uuid;

use support::{clear_file_library_settings, connect, runtime_settings, store_quota, stored_quota};

/// A staging registry entry with the given staged point ids.
fn registry_entry(file_id: Uuid, staged: usize) -> SearchRegistryEntry {
    SearchRegistryEntry {
        run_id: Uuid::new_v4(),
        file_id: Some(file_id),
        document_ids: Vec::new(),
        staged_ids: (0..staged).map(|_| Uuid::new_v4()).collect(),
        retired_ids: Vec::new(),
        baseline_record_hash: None,
        staged_record_hash: None,
        generation: 1,
        phase: "staging".to_string(),
    }
}

/// The domain error of a failed call, if it carries one.
fn domain_error(error: &anyhow::Error) -> Option<context69::domain_errors::DomainError> {
    context69::domain_errors::find_domain_error(error).cloned()
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_boot_seeds_the_configured_capacity_policy() {
    let db = connect().await;
    clear_file_library_settings(&db).await;
    let had_identity = support::identity_rows_exist(&db).await;

    // The configuration of a deployment carries the policy, and the boot
    // writes it into the runtime row the admission reads.
    db.save_runtime_settings(&runtime_settings(&db, Some((4096, 2048, 1024))).await)
        .await
        .expect("seed the runtime settings");

    assert_eq!(stored_quota(&db).await, (4096, 2048, 1024));
    let policy = db.read_registry_quota().await.expect("read the policy");
    assert_eq!(policy.max_ids_total, 4096);
    assert_eq!(policy.max_ids_per_file, 2048);
    assert_eq!(policy.max_ids_per_run, 1024);

    clear_file_library_settings(&db).await;
    if !had_identity {
        support::clear_identity_rows(&db).await;
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_upgraded_installation_keeps_the_documented_policy() {
    let db = connect().await;
    clear_file_library_settings(&db).await;

    // A row that predates the policy takes the migration defaults, which are
    // the same values a fresh boot seeds.
    sqlx::query(
        "INSERT INTO context69.runtime_file_library_settings \
         (singleton, storage_root, max_upload_size_mb) \
         VALUES (TRUE, '/tmp/quota-policy', 16)",
    )
    .execute(db.pool())
    .await
    .expect("seed an upgraded file-library row");

    assert_eq!(
        stored_quota(&db).await,
        (
            context69::config::DEFAULT_REGISTRY_MAX_IDS_TOTAL,
            context69::config::DEFAULT_REGISTRY_MAX_IDS_PER_FILE,
            context69::config::DEFAULT_REGISTRY_MAX_IDS_PER_RUN,
        )
    );

    clear_file_library_settings(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_client_without_the_setting_keeps_the_stored_policy() {
    let db = connect().await;
    clear_file_library_settings(&db).await;
    let had_identity = support::identity_rows_exist(&db).await;
    store_quota(&db, (4096, 2048, 1024)).await;

    // A request that does not mention the caps keeps the stored ones.
    db.save_runtime_settings(&runtime_settings(&db, None).await)
        .await
        .expect("save the settings without the capacity caps");

    assert_eq!(stored_quota(&db).await, (4096, 2048, 1024));

    clear_file_library_settings(&db).await;
    if !had_identity {
        support::clear_identity_rows(&db).await;
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_invalid_policy_is_refused_before_any_write() {
    let db = connect().await;
    clear_file_library_settings(&db).await;
    let had_identity = support::identity_rows_exist(&db).await;
    store_quota(&db, (4096, 2048, 1024)).await;

    // A run cap above the file cap is not a policy.
    let error = db
        .save_runtime_settings(&runtime_settings(&db, Some((4096, 2048, 4096))).await)
        .await
        .expect_err("an unordered policy must be refused");
    let domain = domain_error(&error).expect("the refusal carries a domain error");
    assert!(
        matches!(
            domain,
            context69::domain_errors::DomainError::InvalidArgument(_)
        ),
        "the refusal must be deterministic, got {domain:?}"
    );

    // The refused save changed nothing.
    assert_eq!(stored_quota(&db).await, (4096, 2048, 1024));

    // A cap of zero is not a policy either, and it never disables the
    // registry silently.
    let error = db
        .save_runtime_settings(&runtime_settings(&db, Some((4096, 2048, 0))).await)
        .await
        .expect_err("a zero cap must be refused");
    assert!(domain_error(&error).is_some());
    assert_eq!(stored_quota(&db).await, (4096, 2048, 1024));

    clear_file_library_settings(&db).await;
    if !had_identity {
        support::clear_identity_rows(&db).await;
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_caller_cannot_raise_the_stored_caps() {
    let db = connect().await;
    clear_file_library_settings(&db).await;
    store_quota(&db, (1000, 1000, 1000)).await;

    let file_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.expect("begin the admission");
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(file_id, 2),
            None,
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await
        .expect("admit the staging entry");
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    tx.commit().await.expect("commit the admission");

    // The stored policy is now smaller than the caller's request.
    store_quota(&db, (1000, 1000, 1)).await;

    let mut tx = db.pool().begin().await.expect("begin the admission");
    let refused = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(file_id, 2),
            None,
            0,
            RegistryQuotaOverride::exact([1000, 1000, 1000]),
            &mut tx,
        )
        .await
        .expect("the admission reports the refusal");
    assert!(
        matches!(refused, AdmissionOutcome::CapsRefused),
        "a caller must not raise the stored cap"
    );
    tx.rollback()
        .await
        .expect("roll the refused admission back");

    clear_file_library_settings(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_lowered_policy_keeps_the_retained_entries_and_refuses_new_work() {
    let db = connect().await;
    clear_file_library_settings(&db).await;
    store_quota(&db, (1000, 1000, 1000)).await;

    let file_id = Uuid::new_v4();
    let entry = registry_entry(file_id, 1);
    let mut tx = db.pool().begin().await.expect("begin the admission");
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &entry,
            None,
            0,
            RegistryQuotaOverride::none(),
            &mut tx,
        )
        .await
        .expect("admit the staging entry");
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    tx.commit().await.expect("commit the admission");

    // Lowering the policy keeps the entries that already exist.
    store_quota(&db, (1000, 1000, 1)).await;
    let state = db
        .read_search_registry_state()
        .await
        .expect("read the registry state");
    let retained = state
        .entries
        .iter()
        .find(|candidate| candidate.run_id == entry.run_id)
        .expect("the retained entry must survive a lowered policy");
    assert_eq!(retained.staged_ids, entry.staged_ids);

    // A new admission does not fit anymore and is refused.
    let mut tx = db.pool().begin().await.expect("begin the admission");
    let refused = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(file_id, 2),
            None,
            0,
            RegistryQuotaOverride::none(),
            &mut tx,
        )
        .await
        .expect("the admission reports the refusal");
    assert!(matches!(refused, AdmissionOutcome::CapsRefused));
    tx.rollback()
        .await
        .expect("roll the refused admission back");

    // The refusal left the retained entry untouched.
    let state = db
        .read_search_registry_state()
        .await
        .expect("read the registry state");
    let retained = state
        .entries
        .iter()
        .find(|candidate| candidate.run_id == entry.run_id)
        .expect("the retained entry must survive the refused admission");
    assert_eq!(retained.staged_ids, entry.staged_ids);

    clear_file_library_settings(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_service_with_an_older_policy_cannot_exceed_the_current_one() {
    let db = connect().await;
    clear_file_library_settings(&db).await;
    store_quota(&db, (1000, 1000, 1000)).await;

    // The service captured its caps before the policy changed.
    let captured = RegistryQuotaOverride::exact([1000, 1000, 1000]);
    let file_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.expect("begin the admission");
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(file_id, 1),
            None,
            0,
            captured,
            &mut tx,
        )
        .await
        .expect("admit the staging entry");
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    tx.commit().await.expect("commit the admission");

    // Another node lowers the policy. The service still holds the old caps.
    store_quota(&db, (1000, 1000, 1)).await;

    let mut tx = db.pool().begin().await.expect("begin the admission");
    let refused = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(file_id, 2),
            None,
            0,
            captured,
            &mut tx,
        )
        .await
        .expect("the admission reports the refusal");
    assert!(
        matches!(refused, AdmissionOutcome::CapsRefused),
        "a stale service must not exceed the current policy"
    );
    tx.rollback()
        .await
        .expect("roll the refused admission back");

    clear_file_library_settings(&db).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_admission_without_a_stored_row_uses_the_documented_policy() {
    let db: Database = connect().await;
    clear_file_library_settings(&db).await;

    // An installation without a runtime row has no persisted policy yet, so
    // the admission uses the documented defaults instead of an unbounded one.
    let policy = db.read_registry_quota().await.expect("read the policy");
    assert_eq!(policy.max_ids_total, 32768);
    assert_eq!(policy.max_ids_per_file, 16384);
    assert_eq!(policy.max_ids_per_run, 8192);

    let file_id = Uuid::new_v4();
    let mut tx = db.pool().begin().await.expect("begin the admission");
    let admitted = db
        .admit_search_registry_entry_checked_in(
            &registry_entry(file_id, 1),
            None,
            0,
            RegistryQuotaOverride::none(),
            &mut tx,
        )
        .await
        .expect("admit the staging entry");
    assert!(matches!(admitted, AdmissionOutcome::Admitted(_)));
    tx.commit().await.expect("commit the admission");
}

/// A replacement that the capacity policy refuses is a deterministic failure
/// that keeps the old data.
///
/// The refusal happens before the first point write, so the run stops without
/// a retry: the task item terminates and retires its own staging, while the
/// live file, its documents and their searchable vectors stay as they were.
#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_refused_replacement_is_deterministic_and_keeps_the_old_data() {
    use async_trait::async_trait;
    use context69::chunking::{ChunkBudget, ChunkingConfig};
    use context69::db::IngestRunSource;
    use context69::embedding::EmbeddingProvider;
    use context69::qdrant_index::QdrantIndex;
    use context69::services::library::{
        ReplacementRunRequest, ReplacementSectionInput, ReplacementVector, run_replacement,
    };

    const DIMENSIONS: usize = 8;

    struct FakeEmbedding;

    #[async_trait]
    impl EmbeddingProvider for FakeEmbedding {
        async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| vec![text.len() as f32; DIMENSIONS])
                .collect())
        }
    }

    let fixture = harness::SwitchFixture::new().await;
    let mock = qdrant_points_mock::QdrantPointsMock::start().await;
    // The persisted policy admits one point id per run, and the replacement
    // below stages more than one.
    store_quota(&fixture.db, (1000, 1000, 1)).await;
    let before = fixture.file_source().await;

    let embedding = FakeEmbedding;
    let index = QdrantIndex::for_test_unreachable(mock.url(), "quota-policy", DIMENSIONS)
        .expect("build the test index");
    // The run captures the vector space of the deployment it writes under, so
    // the admission compares it with the persisted configuration.
    let identity = support::registry_identity(&fixture.db)
        .await
        .unwrap_or_else(support::fallback_identity);
    let chunk_budget = ChunkBudget::chars(ChunkingConfig::default());
    let vector = ReplacementVector {
        embedding: &embedding,
        index: &index,
        chunking: &chunk_budget,
        identity: Some(identity),
    };
    let staged = fixture.attach_staged_object().await;
    let request = ReplacementRunRequest {
        task_id: fixture.task_id,
        file_id: fixture.file_id,
        source: IngestRunSource::Stored,
        staged_source_object: Some(staged),
        quota: RegistryQuotaOverride::none(),
        attributes: None,
        sections: vec![
            ReplacementSectionInput {
                section_key: "one".into(),
                section_label: "one".into(),
                sort_order: 0,
                section_metadata_json: serde_json::json!({}),
                title: "one".into(),
                summary: None,
                body_text: "one".into(),
                source_uri: Some("context69://quota/one".into()),
                external_id: Some("one".into()),
                published_at: None,
            },
            ReplacementSectionInput {
                section_key: "two".into(),
                section_label: "two".into(),
                sort_order: 1,
                section_metadata_json: serde_json::json!({}),
                title: "two".into(),
                summary: None,
                body_text: "two".into(),
                source_uri: Some("context69://quota/two".into()),
                external_id: Some("two".into()),
                published_at: None,
            },
        ],
    };

    let error = run_replacement(
        &fixture.db,
        &fixture.store,
        &vector,
        fixture.fence,
        &request,
    )
    .await
    .expect_err("the capacity policy must refuse the replacement");
    let domain = domain_error(&error).expect("the refusal carries a domain error");
    assert!(
        matches!(
            domain,
            context69::domain_errors::DomainError::InvalidArgument(_)
        ),
        "a capacity refusal must be deterministic, got {domain:?}"
    );

    // The refusal happened before the first point write.
    assert!(
        mock.upsert_calls().is_empty(),
        "a refused replacement must not write a point"
    );
    // The live file kept its source and its status.
    let after = fixture.file_source().await;
    assert_eq!(after.ingest_status, before.ingest_status);
    assert_eq!(after.sha256, before.sha256);
    assert_eq!(after.storage_rel_path, before.storage_rel_path);

    clear_file_library_settings(&fixture.db).await;
}
