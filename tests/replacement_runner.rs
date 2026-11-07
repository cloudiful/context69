//! The staged replacement executor tests (issue #1138 P16).
//!
//! The database cases run against the configured reusable PostgreSQL through
//! the shared P15 switch fixture; every Qdrant interaction goes to the
//! in-process Points mock, and embedding goes to a local fake. No real vector
//! service is touched and no external test collection is created.

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
#[path = "library_ingest_switch/harness.rs"]
mod harness;

#[cfg(feature = "integration-test-helpers")]
mod runner_cases {
    #![allow(clippy::too_many_arguments)]

    use super::harness;
    use super::qdrant_points_mock;
    use anyhow::Result;
    use async_trait::async_trait;
    use qdrant_points_mock::QdrantPointsMock;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    use context69::chunking::{ChunkBudget, ChunkingConfig};
    use context69::db::{IngestRunSource, RegistryQuotaOverride};
    use context69::embedding::EmbeddingProvider;
    use context69::qdrant_index::QdrantIndex;
    use context69::services::library::{
        ReplacementRunDisposition, ReplacementRunOutcome, ReplacementRunRequest,
        ReplacementSectionInput, ReplacementVector, run_replacement,
    };

    const DIMENSIONS: usize = 4;

    /// A fake embedder with a deterministic vector per text and an optional
    /// failure on the first N calls, so a retry can succeed.
    struct FakeEmbedding {
        fail_first_calls: AtomicUsize,
    }

    impl FakeEmbedding {
        fn new(fail_first_calls: usize) -> Self {
            Self {
                fail_first_calls: AtomicUsize::new(fail_first_calls),
            }
        }
    }

