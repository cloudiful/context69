//! The runtime scheduler concurrency as the settings family sees it: committed
//! global and per-resource ceilings are applied atomically to the durable task
//! queue policy, and a rejected save leaves the persisted settings and the live
//! policies unchanged.

use std::sync::{Arc, Mutex};

use context69::contracts::settings::SchedulerResourceLimits;
use context69::db::Database;

use super::support::{keyed, reset, run, runtime_request, service};

async fn queue_policies(db: &Database) -> Vec<(String, u32)> {
    db.execution_queue()
        .await
        .expect("load the execution queue")
        .resource_policy()
        .await
        .expect("read the durable resource policies")
}

async fn queue_global(db: &Database) -> Option<u32> {
    db.execution_queue()
        .await
        .expect("load the execution queue")
        .policy()
        .await
        .expect("read the durable queue policy")
        .max_concurrent_attempts
}

/// A save reports success only once the applier has installed its global and
/// per-resource limits, and the persisted row, the response, and the durable
/// queue policy agree with it.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_runtime_save_applies_its_scheduler_concurrency_to_the_queue_policy() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        let applied = Arc::new(Mutex::new(Vec::<(usize, SchedulerResourceLimits)>::new()));
        settings.set_runtime_scheduler_applier(Some(Arc::new({
            let applied = applied.clone();
            let db = db.clone();
            move |global, limits| {
                applied
                    .lock()
                    .expect("applied")
                    .push((global, limits.clone()));
                let db = db.clone();
                Box::pin(
                    async move { db.apply_execution_admission_policies(global, &limits).await },
                )
            }
        })));

        let mut request = runtime_request(Some("synthetic-runtime-key"), None);
        request.scheduler.max_concurrency = 4;
        request.scheduler.resource_limits = SchedulerResourceLimits {
            interactive: Some(2),
            bulk: Some(3),
            maintenance: None,
            ingest: Some(1),
        };
        let response = settings
            .update_runtime_settings(&request)
            .await
            .expect("a save with a working applier succeeds");
        assert_eq!(
            *applied.lock().expect("applied"),
            vec![(
                4,
                SchedulerResourceLimits {
                    interactive: Some(2),
                    bulk: Some(3),
                    maintenance: None,
                    ingest: Some(1),
                }
            )]
        );
        assert_eq!(
            response.scheduler.resource_limits,
            request.scheduler.resource_limits
        );

        let persisted: (i64, Option<i32>, Option<i32>, Option<i32>, Option<i32>) = sqlx::query_as(
            "SELECT max_concurrency, interactive_max_concurrency, bulk_max_concurrency, \
                    maintenance_max_concurrency, ingest_max_concurrency \
             FROM context69.runtime_scheduler_settings WHERE singleton",
        )
        .fetch_one(db.pool())
        .await
        .expect("read the persisted scheduler row");
        assert_eq!(persisted, (4, Some(2), Some(3), None, Some(1)));

        assert_eq!(queue_global(db).await, Some(4));
        assert_eq!(
            queue_policies(db).await,
            vec![
                ("bulk".to_string(), 3),
                ("ingest".to_string(), 1),
                ("interactive".to_string(), 2),
            ]
        );

        reset(db).await;
    });
}

/// Clearing one resource removes its exact policy key so the resource follows the
/// global cap while every other committed limit stays installed.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn clearing_a_resource_limit_restores_global_follow_without_touching_the_others() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        let mut request = runtime_request(Some("synthetic-runtime-key"), None);
        request.scheduler.max_concurrency = 4;
        request.scheduler.resource_limits = SchedulerResourceLimits {
            interactive: Some(2),
            bulk: Some(3),
            maintenance: Some(1),
            ingest: Some(1),
        };
        settings
            .update_runtime_settings(&request)
            .await
            .expect("the first save succeeds");

        request.scheduler.resource_limits.bulk = None;
        settings
            .update_runtime_settings(&request)
            .await
            .expect("the cleared save succeeds");

        let persisted: Option<i32> = sqlx::query_scalar(
            "SELECT bulk_max_concurrency FROM context69.runtime_scheduler_settings WHERE singleton",
        )
        .fetch_one(db.pool())
        .await
        .expect("read the persisted bulk limit");
        assert_eq!(persisted, None);
        assert_eq!(
            queue_policies(db).await,
            vec![
                ("ingest".to_string(), 1),
                ("interactive".to_string(), 2),
                ("maintenance".to_string(), 1),
            ],
            "the cleared key is removed and the other owned keys stay"
        );

        reset(db).await;
    });
}

