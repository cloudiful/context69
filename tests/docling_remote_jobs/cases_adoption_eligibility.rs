#[cfg(feature = "integration-test-helpers")]
use crate::http_mock::DoclingMock;
use crate::support::{LOCK, claim_item, cleanup, connect, crash_holder, seed_task, seed_user};
#[cfg(feature = "integration-test-helpers")]
use crate::support::{
    build_task_service, restore_docling_settings, try_claim_item, try_delete_group,
    try_delete_task, try_delete_user, try_seed_file, try_seed_file_task, try_seed_group,
    try_seed_user,
};
#[cfg(feature = "integration-test-helpers")]
use anyhow::Context;
#[cfg(feature = "integration-test-helpers")]
use chrono::{Duration, Utc};
#[cfg(feature = "integration-test-helpers")]
use context69::db::{Database, StoredDoclingSettings};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn due_remote_owners_are_claimed_once_with_identity_retained() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    for status in ["queued", "waiting", "running"] {
        let user = seed_user(&db).await;
        let (task, item) = seed_task(&db, user).await;
        let job = db
            .create_docling_remote_job(
                task,
                item,
                &Uuid::new_v4().to_string(),
                None,
                None,
                None,
                Some("http://snapshot.invalid"),
            )
            .await
            .unwrap();
        if status == "running" {
            // A running owner is a claimed item whose worker died: the
            // authoritative library attempt and its domain mirror expire
            // together, so the claim may take the item over.
            let holder = claim_item(&db, item).await;
            crash_holder(&db, &holder).await;
        } else {
            sqlx::query_file!(
                "tests/sql/task_execution/release_item_to_due.sql",
                item,
                status
            )
            .execute(db.pool())
            .await
            .unwrap();
        }
        let (first, second) = tokio::join!(db.claim_items_fast(10), db.claim_items_fast(10));
        let claims = first
            .unwrap()
            .into_iter()
            .chain(second.unwrap())
            .filter(|row| row.id == item)
            .collect::<Vec<_>>();
        assert_eq!(claims.len(), 1, "one fresh owner for {status}");
        let adopted = db
            .get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(adopted.id, job.id);
        assert_eq!(adopted.remote_task_id, job.remote_task_id);
        assert_eq!(adopted.endpoint_base_url, job.endpoint_base_url);
        cleanup(&db, task, user).await;
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn unused_detach_is_exact_job_and_live_lease_fenced() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user = seed_user(&db).await;
    let (task, item) = seed_task(&db, user).await;
    let job = db
        .create_docling_remote_job(
            task,
            item,
            &Uuid::new_v4().to_string(),
            None,
            None,
            None,
            Some("http://snapshot.invalid"),
        )
        .await
        .unwrap();
    let owner = db
        .claim_items_fast(10)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.id == item)
        .unwrap();
    let execution = owner
        .execution_context()
        .expect("the claimed item carries its captured execution");
    // The captured execution of an earlier generation belongs to a segment the
    // item does not hold any more, so the detach is refused.
    let stale = context69::db::ExecutionContext {
        attempt: execution.attempt.clone(),
        generation: execution.generation + 1,
        item_id: execution.item_id,
    };
    assert!(
        !db.detach_unused_owned_docling_job(job.id, item, &stale)
            .await
            .unwrap()
    );
    assert!(
        !db.detach_unused_owned_docling_job(Uuid::new_v4(), item, &execution)
            .await
            .unwrap()
    );
    assert!(
        db.detach_unused_owned_docling_job(job.id, item, &execution)
            .await
            .unwrap()
    );
    assert!(
        !db.detach_unused_owned_docling_job(job.id, item, &execution)
            .await
            .unwrap()
    );
    assert!(
        db.get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .is_none()
    );
    let captured = sqlx::query_file!(
        "tests/sql/task_execution/read_docling_cleanup_row.sql",
        job.id
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(
        (
            captured.remote_task_id,
            captured.endpoint_base_url,
            captured.reason
        ),
        (
            job.remote_task_id,
            job.endpoint_base_url,
            "orphan".to_string()
        )
    );
    let replacement = db
        .create_docling_remote_job(
            task,
            item,
            &Uuid::new_v4().to_string(),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        !db.detach_unused_owned_docling_job(job.id, item, &execution)
            .await
            .unwrap()
    );
    assert_eq!(
        db.get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .unwrap()
            .id,
        replacement.id
    );
    sqlx::query_file!(
        "tests/sql/task_execution/delete_docling_cleanup_row.sql",
        job.id
    )
    .execute(db.pool())
    .await
    .unwrap();
    cleanup(&db, task, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_paused_owner_keeps_its_active_remote_for_adoption() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user = seed_user(&db).await;
    let (task, item) = seed_task(&db, user).await;
    let job = db
        .create_docling_remote_job(
            task,
            item,
            &Uuid::new_v4().to_string(),
            None,
            None,
            None,
            Some("http://snapshot.invalid"),
        )
        .await
        .unwrap();
    let owner = claim_item(&db, item).await;
    let execution = owner
        .execution_context()
        .expect("the claimed item carries its captured execution");

    assert!(
        db.request_task_pause(task, user, false)
            .await
            .expect("pause the Docling owner"),
        "the owner pauses the running conversion"
    );
    assert!(
        db.park_paused_item(&execution, task)
            .await
            .expect("park the paused conversion"),
        "the owner parks the conversion without releasing its remote identity"
    );

    let row = sqlx::query_file!(
        "tests/sql/docling_remote_jobs/requeued_item_state.sql",
        item
    )
    .fetch_one(db.pool())
    .await
    .expect("read the parked item");
    assert_eq!(row.status, "queued");
    assert!(row.waiting_reason.is_none());

    let adopted = db
        .get_active_docling_remote_job_for_item(item)
        .await
        .unwrap()
        .expect("the paused owner keeps its active remote job");
    assert_eq!(adopted.id, job.id);
    assert_eq!(adopted.remote_task_id, job.remote_task_id);
    assert_eq!(adopted.endpoint_base_url, job.endpoint_base_url);

    let claimed = db.claim_items_fast(10).await.expect("claim while paused");
    assert!(
        !claimed.iter().any(|row| row.id == item),
        "the paused conversion is never claimed again"
    );

    cleanup(&db, task, user).await;
}

#[cfg(feature = "integration-test-helpers")]
enum AdoptionFailure {
    Returned(anyhow::Error),
    Panicked(String),
}

#[cfg(feature = "integration-test-helpers")]
fn adoption_panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Runs the owned adoption setup, turning an unwind into a reported failure so
/// the caller can restore settings and dispose the rows it created.
#[cfg(feature = "integration-test-helpers")]
async fn catch_adoption_failure<F>(future: F) -> Result<(), AdoptionFailure>
where
    F: std::future::Future<Output = anyhow::Result<()>>,
{
    use futures::FutureExt;
    match std::panic::AssertUnwindSafe(future).catch_unwind().await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(AdoptionFailure::Returned(error)),
        Err(payload) => Err(AdoptionFailure::Panicked(adoption_panic_message(payload))),
    }
}