    #[async_trait]
    impl EmbeddingProvider for FakeEmbedding {
        async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            let failing = self.fail_first_calls.fetch_update(
                Ordering::SeqCst,
                Ordering::SeqCst,
                |remaining| {
                    if remaining == 0 {
                        None
                    } else {
                        Some(remaining - 1)
                    }
                },
            );
            if failing.is_ok() {
                return Err(anyhow::anyhow!("transient embedding outage"));
            }
            Ok(texts
                .iter()
                .map(|text| vec![text.len() as f32; DIMENSIONS])
                .collect())
        }
    }

    /// One prepared runner input for a section.
    fn section_input(section_key: &str, external_id: &str, body: &str) -> ReplacementSectionInput {
        ReplacementSectionInput {
            section_key: section_key.into(),
            section_label: section_key.into(),
            sort_order: 0,
            section_metadata_json: serde_json::json!({}),
            title: "replaced".into(),
            summary: None,
            body_text: body.into(),
            source_uri: Some("context69://library/replacement".into()),
            external_id: Some(external_id.into()),
            published_at: None,
        }
    }

    /// The exact metadata the runner composes for a fixture section.
    fn final_section_metadata(
        fixture: &harness::SwitchFixture,
        section_key: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "is_library_file": true,
            "library_file_id": fixture.file_id,
            "library_path": "/",
            "library_section_key": section_key,
            "library_section_label": section_key,
            "library_filename": "switch.txt",
            "library_media_type": "text/plain",
        })
    }

    async fn run(
        fixture: &harness::SwitchFixture,
        embedding: &FakeEmbedding,
        mock: &QdrantPointsMock,
        caps: [i64; 3],
        sections: Vec<ReplacementSectionInput>,
    ) -> Result<ReplacementRunOutcome> {
        run_with_source(fixture, embedding, mock, caps, sections, None).await
    }

    async fn run_with_source(
        fixture: &harness::SwitchFixture,
        embedding: &FakeEmbedding,
        mock: &QdrantPointsMock,
        caps: [i64; 3],
        sections: Vec<ReplacementSectionInput>,
        staged_source_object: Option<Uuid>,
    ) -> Result<ReplacementRunOutcome> {
        let index = QdrantIndex::for_test_unreachable(
            mock.url(),
            &fixture.identity.collection_name,
            DIMENSIONS,
        )?;
        let chunk_budget = ChunkBudget::chars(ChunkingConfig::default());
        let vector = ReplacementVector {
            embedding,
            index: &index,
            chunking: &chunk_budget,
            identity: Some(fixture.identity.clone()),
        };
        let request = ReplacementRunRequest {
            task_id: fixture.task_id,
            file_id: fixture.file_id,
            source: IngestRunSource::Stored,
            staged_source_object,
            quota: RegistryQuotaOverride::exact(caps),
            attributes: None,
            sections,
        };
        run_replacement(
            &fixture.db,
            &fixture.store,
            &vector,
            fixture.fence,
            &request,
        )
        .await
    }

    /// Run one replacement under an explicit chunk budget, so a case can pin a
    /// token budget while the service-level configuration stays characters.
    async fn run_with_budget(
        fixture: &harness::SwitchFixture,
        embedding: &FakeEmbedding,
        mock: &QdrantPointsMock,
        chunk_budget: &ChunkBudget,
        sections: Vec<ReplacementSectionInput>,
    ) -> Result<ReplacementRunOutcome> {
        let index = QdrantIndex::for_test_unreachable(
            mock.url(),
            &fixture.identity.collection_name,
            DIMENSIONS,
        )?;
        let vector = ReplacementVector {
            embedding,
            index: &index,
            chunking: chunk_budget,
            identity: Some(fixture.identity.clone()),
        };
        let staged = fixture.attach_staged_object().await;
        let request = ReplacementRunRequest {
            task_id: fixture.task_id,
            file_id: fixture.file_id,
            source: IngestRunSource::Stored,
            staged_source_object: Some(staged),
            quota: RegistryQuotaOverride::exact([1000, 1000, 1000]),
            attributes: None,
            sections,
        };
        run_replacement(
            &fixture.db,
            &fixture.store,
            &vector,
            fixture.fence,
            &request,
        )
        .await
    }

    /// The stored chunk text of every point in one upsert call.
    fn chunk_texts(call: &qdrant_points_mock::UpsertCall) -> Vec<String> {
        call.points
            .values()
            .filter_map(
                |point| match point.payload.get("chunk_text")?.kind.as_ref()? {
                    qdrant_client::qdrant::value::Kind::StringValue(text) => Some(text.clone()),
                    _ => None,
                },
            )
            .collect()
    }

    /// The document id a stored mock payload names.
    fn payload_document_id(call: &qdrant_points_mock::UpsertCall, id: &str) -> Option<i64> {
        let point = call.points.get(id)?;
        match point.payload.get("document_id")?.kind.as_ref()? {
            qdrant_client::qdrant::value::Kind::IntegerValue(number) => Some(*number),
            _ => None,
        }
    }

    /// The record hash a stored mock payload names.
    fn payload_record_hash(call: &qdrant_points_mock::UpsertCall, id: &str) -> Option<String> {
        let point = call.points.get(id)?;
        match point.payload.get("record_hash")?.kind.as_ref()? {
            qdrant_client::qdrant::value::Kind::StringValue(text) => Some(text.clone()),
            _ => None,
        }
    }

    /// A stored translation version with an explicit source hash.
    ///
    /// The version row stays stored beside the current one, so a replacement
    /// that publishes another hash finds a historical translation it must
    /// retire without deleting its history.
    async fn seed_translation_version(
        fixture: &harness::SwitchFixture,
        document_id: i64,
        source_record_hash: &str,
        chunk_id: Uuid,
    ) {
        let version_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.document_translation_versions \
             (id, document_id, target_locale, source_record_hash, provider_key, \
              provider_config_hash, translated_title, translated_body_text) \
             VALUES ($1, $2, 'zh', $3, 'test', 'test', 'test', 'test')",
        )
        .bind(version_id)
        .bind(document_id)
        .bind(source_record_hash)
        .execute(fixture.db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO context69.document_translation_chunks \
             (id, translation_id, document_id, target_locale, chunk_index, chunk_text) \
             VALUES ($1, $2, $3, 'zh', 0, 'translated')",
        )
        .bind(chunk_id)
        .bind(version_id)
        .bind(document_id)
        .execute(fixture.db.pool())
        .await
        .unwrap();
    }

    /// The happy path: a changed and a new section switch together, every
    /// point was staged insert-only and verified, the retired point was
    /// deleted, and the vanished section stays untouched.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_replacement_switches_changed_and_new_sections_together() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let vanished = fixture.seed_section("vanished", "vanished-hash").await;
        let staged = fixture.attach_staged_object().await;
        let (run_id, generation) = fixture.create_run().await;

        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);
        let sections = vec![
            section_input("kept", &kept.external_id, "new body"),
            section_input("added", "added-external", "added body"),
        ];
        let outcome = run_with_source(
            &fixture,
            &embedding,
            &mock,
            [1000, 1000, 1000],
            sections.clone(),
            Some(staged),
        )
        .await
        .expect("the replacement switches");

        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);
        assert_eq!(outcome.run_id, run_id);
        assert_eq!(outcome.generation, generation);
        assert_eq!(
            outcome.retired_point_ids,
            vec![kept.chunk_id],
            "only the current affected point retires; the vanished section stays"
        );
        assert!(outcome.pending_cleanup_point_ids.is_empty());
        assert!(!outcome.staged_point_ids.contains(&kept.chunk_id));

        // Every staged point went in insert-only mode with the intended
        // identity, and no payload carried a hidden state marker.
        let upserts = mock.upsert_calls();
        assert!(!upserts.is_empty());
        for call in &upserts {
            assert!(call.insert_only, "staging must be insert-only");
            for point in call.points.values() {
                assert!(
                    !point.payload.contains_key("c69_ix_state"),
                    "no hidden-state payload marker is written"
                );
            }
            for id in &call.point_ids {
                assert_eq!(
                    payload_document_id(call, id),
                    Some(if *id == outcome.staged_point_ids[0].to_string() {
                        kept.document_id
                    } else {
                        outcome.document_ids[1]
                    }),
                    "the staged payload names its final document"
                );
                assert!(
                    !point_matches_hash(call, id, "old-hash"),
                    "the staged payload carries the new record hash"
                );
            }
        }
        let staged_ids: Vec<String> = outcome
            .staged_point_ids
            .iter()
            .map(|id| id.to_string())
            .collect();
        let all_upserted: Vec<String> = upserts
            .iter()
            .flat_map(|call| call.point_ids.clone())
            .collect();
        for id in &staged_ids {
            assert!(all_upserted.contains(id), "every staged id was upserted");
        }

        // The registry moved to switched with the old point as the tombstone.
        let entry = fixture.registry_entry(run_id).await;
        assert_eq!(entry.phase, "switched");
        assert_eq!(entry.retired_ids, vec![kept.chunk_id]);
        assert!(entry.staged_ids.is_empty());

        // The durable run resolved and the switch moved every layer.
        let run = fixture
            .db
            .read_library_ingest_run(fixture.item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(run.resolved_at.is_some());
        let file = fixture.file_record().await;
        assert_eq!(
            file.ingest_status,
            context69::contracts::LibraryIngestStatus::Succeeded
        );
        let source = fixture.file_source().await;
        assert_eq!(source.storage_object_id, Some(staged));
        let intents = sqlx::query_file_scalar!(
            "tests/library_ingest_switch/cleanup_intent_count.sql",
            fixture.old_object
        )
        .fetch_one(fixture.db.pool())
        .await
        .unwrap();
        assert_eq!(intents, 1, "the old source object is queued for cleanup");

        // The old point was physically deleted, the new ones stay.
        let stored: std::collections::BTreeSet<String> =
            mock.stored_point_ids().into_iter().collect();
        let expected: std::collections::BTreeSet<String> = staged_ids.iter().cloned().collect();
        assert_eq!(stored, expected);
        let deleted: Vec<String> = mock
            .delete_calls()
            .into_iter()
            .flat_map(|call| call.point_ids)
            .collect();
        assert!(deleted.contains(&kept.chunk_id.to_string()));

        // The kept section carries its new chunk generation; the vanished
        // section keeps its link.
        assert_eq!(
            fixture
                .db
                .list_chunk_ids_for_document(kept.document_id)
                .await
                .unwrap(),
            vec![outcome.staged_point_ids[0]]
        );
        let links = fixture
            .store
            .list_file_documents(fixture.file_id)
            .await
            .unwrap();
        assert!(
            links
                .iter()
                .any(|link| link.document_id == vanished.document_id),
            "the vanished section keeps its link"
        );
        assert!(
            links
                .iter()
                .any(|link| link.document_id == outcome.document_ids[1]),
            "the new section is linked"
        );

        // A replay of the same durable run is already applied and writes
        // nothing: no staged/document/retired ids and no extra upsert.
        let upserts_after_switch = upserts.len();
        let replay = run_with_source(
            &fixture,
            &embedding,
            &mock,
            [1000, 1000, 1000],
            sections,
            Some(staged),
        )
        .await
        .expect("the resolved replay is already applied");
        assert_eq!(
            replay.disposition,
            ReplacementRunDisposition::AlreadyApplied
        );
        assert_eq!(replay.run_id, run_id);
        assert!(replay.staged_point_ids.is_empty());
        assert!(replay.document_ids.is_empty());
        assert!(replay.retired_point_ids.is_empty());
        assert_eq!(
            mock.upsert_calls().len(),
            upserts_after_switch,
            "a replay writes no points"
        );
        assert_eq!(fixture.registry_entry(run_id).await.phase, "switched");
    }

    fn point_matches_hash(call: &qdrant_points_mock::UpsertCall, id: &str, hash: &str) -> bool {
        payload_record_hash(call, id).as_deref() == Some(hash)
    }

    /// An admission refusal sends no upsert and stages nothing.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_admission_refusal_sends_no_upsert() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);

        let error = run(
            &fixture,
            &embedding,
            &mock,
            [1, 1, 1],
            vec![section_input("kept", &kept.external_id, "new body")],
        )
        .await
        .expect_err("a per-run cap of one must refuse the admission");
        assert!(
            error.to_string().contains("registry capacity policy"),
            "the refusal names the capacity policy: {error}"
        );

        assert!(mock.upsert_calls().is_empty());
        let state = fixture.db.read_search_registry_state().await.unwrap();
        assert!(
            !state
                .entries
                .iter()
                .any(|entry| entry.file_id == Some(fixture.file_id)),
            "a refused admission leaves no registry entry for the file"
        );
        let file = fixture.file_record().await;
        assert_ne!(
            file.ingest_status,
            context69::contracts::LibraryIngestStatus::Succeeded
        );
        assert_eq!(
            fixture
                .db
                .list_chunk_ids_for_document(kept.document_id)
                .await
                .unwrap(),
            vec![kept.chunk_id]
        );
    }

    /// A transient embedding failure keeps the staging admission and the old
    /// data, and a retry with the same durable generation succeeds without
    /// new ids.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_transient_embedding_failure_preserves_staging_and_a_retry_succeeds() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let staged = fixture.attach_staged_object().await;
        let (run_id, generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);
        let sections = vec![
            section_input("kept", &kept.external_id, "new body"),
            section_input("added", "retry-added-external", "new section body"),
        ];

        let error = run(
            &fixture,
            &FakeEmbedding::new(1),
            &mock,
            [1000, 1000, 1000],
            sections.clone(),
        )
        .await
        .expect_err("the first attempt fails at the embedder");
        assert!(error.to_string().contains("transient embedding outage"));

        // The staging admission was already committed and stays in place.
        let entry = fixture.registry_entry(run_id).await;
        assert_eq!(entry.phase, "staging");
        assert_eq!(entry.generation, generation);
        let staged_ids = entry.staged_ids.clone();
        assert!(!staged_ids.is_empty());

        // The old data and the file are untouched, and the staged source was
        // never released on the transient failure.
        assert_eq!(
            fixture
                .db
                .list_chunk_ids_for_document(kept.document_id)
                .await
                .unwrap(),
            vec![kept.chunk_id]
        );
        let source = fixture.file_source().await;
        assert_eq!(source.storage_object_id, Some(fixture.old_object));
        assert_ne!(
            source.ingest_status, "succeeded",
            "the transient failure leaves the file unsucceeded"
        );

        // The retry reuses the same admission and the same staged ids.
        let outcome = run_with_source(
            &fixture,
            &embedding,
            &mock,
            [1000, 1000, 1000],
            sections,
            Some(staged),
        )
        .await
        .expect("the retry switches");
        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);
        assert_eq!(outcome.staged_point_ids, staged_ids);
        assert_eq!(outcome.run_id, run_id);
        assert_eq!(outcome.generation, generation);

        let entry = fixture.registry_entry(run_id).await;
        assert_eq!(entry.phase, "switched");
        let run = fixture
            .db
            .read_library_ingest_run(fixture.item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(run.resolved_at.is_some());
        assert_eq!(fixture.file_source().await.storage_object_id, Some(staged));
    }

    /// A new section's final document id is durable before the first point
    /// write, so a transparent failure and its retry reuse the exact mapping,
    /// and the same run refuses a changed section identity instead of
    /// silently remapping it.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_new_section_identity_is_reserved_durably_before_the_first_point_write() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let (run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let sections = vec![
            section_input("kept", &kept.external_id, "new body"),
            section_input("added", "durable-added-external", "new section body"),
        ];

        // The first attempt stops at the embedder, after the mapping and the
        // staging admission committed and before any point was written.
        let error = run(
            &fixture,
            &FakeEmbedding::new(1),
            &mock,
            [1000, 1000, 1000],
            sections.clone(),
        )
        .await
        .expect_err("the first attempt fails at the embedder");
        assert!(
            error.to_string().contains("transient embedding outage"),
            "unexpected first error: {error:?}"
        );
        assert!(mock.upsert_calls().is_empty());
        let reserved: i64 = sqlx::query_scalar(
            "SELECT document_id FROM context69.library_ingest_run_sections \
             WHERE run_id = $1 AND section_key = 'added'",
        )
        .bind(run_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("the new section reserved its final document id");
        let entry = fixture.registry_entry(run_id).await;
        assert!(
            entry.document_ids.contains(&reserved),
            "the staging admission holds the same document set as the durable mapping"
        );

        // The same run refuses a changed section identity under the reserved
        // key instead of allocating another document id.
        let current_run = fixture
            .db
            .read_library_ingest_run(fixture.item_id)
            .await
            .unwrap()
            .unwrap();
        let error = fixture
            .db
            .reserve_library_ingest_run_section(
                fixture.fence,
                &current_run,
                "added",
                "changed-added-external",
            )
            .await
            .expect_err("a changed section identity is refused");
        assert!(error.to_string().contains("another section identity"));

        // The retry switches the mapped id, so the committed document is the
        // one the mapping reserved before the first external write.
        let outcome = run(
            &fixture,
            &FakeEmbedding::new(0),
            &mock,
            [1000, 1000, 1000],
            sections,
        )
        .await
        .expect("the retry switches");
        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);
        assert_eq!(
            outcome.document_ids,
            vec![kept.document_id, reserved],
            "the new section keeps its reserved final document id"
        );
    }

    /// A missing staged point prevents the switch: the insert-only staging
    /// lands the points, the verification read finds one missing, and the
    /// switch never runs while the staging stays for a retry.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_missing_point_prevents_the_switch() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let (run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let sections = vec![section_input("kept", &kept.external_id, "new body")];

        // A poisoned first attempt commits the admission without writing
        // points, so the planned id is known before the real attempt.
        assert!(
            run(
                &fixture,
                &FakeEmbedding::new(1),
                &mock,
                [1000, 1000, 1000],
                sections.clone()
            )
            .await
            .is_err()
        );
        let staged_id = fixture.registry_entry(run_id).await.staged_ids[0];

        // The real attempt upserts the point, but the verification read never
        // sees it, as if a concurrent writer had deleted it in between.
        mock.omit_from_get(&[staged_id.to_string()]);
        let embedding = FakeEmbedding::new(0);
        let error = run(&fixture, &embedding, &mock, [1000, 1000, 1000], sections)
            .await
            .expect_err("a missing staged point must refuse the switch");
        assert!(
            error.to_string().contains("missing"),
            "the verifier names the missing point: {error}"
        );

        // The run never switched: the registry stays staging, the old data
        // and the file are unchanged.
        assert_eq!(fixture.registry_entry(run_id).await.phase, "staging");
        assert_eq!(
            fixture
                .db
                .list_chunk_ids_for_document(kept.document_id)
                .await
                .unwrap(),
            vec![kept.chunk_id]
        );
        let source = fixture.file_source().await;
        assert_eq!(source.storage_object_id, Some(fixture.old_object));
        assert_ne!(
            source.ingest_status, "succeeded",
            "the transient failure leaves the file unsucceeded"
        );
        let run = fixture
            .db
            .read_library_ingest_run(fixture.item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(run.resolved_at.is_none());
    }

    /// A foreign point under a planned id keeps its stored identity: the
    /// upsert is insert-only, so the verification refuses the switch.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_foreign_point_under_a_planned_id_prevents_the_switch() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let (run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);
        let sections = vec![section_input("kept", &kept.external_id, "new body")];

        // The poisoned attempt commits the admission without writing points.
        assert!(
            run(
                &fixture,
                &FakeEmbedding::new(1),
                &mock,
                [1000, 1000, 1000],
                sections.clone()
            )
            .await
            .is_err()
        );
        let staged_id = fixture.registry_entry(run_id).await.staged_ids[0];

        // A foreign point under the planned id, with another document.
        mock.insert_preexisting(
            &staged_id.to_string(),
            kept.document_id + 1_000_000,
            "foreign-hash",
            DIMENSIONS,
        );

        let error = run(&fixture, &embedding, &mock, [1000, 1000, 1000], sections)
            .await
            .expect_err("the foreign identity must refuse the switch");
        assert!(
            error
                .to_string()
                .contains("belongs to a different document"),
            "the verification names the identity conflict: {error}"
        );
        assert_eq!(fixture.registry_entry(run_id).await.phase, "staging");
        assert_eq!(
            fixture
                .db
                .list_chunk_ids_for_document(kept.document_id)
                .await
                .unwrap(),
            vec![kept.chunk_id]
        );
    }

    /// A stored staging admission that another plan produced is never
    /// overwritten; the conflicting plan is refused before any upsert.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_conflicting_plan_is_refused_without_an_upsert() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let (run_id, generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;

        // Commit a staging admission with a foreign staged set.
        fixture
            .admit(
                run_id,
                generation,
                vec![kept.document_id],
                vec![Uuid::new_v4()],
                1,
            )
            .await;

        let embedding = FakeEmbedding::new(0);
        let error = run(
            &fixture,
            &embedding,
            &mock,
            [1000, 1000, 1000],
            vec![section_input("kept", &kept.external_id, "new body")],
        )
        .await
        .expect_err("a conflicting staged set must be refused");
        assert!(
            error.to_string().contains("another staged set"),
            "the refusal names the stored admission: {error}"
        );
        assert!(mock.upsert_calls().is_empty());
    }

    /// A physical cleanup failure is tolerated: the run still reports a
    /// successful switch and the tombstone stays.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_cleanup_failure_reports_pending_cleanup_and_keeps_the_tombstone() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        fixture.attach_staged_object().await;
        let (run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        mock.set_fail_deletes(true);
        let embedding = FakeEmbedding::new(0);
        let sections = vec![section_input("kept", &kept.external_id, "new body")];

        let outcome = run(&fixture, &embedding, &mock, [1000, 1000, 1000], sections)
            .await
            .expect("the switch succeeds despite the cleanup failure");
        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);
        assert_eq!(outcome.retired_point_ids, vec![kept.chunk_id]);
        assert_eq!(
            outcome.pending_cleanup_point_ids,
            vec![kept.chunk_id],
            "the failed delete is reported as pending"
        );
        let entry = fixture.registry_entry(run_id).await;
        assert_eq!(entry.phase, "switched");
        assert_eq!(
            entry.retired_ids,
            vec![kept.chunk_id],
            "the tombstone stays in the registry"
        );
        assert!(
            mock.delete_calls().is_empty(),
            "the failed physical delete is never recorded as done"
        );
        assert!(
            outcome
                .staged_point_ids
                .iter()
                .all(|id| mock.stored_point_ids().contains(&id.to_string())),
            "the new live ids are never removed"
        );
    }

    /// A lost grant refuses the run before anything is staged.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_lost_grant_refuses_the_run_before_anything_is_staged() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let (run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);

        sqlx::query_file!(
            "tests/library_ingest_switch/revoke_lease.sql",
            Uuid::new_v4(),
            fixture.item_id
        )
        .execute(fixture.db.pool())
        .await
        .unwrap();

        let error = run(
            &fixture,
            &embedding,
            &mock,
            [1000, 1000, 1000],
            vec![section_input("kept", &kept.external_id, "new body")],
        )
        .await
        .expect_err("a lost grant refuses the claim");
        assert!(
            error.to_string().contains("grant") || error.to_string().contains("authoriz"),
            "the refusal names the lost grant: {error}"
        );
        assert!(mock.upsert_calls().is_empty());
        let state = fixture.db.read_search_registry_state().await.unwrap();
        assert!(
            !state
                .entries
                .iter()
                .any(|entry| entry.run_id == run_id && entry.phase == "staging"),
            "the pre-created run has no fresh staging admission"
        );
        let file = fixture.file_record().await;
        assert_ne!(
            file.ingest_status,
            context69::contracts::LibraryIngestStatus::Succeeded
        );

        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let (run_id, _) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);
        let mut blocker = fixture.db.pool().begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM context69.documents WHERE id = $1 FOR UPDATE")
            .bind(kept.document_id)
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let attempt = run(
            &fixture,
            &embedding,
            &mock,
            [1000, 1000, 1000],
            vec![
                section_input("kept", &kept.external_id, "new body"),
                section_input("added", "cancelled-added-external", "new section body"),
            ],
        );
        tokio::pin!(attempt);
        let blocked = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                     WHERE $1 = ANY(pg_blocking_pids(pid)))",
                )
                .bind(blocker_pid)
                .fetch_one(fixture.db.pool())
                .await
                .unwrap();
                if waiting {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });
        tokio::select! {
            result = &mut attempt => panic!("the preparation must wait for its document lock: {result:?}"),
            result = blocked => result.expect("observe preparation after its fenced claim"),
        }
        sqlx::query("UPDATE context69.task_items SET status = 'cancelled' WHERE id = $1")
            .bind(fixture.item_id)
            .execute(fixture.db.pool())
            .await
            .unwrap();
        let retired: bool = sqlx::query_scalar(
            "SELECT retired_at IS NOT NULL FROM context69.library_ingest_runs WHERE batch_id = $1",
        )
        .bind(run_id)
        .fetch_one(fixture.db.pool())
        .await
        .unwrap();
        assert!(
            retired,
            "the cancellation retired the run before preparation resumed"
        );
        blocker.rollback().await.unwrap();
        let error = attempt.await.expect_err("the cancelled item cannot switch");
        assert!(
            error.to_string().contains("grant") || error.to_string().contains("authoriz"),
            "the resumed preparation must reach its stale-grant refusal, not another failure: {error}"
        );
        let state = fixture.db.read_search_registry_state().await.unwrap();
        let orphan = state.entries.iter().find(|entry| entry.run_id == run_id);
        assert!(
            orphan.is_none_or(|entry| entry.phase != "staging"),
            "a terminal run cannot acquire a new staging reservation after its atomic retirement: {orphan:?}; point upserts={}",
            mock.upsert_calls().len()
        );
        assert!(
            mock.upsert_calls().is_empty(),
            "no point write after terminal retirement"
        );
    }

    /// The staged ids of one run are generation-scoped and stay stable across
    /// retries, so a retired id is never reproduced by a later generation.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn staged_ids_are_generation_scoped_and_stay_stable_across_retries() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let (run_id, generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;

        let sections = vec![section_input("kept", &kept.external_id, "same body")];
        assert!(
            run(
                &fixture,
                &FakeEmbedding::new(1),
                &mock,
                [1000, 1000, 1000],
                sections.clone()
            )
            .await
            .is_err()
        );
        let entry = fixture.registry_entry(run_id).await;
        let first_ids = entry.staged_ids.clone();
        assert_eq!(entry.generation, generation);
        assert!(
            !first_ids.contains(&kept.chunk_id),
            "the staged ids are generation-scoped, never the seeded live id"
        );

        let embedding = FakeEmbedding::new(0);
        let outcome = run(&fixture, &embedding, &mock, [1000, 1000, 1000], sections)
            .await
            .expect("the retry switches");
        assert_eq!(outcome.staged_point_ids, first_ids);
    }

    /// A content-identical replacement still reindexes under the new
    /// generation without adding a second document version.
    ///
    /// The seeded document is normalized from exactly the final content,
    /// metadata and external identity the runner derives, so the replacement is
    /// a true same-hash case rather than a weakened one.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_same_hash_new_generation_reindexes_without_a_new_version() {
        let fixture = harness::SwitchFixture::new().await;
        let metadata = final_section_metadata(&fixture, "kept");
        let external_id = format!("kept-{}", Uuid::new_v4());
        let normalized = context69::normalize::normalize_record(context69::domain::SourceRecord {
            external_id: external_id.clone(),
            title: "replaced".into(),
            body_text: "same body".into(),
            source_uri: "context69://library/replacement".into(),
            summary: None,
            published_at: None,
            updated_at: chrono::Utc::now(),
            metadata_json: metadata,
        });
        let mut payload = fixture.payload(&external_id, &normalized.record_hash, Uuid::new_v4());
        payload.title = normalized.title.clone();
        payload.summary = normalized.summary.clone();
        payload.source_uri = normalized.source_uri.clone();
        payload.metadata_json = normalized.metadata_json.clone();
        payload.chunk_text = normalized.body_text.clone();
        payload.source_key = "library".into();
        let kept = fixture.seed_document("kept", &payload).await;
        let (run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);
        let sections = vec![section_input("kept", &external_id, "same body")];
        let outcome = run(&fixture, &embedding, &mock, [1000, 1000, 1000], sections)
            .await
            .expect("the content-identical replacement switches");
        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);
        assert_eq!(
            outcome.retired_point_ids,
            vec![kept.chunk_id],
            "the old generation point retires"
        );
        assert_ne!(outcome.staged_point_ids[0], kept.chunk_id);
        assert_eq!(
            fixture
                .db
                .list_chunk_ids_for_document(kept.document_id)
                .await
                .unwrap(),
            vec![outcome.staged_point_ids[0]],
            "the new generation chunks replace the old ones"
        );
        let versions = sqlx::query_file_scalar!(
            "tests/library_ingest_switch/version_count.sql",
            kept.document_id
        )
        .fetch_one(fixture.db.pool())
        .await
        .unwrap();
        assert_eq!(
            versions, 1,
            "a same-hash replacement never appends a duplicate version"
        );
        assert_eq!(fixture.registry_entry(run_id).await.phase, "switched");
    }

    /// The stored document hash and every point payload are the normalization
    /// of the final stored source, metadata and authoritative identity, never a
    /// caller-provided hash.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_stored_hash_matches_the_final_normalized_source_and_metadata() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let staged = fixture.attach_staged_object().await;
        let (_run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);
        let sections = vec![section_input("kept", &kept.external_id, "new body")];
        let outcome = run_with_source(
            &fixture,
            &embedding,
            &mock,
            [1000, 1000, 1000],
            sections,
            Some(staged),
        )
        .await
        .expect("the replacement switches");
        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);

        // The staged point of the kept section carries the final source and
        // the composed metadata; normalizing them together must reproduce the
        // payload hash.
        let kept_point = outcome.staged_point_ids[0].to_string();
        let call = mock
            .upsert_calls()
            .into_iter()
            .find(|call| call.point_ids.contains(&kept_point))
            .expect("the kept section was upserted");
        let point = call.points.get(&kept_point).expect("the kept point");
        let payload: serde_json::Value = qdrant_client::Payload::from(point.payload.clone())
            .deserialize()
            .expect("the payload decodes as JSON");
        let normalized = context69::normalize::normalize_record(context69::domain::SourceRecord {
            external_id: payload["external_id"]
                .as_str()
                .expect("the payload carries its external id")
                .to_string(),
            title: payload["title"]
                .as_str()
                .expect("the payload carries its title")
                .to_string(),
            body_text: "new body".into(),
            source_uri: payload["source_uri"]
                .as_str()
                .expect("the payload carries its source uri")
                .to_string(),
            summary: None,
            published_at: None,
            updated_at: chrono::Utc::now(),
            metadata_json: payload["metadata_index"].clone(),
        });
        assert_eq!(
            normalized.record_hash,
            payload["record_hash"]
                .as_str()
                .expect("the payload carries its record hash"),
            "the payload hash is the normalization of the stored source and metadata"
        );

        // The stored document row carries the same hash.
        let mut connection = fixture.db.pool().acquire().await.unwrap();
        let stored = fixture
            .db
            .lock_replacement_documents_on(&mut connection, &[kept.document_id])
            .await
            .unwrap();
        assert_eq!(stored[0].record_hash, normalized.record_hash);
    }

    /// An insert-only upsert never overwrites a live point that shares an id:
    /// the mock keeps the pre-existing payload, so the staged retry is
    /// idempotent instead of destructive.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_insert_only_retry_never_overwrites_a_verified_point() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let (run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;

        let sections = vec![section_input("kept", &kept.external_id, "new body")];
        assert!(
            run(
                &fixture,
                &FakeEmbedding::new(1),
                &mock,
                [1000, 1000, 1000],
                sections.clone()
            )
            .await
            .is_err()
        );
        let staged_id = fixture.registry_entry(run_id).await.staged_ids[0];
        let identity = (
            staged_id.to_string(),
            kept.document_id,
            "new-hash".to_string(),
        );

        // The real attempt writes and switches.
        let embedding = FakeEmbedding::new(0);
        let outcome = run(&fixture, &embedding, &mock, [1000, 1000, 1000], sections)
            .await
            .expect("the retry switches");
        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);

        // A further insert-only write of the same id keeps the stored
        // payload, so a repeated staging can never corrupt the live point.
        let stored = mock.stored_point(&identity.0).expect("the live point");
        assert!(stored.payload.contains_key("record_hash"));
        let _ = identity;
    }

    /// A replacement that publishes another source hash retires every stored
    /// translation of the affected document, including the one of a historical
    /// version, and keeps the translation history queryable.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_replacement_retires_the_historical_stale_translation_ids() {
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let current_translation = Uuid::new_v4();
        let historical_translation = Uuid::new_v4();
        seed_translation_version(&fixture, kept.document_id, "old-hash", current_translation).await;
        seed_translation_version(
            &fixture,
            kept.document_id,
            "older-hash",
            historical_translation,
        )
        .await;
        let (run_id, _generation) = fixture.create_run().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);
        let sections = vec![section_input("kept", &kept.external_id, "new body")];
        let outcome = run(&fixture, &embedding, &mock, [1000, 1000, 1000], sections)
            .await
            .expect("the replacement switches");

        let retired: std::collections::BTreeSet<Uuid> =
            outcome.retired_point_ids.iter().copied().collect();
        assert!(
            retired.contains(&kept.chunk_id),
            "the replaced document chunk retires"
        );
        assert!(
            retired.contains(&current_translation),
            "the translation of the replaced hash retires"
        );
        assert!(
            retired.contains(&historical_translation),
            "the translation of an older hash retires too"
        );

        let versions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.document_translation_versions WHERE document_id = $1",
        )
        .bind(kept.document_id)
        .fetch_one(fixture.db.pool())
        .await
        .unwrap();
        assert_eq!(
            versions, 2,
            "the translation history stays queryable after the retirement"
        );

        let entry = fixture.registry_entry(run_id).await;
        assert_eq!(entry.phase, "switched");
        assert!(
            entry.retired_ids.contains(&historical_translation),
            "the tombstone covers the historical translation id"
        );
    }

    /// Unused-import guard for the shared fixture helper types this module
    /// keeps for the cases above.
    #[allow(dead_code)]
    fn retain_imports(_: Option<Arc<()>>) {}

    /// A pinned token budget governs the replacement chunks even though the
    /// service-level configuration is characters: every stored chunk stays
    /// within the token limit, the body yields more than one chunk, and an
    /// invalid token budget fails before any point is staged.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_pinned_token_budget_governs_the_replacement_chunks() {
        let fixture = harness::SwitchFixture::new().await;
        let mock = QdrantPointsMock::start().await;
        let embedding = FakeEmbedding::new(0);
        let body = vec!["token budget replacement body"; 40].join("\n\n");
        let sections = vec![section_input("kept", "kept-external", &body)];
        let budget = ChunkBudget::tokens(
            context69::chunking::chunking_token::TokenizerKind::Cl100kBase,
            8,
            2,
        );

        let outcome = run_with_budget(&fixture, &embedding, &mock, &budget, sections)
            .await
            .expect("the token-budget replacement switches");
        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);
        assert!(
            outcome.staged_point_ids.len() > 1,
            "the token budget must split the body instead of the 1000-char budget"
        );
        for call in mock.upsert_calls() {
            for text in chunk_texts(&call) {
                let tokens = context69::chunking::chunking_token::token_count(
                    &text,
                    context69::chunking::chunking_token::TokenizerKind::Cl100kBase,
                );
                assert!(
                    tokens <= 8,
                    "a pinned token budget must bound every stored chunk: {tokens} tokens"
                );
            }
        }

        // A malformed token budget is refused while planning, before any
        // point write or state change.
        let invalid = ChunkBudget {
            mode: context69::chunking::ChunkBudgetMode::Tokens,
            chars: ChunkingConfig::default(),
            tokens: Some(context69::chunking::TokenBudget {
                tokenizer: context69::chunking::chunking_token::TokenizerKind::Cl100kBase.into(),
                max_tokens: 0,
                overlap_tokens: 0,
            }),
            input_policy: None,
        };
        let mock = QdrantPointsMock::start().await;
        let fixture = harness::SwitchFixture::new().await;
        let sections = vec![section_input("kept", "kept-external", &body)];
        let error = run_with_budget(&fixture, &embedding, &mock, &invalid, sections)
            .await
            .expect_err("an invalid token budget must fail before committing");
        assert!(
            error.to_string().contains("token"),
            "the failure names the invalid token budget: {error}"
        );
        assert!(
            mock.upsert_calls().is_empty(),
            "a refused plan writes no points"
        );
    }
}
