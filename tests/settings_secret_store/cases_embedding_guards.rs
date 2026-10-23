//! The embedding identity guard as the settings family sees it: a rejected save
//! must leave every settings block, every store row and the runtime observer
//! untouched, because runtime settings are five singletons committed as one unit.

use std::sync::Arc;

use super::support::{
    assert_sealed_under, assert_stored_bytes_exclude, identity_fingerprint, keyed, reset, run,
    runtime_request, s3_request, seed_vector_identity, service, store_metadata,
    vector_identity_state,
};
use context69::{
    domain_errors::DomainError,
    services::{secret_store::key_names, settings::RuntimeEmbeddingTransition},
};

const SYNTHETIC_KEY: &str = "synthetic-embedding-key";
const PURPOSE: &str = "embedding.api_key";

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn an_identity_guard_rejects_the_save_before_anything_is_persisted() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        // Mirrors the application guard: while a fixed vector index is live, a
        // model/dimension/endpoint identity change cannot be applied live, so
        // the save is rejected rather than accepted and silently ignored.
        settings.set_runtime_embedding_guard(Some(Arc::new(|embedding| {
            if embedding.base_url != "http://127.0.0.1:1/v1"
                || embedding.model != "context69-secret-store-test"
                || embedding.dimensions != 128
            {
                return Err(DomainError::invalid_argument(
                    "runtime.embedding identity cannot change while the vector index is live",
                )
                .into());
            }
            Ok(())
        })));

        let mut changed = runtime_request(Some(SYNTHETIC_KEY), None);
        changed.embedding.dimensions = 256;
        let error = settings
            .update_runtime_settings(&changed)
            .await
            .expect_err("a dimension change must be rejected");
        assert!(error.to_string().contains("identity"), "{error}");

        // A model-only change is rejected the same way.
        let mut model_change = runtime_request(Some(SYNTHETIC_KEY), None);
        model_change.embedding.model = "other-model".to_string();
        settings
            .update_runtime_settings(&model_change)
            .await
            .expect_err("a model change must be rejected");

        // An endpoint-only change is rejected too.
        let mut endpoint_change = runtime_request(Some(SYNTHETIC_KEY), None);
        endpoint_change.embedding.base_url = "https://other.example/v1".to_string();
        settings
            .update_runtime_settings(&endpoint_change)
            .await
            .expect_err("an endpoint change must be rejected");

        // Nothing was persisted: no stored key and no reported presence.
        assert!(
            store_metadata(db.pool(), key_names::EMBEDDING_API_KEY)
                .await
                .is_none(),
            "a rejected save must not write the key"
        );
        assert!(
            !settings
                .get_runtime_settings()
                .await
                .expect("read presence")
                .embedding
                .has_api_key
        );

        // The same-identity save still succeeds and stores the key.
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("a same-identity save is accepted");
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;

        reset(db).await;
    });
}

/// The guard has to be inert across the whole settings family, not only the
/// embedding key: runtime settings are five singletons committed as one unit, so
/// a rejected save that still wrote the scheduler, qdrant, chunking or file
/// library row would leave the persisted configuration a mix of two requests.
/// It also must not notify the observer, or the live runtime would reload from
/// settings the save was supposed to have refused.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_rejected_dimension_save_leaves_every_settings_block_and_the_observer_untouched() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));

        // A committed baseline for every block the family owns.
        let mut baseline = runtime_request(
            Some(SYNTHETIC_KEY),
            Some(s3_request(Some("baseline-secret"))),
        );
        baseline.qdrant.collection_name = "baseline-collection".to_string();
        baseline.scheduler.interval_secs = 3600;
        baseline.chunking.max_chars = 1200;
        baseline.embedding.model = "baseline-model".to_string();
        settings
            .update_runtime_settings(&baseline)
            .await
            .expect("seed the baseline runtime settings");

        let notified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = notified.clone();
        settings.set_runtime_settings_observer(Some(Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })));
        settings.set_runtime_embedding_guard(Some(Arc::new(|embedding| {
            if embedding.dimensions != 128 {
                return Err(DomainError::invalid_argument(
                    "runtime.embedding.dimensions must match the configured vector index",
                )
                .into());
            }
            Ok(())
        })));

        // A rejected save that changes every other block too.
        let mut rejected = runtime_request(
            Some("replacement-key-that-must-not-be-stored"),
            Some(s3_request(Some("replacement-secret"))),
        );
        rejected.embedding.dimensions = 256;
        rejected.embedding.model = "replacement-model".to_string();
        rejected.qdrant.collection_name = "replacement-collection".to_string();
        rejected.scheduler.interval_secs = 60;
        rejected.chunking.max_chars = 4096;
        rejected.file_library.storage_root = "/tmp/context69-guard-must-not-apply".to_string();
        settings
            .update_runtime_settings(&rejected)
            .await
            .expect_err("a dimension change must be rejected");

        // Every block still reads back as the baseline, so nothing was partially
        // applied.
        let after = settings
            .get_runtime_settings()
            .await
            .expect("read the runtime settings back");
        assert_eq!(after.embedding.dimensions, 128);
        assert_eq!(after.embedding.model, "baseline-model");
        assert_eq!(after.qdrant.collection_name, "baseline-collection");
        assert_eq!(after.scheduler.interval_secs, 3600);
        assert_eq!(after.chunking.max_chars, 1200);
        assert!(
            !after.file_library.storage_root.contains("must-not-apply"),
            "storage_root must not change: {}",
            after.file_library.storage_root
        );
        // The stored keys are the baseline ones: the replacement key and secret
        // were never written.
        assert_stored_bytes_exclude(
            db.pool(),
            key_names::EMBEDDING_API_KEY,
            "replacement-key-that-must-not-be-stored",
        )
        .await;
        assert_stored_bytes_exclude(
            db.pool(),
            key_names::RUNTIME_S3_SECRET_KEY,
            "replacement-secret",
        )
        .await;
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;
        assert_eq!(
            notified.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a rejected save must not notify the runtime observer"
        );

        reset(db).await;
    });
}

