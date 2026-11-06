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
    chunking::ChunkingRuntime,
    contracts::settings::{ChunkingTokenizer, ChunkingUnit},
    db::Database,
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

        let identity = vector_identity_state(db)
            .await
            .expect("the acknowledged identity row exists");
        assert_eq!(
            identity.fingerprint,
            identity_fingerprint("http://127.0.0.1:1/gateway", "alias-model", 128)
        );
        assert_eq!(
            identity.rebuilt_chunks, 7,
            "the rebuilt chunk count is preserved"
        );
        assert_eq!(
            identity.rebuilt_at, rebuilt_at,
            "the rebuild timestamp is preserved"
        );
        assert!(identity.active, "the acknowledged row is the active one");

        reset(db).await;
    });
}

/// An unconfirmed identity change is a desired change: it is saved as the
/// desired configuration, the active collection keeps the identity it was
/// built with, and the deployment owes an explicit rebuild for the new
/// identity.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn an_unconfirmed_identity_change_is_saved_as_the_desired_configuration() {
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

        let mut changed = runtime_request(Some("replacement-key"), None);
        changed.embedding.model = "alias-model".to_string();
        let response = settings
            .update_runtime_settings(&changed)
            .await
            .expect("an unconfirmed identity change is a desired change");
        assert_eq!(response.embedding.model, "alias-model");
        assert!(
            response.requires_rebuild,
            "the desired identity no longer matches the active collection"
        );

        // The active identity is untouched: the desired save did not rekey the
        // collection ordinary point writers use.
        let identity = vector_identity_state(db)
            .await
            .expect("the active identity row");
        assert_eq!(identity.fingerprint, baseline_fingerprint);
        assert!(identity.active);
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;

        reset(db).await;
    });
}

/// A desired dimension change is saved while the runtime is quiet: the active
/// collection keeps its identity and its dimension, and the deployment owes an
/// explicit rebuild for the new space.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_quiet_dimension_change_leaves_the_active_identity_unchanged() {
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
        changed.embedding.dimensions = 1024;
        let response = settings
            .update_runtime_settings(&changed)
            .await
            .expect("a quiet dimension change is a desired change");
        assert_eq!(response.embedding.dimensions, 1024);
        assert!(
            response.requires_rebuild,
            "the desired dimensions no longer match the active collection"
        );

        let identity = vector_identity_state(db)
            .await
            .expect("the active identity row");
        assert_eq!(
            identity.fingerprint, baseline_fingerprint,
            "the active identity is not rewritten by a desired save"
        );
        assert_eq!(identity.dimensions, 128);
        assert!(identity.active);

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
        let fingerprint = vector_identity_state(db)
            .await
            .expect("owned identity row")
            .fingerprint;
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

        let fingerprint = vector_identity_state(db)
            .await
            .expect("baseline identity")
            .fingerprint;
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

        let fingerprint = vector_identity_state(db)
            .await
            .expect("baseline identity")
            .fingerprint;
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
            let fingerprint = vector_identity_state(db)
                .await
                .expect("baseline identity")
                .fingerprint;
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

/// The chunking budget belongs to the settings family: a token budget must
/// survive a save and a read, and a rejected budget must leave the stored
/// configuration exactly as it was.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_token_chunking_budget_survives_a_save_and_a_read() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        let mut request = runtime_request(None, None);
        request.chunking.unit = ChunkingUnit::Tokens;
        request.chunking.tokenizer = ChunkingTokenizer::O200kBase;
        request.chunking.max_tokens = 512;
        request.chunking.overlap_tokens = 64;

        let saved = settings
            .update_runtime_settings(&request)
            .await
            .expect("a token budget should be saved");
        assert_eq!(saved.chunking.unit, ChunkingUnit::Tokens);
        assert_eq!(saved.chunking.tokenizer, ChunkingTokenizer::O200kBase);
        assert_eq!(saved.chunking.max_tokens, 512);
        assert_eq!(saved.chunking.overlap_tokens, 64);

        let read = settings
            .get_runtime_settings()
            .await
            .expect("read the runtime settings back");
        assert_eq!(read.chunking.unit, ChunkingUnit::Tokens);
        assert_eq!(read.chunking.tokenizer, ChunkingTokenizer::O200kBase);
        assert_eq!(read.chunking.max_tokens, 512);
        assert_eq!(read.chunking.overlap_tokens, 64);

        reset(db).await;
    });
}

/// A rejected budget is refused before anything is written, so the stored
/// configuration still describes the chunks it described before.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_rejected_chunking_budget_leaves_the_stored_configuration_unchanged() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));
        let mut request = runtime_request(None, None);
        request.chunking.unit = ChunkingUnit::Tokens;
        request.chunking.max_tokens = 256;
        request.chunking.overlap_tokens = 64;
        settings
            .update_runtime_settings(&request)
            .await
            .expect("a token budget should be saved");

        let mut invalid = runtime_request(None, None);
        invalid.chunking.unit = ChunkingUnit::Tokens;
        invalid.chunking.max_tokens = 64;
        invalid.chunking.overlap_tokens = 64;
        let error = settings
            .update_runtime_settings(&invalid)
            .await
            .expect_err("an overlap that reaches the budget must be refused");
        assert!(error.to_string().contains("overlap_tokens"), "{error}");

        let read = settings
            .get_runtime_settings()
            .await
            .expect("read the runtime settings back");
        assert_eq!(read.chunking.unit, ChunkingUnit::Tokens);
        assert_eq!(read.chunking.max_tokens, 256);
        assert_eq!(read.chunking.overlap_tokens, 64);

        reset(db).await;
    });
}