#[cfg(feature = "integration-test-helpers")]
async fn run_adoption_case(db: &Database, fail_before_claim: bool) -> anyhow::Result<()> {
    // The prior singleton is a read that owns nothing, so it can be captured
    // before the cleanup-protected scope starts.
    let prior_settings = db
        .get_docling_settings()
        .await
        .expect("read the prior docling settings");

    let mut mock: Option<DoclingMock> = None;
    let mut user: Option<i64> = None;
    let mut group: Option<i64> = None;
    let mut task: Option<Uuid> = None;
    let mut settings_written = false;

    let outcome = catch_adoption_failure(async {
        // The loopback endpoint is the first owned resource; every later
        // fallible setup step stays inside this outcome scope.
        let started_mock = DoclingMock::start().await?;
        let root_url = started_mock.root_url().to_string();
        mock = Some(started_mock);

        let uid = try_seed_user(&db).await?;
        user = Some(uid);
        let gid = try_seed_group(&db).await?;
        group = Some(gid);
        let (tid, item) = try_seed_file_task(&db, uid, gid).await?;
        task = Some(tid);
        try_seed_file(&db, gid, item).await?;

        db.save_docling_settings(&StoredDoclingSettings {
            base_url: root_url.clone(),
            timeout_secs: 30,
            poll_interval_secs: 1,
            task_timeout_secs: 600,
            max_inflight: 1,
            pdf_backend: None,
            images_scale: None,
            image_export_mode: None,
            do_ocr: false,
            force_ocr: false,
            ocr_engine: None,
            ocr_lang: Vec::new(),
            do_code_enrichment: false,
            do_formula_enrichment: false,
            do_picture_description: false,
            openai_base_url: None,
            api_key: None,
            vlm_pipeline_model: None,
            picture_description_model: None,
            code_formula_model: None,
            picture_description_preset: None,
        })
        .await
        .context("point the docling settings at the mock")?;
        settings_written = true;

        if fail_before_claim {
            anyhow::bail!(
                "injected Docling setup failure after the settings write and before the claim"
            );
        }

        let remote_task_id = format!("docling-adoption-{}", Uuid::new_v4());
        let job = db
            .create_docling_remote_job(
                tid,
                item,
                &remote_task_id,
                Some("submitted"),
                Some(Utc::now()),
                Some(Utc::now() + Duration::hours(1)),
                Some(&format!("{root_url}/v1")),
            )
            .await
            .context("create the active remote job")?;

        let owner = try_claim_item(&db, item).await?;
        let execution = owner
            .execution_context()
            .context("the claimed item carries its captured execution")?;
        anyhow::ensure!(
            db.request_task_pause(tid, uid, true).await?,
            "the owner pauses the running conversion"
        );
        db.park_paused_item(&execution, tid).await?;
        anyhow::ensure!(
            db.get_active_docling_remote_job_for_item(item)
                .await?
                .is_some(),
            "the park keeps the active remote"
        );
        anyhow::ensure!(
            db.request_task_resume(tid, uid, true).await?,
            "the resume reopens the parked owner"
        );
        db.reconcile_docling_remote_state().await?;

        let reclaimed = try_claim_item(&db, item).await?;
        let service = build_task_service(&db).await?;
        let run_result = service.run_item_for_test(reclaimed).await;

        // The adoption is the resumed item polling and fetching the remote the
        // parked owner submitted, so no submit may happen and exactly one
        // result is fetched through the captured endpoint.
        anyhow::ensure!(
            mock.as_ref().expect("the docling mock exists").submits() == 0,
            "the adoption never submits a second remote conversion"
        );
        anyhow::ensure!(
            mock.as_ref().expect("the docling mock exists").statuses() >= 1,
            "the adopted remote is polled through the captured endpoint"
        );
        anyhow::ensure!(
            mock.as_ref().expect("the docling mock exists").results() == 1,
            "the adopted remote result is fetched exactly once"
        );
        let row: (
            String,
            String,
            Option<String>,
            Option<chrono::DateTime<Utc>>,
        ) = sqlx::query_as(
            "SELECT status, remote_task_id, remote_status, finished_at \
                 FROM context69.task_docling_remote_jobs WHERE id = $1",
        )
        .bind(job.id)
        .fetch_one(db.pool())
        .await?;
        anyhow::ensure!(
            row.0 == "succeeded" && row.1 == remote_task_id,
            "the persisted job keeps its identity and succeeds: {row:?}"
        );
        anyhow::ensure!(
            row.2.as_deref() == Some("success"),
            "the remote status is recorded"
        );
        anyhow::ensure!(row.3.is_some(), "the adopted job finishes");
        // The item left Docling because the fetched sections advanced it to the
        // later indexing stage. That stage has no library ingest runtime in this
        // case, so the item records the indexing failure; this case observes the
        // adoption and the advance past Docling, not the downstream stage.
        let stage: (Option<String>, Option<String>) =
            sqlx::query_as("SELECT status, failure_stage FROM context69.task_items WHERE id = $1")
                .bind(item)
                .fetch_one(db.pool())
                .await?;
        anyhow::ensure!(
            stage.1.as_deref() == Some("indexing"),
            "the adopted item advanced past Docling into indexing: {stage:?}"
        );
        if let Err(error) = run_result {
            anyhow::bail!("the driver stopped after the adoption boundary: {error:#}");
        }
        Ok(())
    })
    .await;

    // Cleanup always runs, including after a memory or claim failure that
    // happened after the mock, settings, or rows were created.
    let mut cleanup_failures: Vec<String> = Vec::new();
    if let Some(mock) = mock.take() {
        mock.shutdown().await;
    }
    if settings_written {
        if let Err(error) = restore_docling_settings(&db, prior_settings).await {
            cleanup_failures.push(format!("restore the prior docling settings: {error:#}"));
        }
    }
    if let Some(tid) = task {
        if let Err(error) = try_delete_task(&db, tid).await {
            cleanup_failures.push(format!("delete the adoption task: {error:#}"));
        }
    }
    if let Some(uid) = user {
        if let Err(error) = try_delete_user(&db, uid).await {
            cleanup_failures.push(format!("delete the adoption user: {error:#}"));
        }
    }
    if let Some(gid) = group {
        if let Err(error) = try_delete_group(&db, gid).await {
            cleanup_failures.push(format!("delete the adoption group: {error:#}"));
        }
    }

    let cleanup_suffix = || {
        if cleanup_failures.is_empty() {
            String::new()
        } else {
            format!("\ncleanup also failed: {}", cleanup_failures.join("; "))
        }
    };
    match outcome {
        Ok(()) if cleanup_failures.is_empty() => Ok(()),
        Ok(()) => Err(anyhow::anyhow!(
            "cleanup failed after a passing case: {}",
            cleanup_failures.join("; ")
        )),
        Err(AdoptionFailure::Returned(error)) => {
            Err(anyhow::anyhow!("{error:#}{}", cleanup_suffix()))
        }
        Err(AdoptionFailure::Panicked(message)) => Err(anyhow::anyhow!(
            "the case panicked: {message}{}",
            cleanup_suffix()
        )),
    }
}