fn allow_all_guard() -> Arc<
    dyn Fn(&context69::contracts::UpdateRuntimeEmbeddingSettings) -> anyhow::Result<()>
        + Send
        + Sync,
> {
    Arc::new(|_| Ok(()))
}

fn open_transition() -> RuntimeEmbeddingTransition {
    RuntimeEmbeddingTransition::new(|| true, || {})
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_confirmed_same_space_transition_acknowledges_identity_in_the_settings_transaction() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        let baseline = runtime_request(Some(SYNTHETIC_KEY), None);
        settings
            .update_runtime_settings(&baseline)
            .await
            .expect("seed the baseline runtime settings");
        let baseline_fingerprint =
            identity_fingerprint("http://127.0.0.1:1/v1", "context69-secret-store-test", 128);
        let rebuilt_at = seed_vector_identity(db, &baseline_fingerprint, 128).await;

        let applied = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let applied_count = applied.clone();
        settings.set_runtime_embedding_guard(Some(allow_all_guard()));
        settings.set_runtime_embedding_transition(Some(open_transition()));
        settings.set_runtime_embedding_applier(Some(Arc::new(
            move |_provider, _identity, _stored, _key| {
                applied_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async { Ok::<(), anyhow::Error>(()) })
            },
        )));

        let mut transition = runtime_request(Some(SYNTHETIC_KEY), None);
        transition.embedding.base_url = "http://127.0.0.1:1/gateway".to_string();
        transition.embedding.model = "alias-model".to_string();
        transition.embedding.confirm_vector_space_unchanged = true;

        let response = settings
            .update_runtime_settings(&transition)
            .await
            .expect("a confirmed same-dimensional transition is accepted");
        assert_eq!(response.embedding.model, "alias-model");
        assert!(
            !response.requires_rebuild,
            "the acknowledged identity matches the stored configuration"
        );
        assert_eq!(
            applied.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "success is reported only after the runtime apply ran"
        );

        let (fingerprint, chunks, after) = vector_identity_state(db)
            .await
            .expect("the acknowledged identity row exists");
        assert_eq!(
            fingerprint,
            identity_fingerprint("http://127.0.0.1:1/gateway", "alias-model", 128)
        );
        assert_eq!(chunks, 7, "the rebuilt chunk count is preserved");
        assert_eq!(after, rebuilt_at, "the rebuild timestamp is preserved");

        reset(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn an_unconfirmed_identity_change_is_rejected_before_any_write() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the baseline runtime settings");
        let baseline_fingerprint =
            identity_fingerprint("http://127.0.0.1:1/v1", "context69-secret-store-test", 128);
        seed_vector_identity(db, &baseline_fingerprint, 128).await;

        settings.set_runtime_embedding_guard(Some(allow_all_guard()));
        settings.set_runtime_embedding_transition(Some(open_transition()));

        let mut changed = runtime_request(Some("replacement-key-that-must-not-be-stored"), None);
        changed.embedding.model = "alias-model".to_string();
        let error = settings
            .update_runtime_settings(&changed)
            .await
            .expect_err("an unconfirmed identity change must be rejected");
        assert!(error.to_string().contains("confirmation"), "{error}");

        assert_stored_bytes_exclude(
            db.pool(),
            key_names::EMBEDDING_API_KEY,
            "replacement-key-that-must-not-be-stored",
        )
        .await;
        assert_eq!(
            settings
                .get_runtime_settings()
                .await
                .expect("read back the settings")
                .embedding
                .model,
            "context69-secret-store-test"
        );
        let (fingerprint, _, _) = vector_identity_state(db).await.expect("baseline identity");
        assert_eq!(fingerprint, baseline_fingerprint);

        reset(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_confirmed_transition_without_a_matching_persisted_identity_fails_the_whole_save() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the baseline runtime settings");
        // A row for the right collection and dimensions, but a different
        // identity: the acknowledgment must not overwrite it.
        seed_vector_identity(db, "not-the-stored-identity", 128).await;

        settings.set_runtime_embedding_guard(Some(allow_all_guard()));
        settings.set_runtime_embedding_transition(Some(open_transition()));

        let mut transition = runtime_request(Some(SYNTHETIC_KEY), None);
        transition.embedding.model = "alias-model".to_string();
        transition.embedding.confirm_vector_space_unchanged = true;
        let error = settings
            .update_runtime_settings(&transition)
            .await
            .expect_err("a mismatched persisted identity must fail");
        assert!(
            error.to_string().contains("persisted vector identity"),
            "{error}"
        );

        assert_eq!(
            settings
                .get_runtime_settings()
                .await
                .expect("read back the settings")
                .embedding
                .model,
            "context69-secret-store-test",
            "the settings transaction rolled back"
        );
        let (fingerprint, _, _) = vector_identity_state(db).await.expect("owned identity row");
        assert_eq!(
            fingerprint, "not-the-stored-identity",
            "no fake acknowledgment row was written"
        );

        reset(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_dimension_change_is_rejected_even_when_confirmed() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the baseline runtime settings");
        let baseline_fingerprint =
            identity_fingerprint("http://127.0.0.1:1/v1", "context69-secret-store-test", 128);
        seed_vector_identity(db, &baseline_fingerprint, 128).await;

        settings.set_runtime_embedding_guard(Some(allow_all_guard()));
        settings.set_runtime_embedding_transition(Some(open_transition()));

        let mut changed = runtime_request(Some(SYNTHETIC_KEY), None);
        changed.embedding.dimensions = 256;
        changed.embedding.confirm_vector_space_unchanged = true;
        let error = settings
            .update_runtime_settings(&changed)
            .await
            .expect_err("a dimension change must never be confirmable");
        assert!(error.to_string().contains("dimensions"), "{error}");

        let (fingerprint, _, _) = vector_identity_state(db).await.expect("baseline identity");
        assert_eq!(fingerprint, baseline_fingerprint);

        reset(db).await;
    });
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_confirmed_transition_is_refused_while_another_transition_owns_the_exclusion() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the baseline runtime settings");
        let baseline_fingerprint =
            identity_fingerprint("http://127.0.0.1:1/v1", "context69-secret-store-test", 128);
        seed_vector_identity(db, &baseline_fingerprint, 128).await;

        settings.set_runtime_embedding_guard(Some(allow_all_guard()));
        settings.set_runtime_embedding_applier(Some(Arc::new(
            |_provider, _identity, _stored, _key| Box::pin(async { Ok::<(), anyhow::Error>(()) }),
        )));
        settings.set_runtime_embedding_transition(Some(RuntimeEmbeddingTransition::new(
            || false,
            || {},
        )));

        let mut transition = runtime_request(Some(SYNTHETIC_KEY), None);
        transition.embedding.model = "alias-model".to_string();
        transition.embedding.confirm_vector_space_unchanged = true;
        let error = settings
            .update_runtime_settings(&transition)
            .await
            .expect_err("a busy exclusion must reject the save");
        assert!(error.to_string().contains("in progress"), "{error}");

        let (fingerprint, _, _) = vector_identity_state(db).await.expect("baseline identity");
        assert_eq!(fingerprint, baseline_fingerprint);

        reset(db).await;
    });
}