/// A cap above the global concurrency is refused before any write, and lowering
/// the global concurrency below an existing explicit cap is refused rather than
/// clamping or dropping the stored value.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn invalid_resource_bounds_refuse_the_save_without_rewriting_settings_or_policies() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        let mut valid = runtime_request(Some("synthetic-runtime-key"), None);
        valid.scheduler.max_concurrency = 4;
        valid.scheduler.resource_limits = SchedulerResourceLimits {
            interactive: Some(2),
            bulk: None,
            maintenance: None,
            ingest: None,
        };
        settings
            .update_runtime_settings(&valid)
            .await
            .expect("the baseline save succeeds");

        let mut invalid = valid.clone();
        invalid.scheduler.resource_limits.interactive = Some(5);
        let error = settings
            .update_runtime_settings(&invalid)
            .await
            .expect_err("a cap above the global concurrency must be refused");
        assert!(
            error
                .to_string()
                .contains("resource_limits.interactive must not exceed"),
            "{error}"
        );

        let mut lowered = valid.clone();
        lowered.scheduler.max_concurrency = 1;
        let error = settings
            .update_runtime_settings(&lowered)
            .await
            .expect_err("lowering the global below an explicit cap must be refused");
        assert!(
            error
                .to_string()
                .contains("resource_limits.interactive must not exceed"),
            "{error}"
        );

        let persisted: (i64, Option<i32>) = sqlx::query_as(
            "SELECT max_concurrency, interactive_max_concurrency \
             FROM context69.runtime_scheduler_settings WHERE singleton",
        )
        .fetch_one(db.pool())
        .await
        .expect("read the persisted scheduler row");
        assert_eq!(persisted, (4, Some(2)));
        assert_eq!(queue_global(db).await, Some(4));
        assert_eq!(
            queue_policies(db).await,
            vec![("interactive".to_string(), 2)]
        );

        reset(db).await;
    });
}

/// The database save boundary validates the policy too, so a direct caller cannot
/// persist a cap above the global value.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn the_database_save_rejects_an_invalid_cap_before_writing_anything() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        let mut request = runtime_request(Some("synthetic-runtime-key"), None);
        request.scheduler.max_concurrency = 4;
        settings
            .update_runtime_settings(&request)
            .await
            .expect("the baseline save succeeds");
        let stored = db
            .get_runtime_settings()
            .await
            .expect("read the runtime settings")
            .expect("the runtime settings exist");

        let mut invalid = stored.clone();
        invalid.scheduler.resource_limits.ingest = Some(5);
        let error = db
            .save_runtime_settings(&invalid)
            .await
            .expect_err("the database boundary must reject the cap");
        assert!(
            error
                .to_string()
                .contains("resource_limits.ingest must not exceed"),
            "{error}"
        );

        let persisted: (i64, Option<i32>) = sqlx::query_as(
            "SELECT max_concurrency, ingest_max_concurrency \
             FROM context69.runtime_scheduler_settings WHERE singleton",
        )
        .fetch_one(db.pool())
        .await
        .expect("read the persisted scheduler row");
        assert_eq!(persisted, (4, None));
        assert_eq!(queue_global(db).await, Some(4));
        assert!(queue_policies(db).await.is_empty());

        reset(db).await;
    });
}

