//! The embedding identity guard as the settings family sees it: a rejected save
//! must leave every settings block, every store row and the runtime observer
//! untouched, because runtime settings are five singletons committed as one unit.

use std::sync::Arc;

use super::support::{
    assert_sealed_under, assert_stored_bytes_exclude, keyed, reset, run, runtime_request,
    s3_request, service, store_metadata,
};
use context69::{domain_errors::DomainError, services::secret_store::key_names};

const SYNTHETIC_KEY: &str = "synthetic-embedding-key";
const PURPOSE: &str = "embedding.api_key";

#[test]
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