/// A confirmed embedding alias must never also move the vectors to another
/// Qdrant destination. Both destination fields are rejected before any secret
/// or settings write, and the acknowledged identity stays truthful.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_confirmed_alias_cannot_acknowledge_a_changed_qdrant_destination() {
    run(async |db| {
        for (case, mutate) in [
            (
                "url",
                Box::new(
                    |request: &mut context69::contracts::UpdateRuntimeSettingsRequest| {
                        request.qdrant.url = "http://127.0.0.1:2".to_string();
                    },
                )
                    as Box<dyn Fn(&mut context69::contracts::UpdateRuntimeSettingsRequest)>,
            ),
            (
                "collection",
                Box::new(
                    |request: &mut context69::contracts::UpdateRuntimeSettingsRequest| {
                        request.qdrant.collection_name = "other-collection".to_string();
                    },
                ),
            ),
        ] {
            reset(db).await;
            let mut settings = service(db, keyed(db));
            settings
                .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
                .await
                .expect("seed the baseline runtime settings");
            let baseline_fingerprint =
                identity_fingerprint("http://127.0.0.1:1/v1", "context69-secret-store-test", 128);
            seed_vector_identity(db, &baseline_fingerprint, 128).await;

            let applied = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let applied_count = applied.clone();
            settings.set_runtime_embedding_guard(Some(allow_all_guard()));
            settings.set_runtime_embedding_applier(Some(Arc::new(
                move |_provider, _identity, _stored, _key| {
                    applied_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Box::pin(async { Ok::<(), anyhow::Error>(()) })
                },
            )));
            settings.set_runtime_embedding_transition(Some(open_transition()));

            let mut transition =
                runtime_request(Some("replacement-key-that-must-not-be-stored"), None);
            transition.embedding.model = "alias-model".to_string();
            transition.embedding.confirm_vector_space_unchanged = true;
            mutate(&mut transition);

            let error = settings
                .update_runtime_settings(&transition)
                .await
                .expect_err("a destination change must not be acknowledged");
            assert!(
                error.to_string().contains("cannot change"),
                "{case}: {error}"
            );
            assert_eq!(
                applied.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{case}: the runtime must not adopt a destination the save rejected"
            );

            // No secret, no settings block, and no identity row changed.
            assert_stored_bytes_exclude(
                db.pool(),
                key_names::EMBEDDING_API_KEY,
                "replacement-key-that-must-not-be-stored",
            )
            .await;
            let after = settings
                .get_runtime_settings()
                .await
                .expect("read the runtime settings back");
            assert_eq!(after.embedding.model, "context69-secret-store-test");
            assert_eq!(after.qdrant.url, "http://127.0.0.1:1");
            assert_eq!(after.qdrant.collection_name, "context69-secret-store-test");
            let (fingerprint, _, _) = vector_identity_state(db).await.expect("baseline identity");
            assert_eq!(fingerprint, baseline_fingerprint, "{case}");
        }

        reset(db).await;
    });
}