/// A later failure in the same transaction rolls the settings columns and every
/// policy back together, so the stored configuration and the live policy never
/// diverge.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_transaction_failure_rolls_back_settings_and_policies_together() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        let mut request = runtime_request(Some("synthetic-runtime-key"), None);
        request.scheduler.max_concurrency = 4;
        request.scheduler.resource_limits = SchedulerResourceLimits {
            interactive: Some(2),
            bulk: None,
            maintenance: None,
            ingest: None,
        };
        settings
            .update_runtime_settings(&request)
            .await
            .expect("the baseline save succeeds");

        let stored = db
            .get_runtime_settings()
            .await
            .expect("read the runtime settings")
            .expect("the runtime settings exist");
        let mut candidate = stored.clone();
        candidate.scheduler.max_concurrency = 3;
        candidate.scheduler.resource_limits.interactive = Some(3);
        let acknowledgment = context69::vector_runtime::VectorIdentityAck {
            collection_name: "missing-rollback-collection",
            expected_fingerprint: "missing-fingerprint",
            expected_dimensions: 768,
            candidate_fingerprint: "candidate-fingerprint",
            embedding_base_url: "http://embedding.invalid/v1",
            embedding_model: "missing-model",
        };
        db.save_runtime_settings_with_identity_ack(&candidate, Some(acknowledgment))
            .await
            .expect_err("the missing identity row must fail the save");

        let persisted: (i64, Option<i32>) = sqlx::query_as(
            "SELECT max_concurrency, interactive_max_concurrency \
             FROM context69.runtime_scheduler_settings WHERE singleton",
        )
        .fetch_one(db.pool())
        .await
        .expect("read the persisted scheduler row");
        assert_eq!(
            persisted,
            (4, Some(2)),
            "the settings columns roll back with the failed transaction"
        );
        assert_eq!(queue_global(db).await, Some(4));
        assert_eq!(
            queue_policies(db).await,
            vec![("interactive".to_string(), 2)],
            "the policies of the failed transaction are rolled back"
        );

        reset(db).await;
    });
}

/// A second queue handle observes the committed global and resource limits, so a
/// hot apply is durable rather than local to one in-memory owner.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_second_queue_handle_observes_the_committed_limits() {
    run(async |db| {
        reset(db).await;
        let settings = service(db, keyed(db));

        let mut request = runtime_request(Some("synthetic-runtime-key"), None);
        request.scheduler.max_concurrency = 4;
        request.scheduler.resource_limits = SchedulerResourceLimits {
            interactive: Some(2),
            bulk: Some(1),
            maintenance: None,
            ingest: None,
        };
        settings
            .update_runtime_settings(&request)
            .await
            .expect("the save succeeds");

        let second = cloudiful_tasks::Queue::open(
            db.pool().clone(),
            "context69",
            "items",
            &cloudiful_tasks::WorkerConfig::default(),
        )
        .await
        .expect("open a second queue handle");
        assert_eq!(
            second
                .policy()
                .await
                .expect("read the second queue policy")
                .max_concurrent_attempts,
            Some(4)
        );
        assert_eq!(
            second
                .resource_policy()
                .await
                .expect("read the second queue resource policies"),
            vec![("bulk".to_string(), 1), ("interactive".to_string(), 2)]
        );

        reset(db).await;
    });
}

/// A failed hot apply is reported to the saver, the committed settings value is
/// not silently rolled back, and the next working save repairs the policy.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn a_failed_hot_apply_reports_failure_and_the_next_save_repairs_it() {
    run(async |db| {
        reset(db).await;
        let mut settings = service(db, keyed(db));
        let applied = Arc::new(Mutex::new(Vec::<usize>::new()));
        settings.set_runtime_scheduler_applier(Some(Arc::new({
            let applied = applied.clone();
            move |limit, _limits| {
                applied.lock().expect("applied").push(limit);
                Box::pin(async { Ok(()) })
            }
        })));

        let mut first = runtime_request(Some("synthetic-runtime-key"), None);
        first.scheduler.max_concurrency = 2;
        settings
            .update_runtime_settings(&first)
            .await
            .expect("the first save succeeds");

        settings.set_runtime_scheduler_applier(Some(Arc::new(|_limit, _limits| {
            Box::pin(async {
                Err(anyhow::anyhow!(
                    "the durable queue policy could not be applied"
                ))
            })
        })));
        let mut failing = runtime_request(Some("synthetic-runtime-key"), None);
        failing.scheduler.max_concurrency = 3;
        let error = settings
            .update_runtime_settings(&failing)
            .await
            .expect_err("a failed hot apply must reach the caller");
        assert!(
            error.to_string().contains("could not be applied"),
            "{error}"
        );

        let persisted: i64 = sqlx::query_scalar(
            "SELECT max_concurrency FROM context69.runtime_scheduler_settings WHERE singleton",
        )
        .fetch_one(db.pool())
        .await
        .expect("read the persisted scheduler row");
        assert_eq!(
            persisted, 3,
            "the committed settings value is not silently rolled back"
        );
        assert_eq!(
            *applied.lock().expect("applied"),
            vec![2],
            "the failed apply must not report a new live value"
        );

        settings.set_runtime_scheduler_applier(Some(Arc::new({
            let applied = applied.clone();
            move |limit, _limits| {
                applied.lock().expect("applied").push(limit);
                Box::pin(async { Ok(()) })
            }
        })));
        let mut repaired = runtime_request(Some("synthetic-runtime-key"), None);
        repaired.scheduler.max_concurrency = 4;
        settings
            .update_runtime_settings(&repaired)
            .await
            .expect("the next working save succeeds");
        assert_eq!(*applied.lock().expect("applied"), vec![2, 4]);

        reset(db).await;
    });
}