#[cfg(feature = "integration-test-helpers")]
fn same_docling_settings(left: &StoredDoclingSettings, right: &StoredDoclingSettings) -> bool {
    left.base_url == right.base_url
        && left.timeout_secs == right.timeout_secs
        && left.poll_interval_secs == right.poll_interval_secs
        && left.task_timeout_secs == right.task_timeout_secs
        && left.max_inflight == right.max_inflight
        && left.pdf_backend == right.pdf_backend
        && left.images_scale == right.images_scale
        && left.image_export_mode == right.image_export_mode
        && left.do_ocr == right.do_ocr
        && left.force_ocr == right.force_ocr
        && left.ocr_engine == right.ocr_engine
        && left.ocr_lang == right.ocr_lang
        && left.do_code_enrichment == right.do_code_enrichment
        && left.do_formula_enrichment == right.do_formula_enrichment
        && left.do_picture_description == right.do_picture_description
        && left.openai_base_url == right.openai_base_url
        && left.api_key == right.api_key
        && left.vlm_pipeline_model == right.vlm_pipeline_model
        && left.picture_description_model == right.picture_description_model
        && left.code_formula_model == right.code_formula_model
        && left.picture_description_preset == right.picture_description_preset
}

#[cfg(feature = "integration-test-helpers")]
async fn adoption_fixture_counts(db: &Database) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT \
             (SELECT count(*) FROM context69.groups WHERE group_key LIKE 'docling-adoption-%'), \
             (SELECT count(*) FROM context69.users WHERE login_name LIKE 'docling-job-%'), \
             (SELECT count(*) FROM context69.task_docling_remote_jobs \
              WHERE remote_task_id LIKE 'docling-adoption-%')",
    )
    .fetch_one(db.pool())
    .await
    .expect("read the adoption fixture counts")
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_resumed_owner_adopts_the_active_remote_without_a_new_submit() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    run_adoption_case(&db, false)
        .await
        .unwrap_or_else(|error| panic!("{error:#}"));
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_docling_setup_failure_after_the_settings_write_restores_prior_state_and_owns_nothing() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let prior = db
        .get_docling_settings()
        .await
        .expect("read the prior docling settings");
    let counts_before = adoption_fixture_counts(&db).await;

    let error = run_adoption_case(&db, true)
        .await
        .expect_err("the injected setup failure must be reported, not masked");
    assert!(
        error.to_string().contains("injected Docling setup failure"),
        "the original failure is preserved: {error:#}"
    );

    let after = db
        .get_docling_settings()
        .await
        .expect("read the restored docling settings");
    match (prior, after) {
        (None, None) => {}
        (Some(_), None) => panic!("the setup failure deleted a pre-existing docling singleton"),
        (None, Some(_)) => panic!("the setup failure left a created docling singleton behind"),
        (Some(prior), Some(after)) => assert!(
            same_docling_settings(&prior, &after),
            "the setup failure did not restore the prior docling singleton"
        ),
    }
    assert_eq!(
        adoption_fixture_counts(&db).await,
        counts_before,
        "the setup failure left no adoption group, user, or remote job behind"
    );
}