/// While an explicit rebuild owns a collection, a Qdrant destination change is
/// refused before any secret or settings write even though the embedding
/// identity is unchanged; a same-destination credential change still saves.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_destination_change_is_refused_while_a_rebuild_is_running() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        settings
            .update_runtime_settings(&runtime_request(Some(SYNTHETIC_KEY), None))
            .await
            .expect("seed the baseline runtime settings");

        settings.set_runtime_embedding_guard(Some(allow_all_guard()));
        settings.set_runtime_rebuild_in_progress(Some(std::sync::Arc::new(|| true)));

        for (case, mutate) in [
            (
                "url",
                Box::new(
                    |request: &mut context69::contracts::UpdateRuntimeSettingsRequest| {
                        request.qdrant.url = "http://127.0.0.1:2".to_string();
                    },
                )
                    as Box<dyn Fn(&mut context69::contracts::UpdateRuntimeSettingsRequest)>,
            ),
            (
                "collection",
                Box::new(
                    |request: &mut context69::contracts::UpdateRuntimeSettingsRequest| {
                        request.qdrant.collection_name = "other-collection".to_string();
                    },
                ),
            ),
        ] {
            let mut changed = runtime_request(Some("destination-change-key"), None);
            mutate(&mut changed);
            let error = settings
                .update_runtime_settings(&changed)
                .await
                .expect_err("a destination change during a rebuild must be refused");
            assert!(error.to_string().contains("in progress"), "{case}: {error}");

            assert_stored_bytes_exclude(
                db.pool(),
                key_names::EMBEDDING_API_KEY,
                "destination-change-key",
            )
            .await;
            let after = settings
                .get_runtime_settings()
                .await
                .expect("read the runtime settings back");
            assert_eq!(after.qdrant.url, "http://127.0.0.1:1", "{case}");
            assert_eq!(
                after.qdrant.collection_name, "context69-secret-store-test",
                "{case}"
            );
        }

        // The same destination with a rotated credential is still allowed, so a
        // broken key can be repaired during a rebuild.
        settings
            .update_runtime_settings(&runtime_request(Some("rotated-rebuild-key"), None))
            .await
            .expect("a same-destination credential change is accepted");
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;

        reset(db).await;
    });
}