/// The ingest batch quantum round-trips through the settings row, the response
/// and the focused database getter without changing the applied queue policy.
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn the_ingest_batch_quantum_roundtrips_without_changing_the_queue_policy() {
    run(async |db| {
        reset(db).await;
        assert_eq!(
            db.get_ingest_batches_per_segment()
                .await
                .expect("read the absent setting"),
            context69::contracts::settings::INGEST_BATCHES_PER_SEGMENT_DEFAULT,
            "an absent singleton row uses the application default"
        );
        let mut settings = service(db, keyed(db));
        settings.set_runtime_scheduler_applier(Some(Arc::new({
            let db = db.clone();
            move |global, limits: SchedulerResourceLimits| {
                let db = db.clone();
                Box::pin(
                    async move { db.apply_execution_admission_policies(global, &limits).await },
                )
            }
        })));

        let mut request = runtime_request(Some("synthetic-runtime-key"), None);
        request.scheduler.max_concurrency = 4;
        request.scheduler.resource_limits = SchedulerResourceLimits {
            interactive: Some(2),
            bulk: None,
            maintenance: None,
            ingest: None,
        };
        request.scheduler.ingest_batches_per_segment = 2;
        let response = settings
            .update_runtime_settings(&request)
            .await
            .expect("a save with a valid quantum succeeds");
        assert_eq!(response.scheduler.ingest_batches_per_segment, 2);
        assert_eq!(
            db.get_ingest_batches_per_segment()
                .await
                .expect("read the focused getter"),
            2
        );
        let global_after_two = queue_global(db).await;
        let policies_after_two = queue_policies(db).await;
        assert_eq!(global_after_two, Some(4));
        assert_eq!(policies_after_two, vec![("interactive".to_string(), 2)]);

        request.scheduler.ingest_batches_per_segment = 64;
        let response = settings
            .update_runtime_settings(&request)
            .await
            .expect("the maximum quantum is accepted");
        assert_eq!(response.scheduler.ingest_batches_per_segment, 64);
        assert_eq!(
            db.get_ingest_batches_per_segment()
                .await
                .expect("read the focused getter"),
            64
        );
        assert_eq!(queue_global(db).await, global_after_two);
        assert_eq!(queue_policies(db).await, policies_after_two);

        for invalid in [0usize, 65] {
            request.scheduler.ingest_batches_per_segment = invalid;
            let error = settings
                .update_runtime_settings(&request)
                .await
                .expect_err("an out-of-range quantum must be refused");
            assert!(
                error
                    .to_string()
                    .contains("ingest_batches_per_segment must be between"),
                "{error}"
            );
        }
        assert_eq!(
            db.get_ingest_batches_per_segment()
                .await
                .expect("read the unchanged getter"),
            64,
            "a refused save leaves the stored quantum unchanged"
        );
        assert_eq!(queue_global(db).await, global_after_two);
        assert_eq!(queue_policies(db).await, policies_after_two);

        reset(db).await;
    });
}