/// A character-mode save keeps an inactive token budget that token mode would
/// refuse, and the refused token-mode save leaves the settings and the stored
/// secrets alone.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn an_inactive_token_budget_does_not_block_a_character_save() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        // The inactive token budget would be refused while token mode is
        // active, but the character budget is the one in effect, so the save
        // commits the settings row and the supplied secret patch together.
        let mut character = runtime_request(Some(SYNTHETIC_KEY), None);
        character.chunking.unit = ChunkingUnit::Chars;
        character.chunking.max_chars = 900;
        character.chunking.overlap_chars = 90;
        character.chunking.max_tokens = 10;
        character.chunking.overlap_tokens = 10;
        let saved = settings
            .update_runtime_settings(&character)
            .await
            .expect("a character budget must ignore its inactive token budget");
        assert_eq!(saved.chunking.unit, ChunkingUnit::Chars);
        assert_eq!(saved.chunking.max_chars, 900);
        assert_eq!(saved.chunking.overlap_chars, 90);
        assert_eq!(
            (saved.chunking.max_tokens, saved.chunking.overlap_tokens),
            (10, 10),
            "the inactive token values are persisted unchanged"
        );
        assert_sealed_under(db.pool(), key_names::EMBEDDING_API_KEY, PURPOSE).await;
        let sealed_before = sealed_secret_bytes(db, key_names::EMBEDDING_API_KEY).await;

        // The same budget is invalid once it becomes the active one, so the
        // token-mode save is refused before any settings or secret write.
        let mut tokens = runtime_request(Some("refused-embedding-key"), None);
        tokens.chunking.unit = ChunkingUnit::Tokens;
        tokens.chunking.max_tokens = 10;
        tokens.chunking.overlap_tokens = 10;
        let error = settings
            .update_runtime_settings(&tokens)
            .await
            .expect_err("an invalid active token budget must be refused");
        assert!(error.to_string().contains("overlap_tokens"), "{error}");

        let after = settings
            .get_runtime_settings()
            .await
            .expect("read the runtime settings back");
        assert_eq!(after.chunking.unit, ChunkingUnit::Chars);
        assert_eq!(after.chunking.max_chars, 900);
        assert_eq!(
            (after.chunking.max_tokens, after.chunking.overlap_tokens),
            (10, 10)
        );
        assert_eq!(
            sealed_secret_bytes(db, key_names::EMBEDDING_API_KEY).await,
            sealed_before,
            "a refused save must not rewrite the stored secret"
        );

        // The persisted token integers are PostgreSQL BIGINT, so a value above
        // i64::MAX is refused before any settings or secret write in either
        // mode; otherwise the conversion would fail after the secret commit.
        if let Some(over_limit) = usize::try_from(i64::MAX)
            .ok()
            .and_then(|max| max.checked_add(1))
        {
            for (unit_name, unit) in [
                ("chars", ChunkingUnit::Chars),
                ("tokens", ChunkingUnit::Tokens),
            ] {
                for field in ["max_tokens", "overlap_tokens"] {
                    let mut oversized = runtime_request(Some("oversized-embedding-key"), None);
                    oversized.chunking.unit = unit;
                    oversized.chunking.max_chars = 900;
                    oversized.chunking.overlap_chars = 90;
                    oversized.chunking.max_tokens = 10;
                    oversized.chunking.overlap_tokens = 0;
                    if field == "max_tokens" {
                        oversized.chunking.max_tokens = over_limit;
                    } else {
                        oversized.chunking.overlap_tokens = over_limit;
                    }
                    let error = settings
                        .update_runtime_settings(&oversized)
                        .await
                        .expect_err("an unrepresentable token integer must be refused");
                    assert!(
                        error.to_string().contains(field),
                        "{unit_name}/{field}: {error}"
                    );
                }
            }
        }

        let after_oversized = settings
            .get_runtime_settings()
            .await
            .expect("read the runtime settings back");
        assert_eq!(after_oversized.chunking.unit, ChunkingUnit::Chars);
        assert_eq!(after_oversized.chunking.max_chars, 900);
        assert_eq!(after_oversized.chunking.overlap_chars, 90);
        assert_eq!(
            (
                after_oversized.chunking.max_tokens,
                after_oversized.chunking.overlap_tokens
            ),
            (10, 10),
            "a refused oversized save must not change the stored settings"
        );
        assert_eq!(
            sealed_secret_bytes(db, key_names::EMBEDDING_API_KEY).await,
            sealed_before,
            "a refused oversized save must not rewrite the stored secret"
        );

        reset(db).await;
    });
}

/// The sealed bytes of one stored secret.
///
/// A rewrite reseals the value with a fresh nonce, so equal bytes prove that a
/// refused save did not touch the stored secret.
async fn sealed_secret_bytes(db: &Database, key: &str) -> Vec<u8> {
    sqlx::query_scalar("SELECT value FROM context69.internal_secrets WHERE key = $1")
        .bind(key)
        .fetch_one(db.pool())
        .await
        .expect("read the sealed secret")
}

/// A committed save activates the new chunk budget through the same applier
/// the running application installs, so the next submission observes it
/// without a restart.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_committed_save_activates_the_chunk_budget() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        let runtime = ChunkingRuntime::default();
        let applied = runtime.clone();
        settings.set_runtime_chunking_applier(Some(Arc::new(move |budget| {
            applied.replace(budget);
        })));

        let mut request = runtime_request(None, None);
        request.chunking.unit = ChunkingUnit::Tokens;
        request.chunking.tokenizer = ChunkingTokenizer::O200kBase;
        request.chunking.max_tokens = 512;
        request.chunking.overlap_tokens = 64;

        settings
            .update_runtime_settings(&request)
            .await
            .expect("a token budget saves");

        assert_eq!(
            runtime.snapshot().identity(),
            "tokens:o200k_base:512:64",
            "the save applies the committed budget synchronously"
        );

        reset(db).await;
    });
}
