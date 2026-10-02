//! The retry-facing cases: apply, repeat, bounded, and the failures a run survives.
//!
//! Every assertion and every cleanup comes from the shared fixture in
//! `tests/secret_backfill.rs`, included by path because cargo builds each
//! `tests/*.rs` as a separate target.

#[path = "secret_backfill.rs"]
pub mod fixture;

use fixture::*;

/// An apply seals every source, clears the eligible columns, and defers the rest.
#[tokio::test]
async fn an_apply_seals_before_it_clears_and_defers_the_unsafe_columns() {
    let case = case!("apply");
    case.seed().await;
    let report = apply(&case).await;

    assert_eq!(
        (
            report.sealed,
            report.cleared,
            report.referenced,
            report.retained
        ),
        (6, 4, 1, 1),
        "every source is sealed, only the eligible columns are cleared"
    );
    let runtime = case.runtime().await;
    for (purpose, key_name, category) in CLEARABLE {
        assert_sealed(&case, purpose, key_name, &case.key(category)).await;
    }
    assert_eq!(
        runtime.embedding.api_key, None,
        "the legacy key column is cleared"
    );
    assert_eq!(
        llm_legacy(&case).await,
        None,
        "the shared provider column is cleared"
    );

    // The S3 secret key is sealed but its column stays: nulling it would collapse the
    // all-fields-required projection its readers use, and the access key is an
    // identifier this phase never moves.
    let s3 = case
        .runtime()
        .await
        .file_library
        .s3
        .expect("the S3 projection");
    assert_eq!(
        case.stored(
            SecretPurpose::RuntimeS3SecretKey,
            key_names::RUNTIME_S3_SECRET_KEY
        )
        .await
        .as_deref(),
        Some(case.key("s3").as_str()),
        "the retained key is sealed like the rest"
    );
    assert_eq!(
        s3.secret_key,
        case.key("s3"),
        "the S3 column is retained until the column removal"
    );
    assert_eq!(
        s3.access_key, "backfill-access",
        "the access key keeps its existing outward behavior"
    );

    // The connection keeps its DSN and its stable identity, and gains the reference
    // the application itself would have written for it.
    let connection = case.connection().await;
    let expected = case.connection_key(&connection).await;
    assert_eq!(
        case.stored(SecretPurpose::SourceConnectionDatabaseUrl, &expected)
            .await
            .as_deref(),
        Some(case.dsn().as_str()),
        "the DSN is sealed under its own stable identity"
    );
    assert_eq!(
        connection.database_url_secret_key.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(
        connection.database_url,
        case.dsn(),
        "the legacy DSN survives"
    );
    assert_eq!(
        connection.connection_key.to_string(),
        expected.trim_start_matches(key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX),
        "the connection keeps the identity it was created with"
    );

    let rendered = format!("{report} {report:?}");
    let counts = [
        "scanned=6",
        "sealed=6",
        "cleared=4",
        "referenced=1",
        "retained=1",
    ];
    for field in counts {
        assert!(rendered.contains(field), "{field} is reported: {rendered}");
    }
    for secret in [
        case.key("embedding"),
        case.key("llm"),
        case.dsn(),
        case.connection.clone(),
    ] {
        assert!(
            !rendered.contains(&secret),
            "a report discloses nothing: {rendered}"
        );
    }
    case.quiesce().await;
}

/// A repeat run converges, and a sealed row still lets its pending clear land.
#[tokio::test]
async fn a_repeat_run_is_idempotent_and_a_sealed_row_still_lets_the_clear_finish() {
    let case = case!("retry");
    case.seed().await;
    apply(&case).await;
    let sealed = case
        .metadata(key_names::EMBEDDING_API_KEY)
        .await
        .expect("a sealed row");
    // The state a run interrupted between its write and its clear leaves behind.
    sqlx::query(
        "UPDATE context69.runtime_embedding_settings SET api_key = $1 WHERE singleton = TRUE",
    )
    .bind(case.key("embedding"))
    .execute(case.db.pool())
    .await
    .expect("rewrite the legacy column");
    let retry = apply(&case).await;
    assert_eq!(
        (retry.sealed, retry.cleared, retry.referenced),
        (0, 1, 0),
        "a sealed row is finished work: nothing is rewritten, only the clear lands"
    );
    assert_eq!(case.runtime().await.embedding.api_key, None);
    assert_eq!(
        case.metadata(key_names::EMBEDDING_API_KEY).await,
        Some(sealed),
        "a finished item is not re-sealed"
    );

    let repeat = apply(&case).await;
    assert_eq!(
        (
            repeat.scanned,
            repeat.absent,
            repeat.sealed,
            repeat.cleared,
            repeat.referenced
        ),
        (6, 4, 0, 0, 0),
        "every legacy column is cleared and every retained item is already sealed"
    );
    case.quiesce().await;
}

/// The limit bounds a run's changes, and repeated runs finish the worklist.
#[tokio::test]
async fn a_run_is_bounded_by_its_limit_and_resumes() {
    let case = case!("limit");
    case.seed().await;
    let first = apply_bounded(&case).await;
    assert_eq!(
        (first.sealed, first.limit_reached),
        (1, true),
        "one item per run, and the run says that work is still listed"
    );

    let mut runs = 1;
    while apply_bounded(&case).await.limit_reached {
        runs += 1;
        assert!(
            runs < 25,
            "a bounded worklist finishes in a bounded number of runs"
        );
    }
    assert_eq!(case.runtime().await.embedding.api_key, None);
    assert_eq!(
        case.stored(
            SecretPurpose::RuntimeS3SecretKey,
            key_names::RUNTIME_S3_SECRET_KEY
        )
        .await
        .as_deref(),
        Some(case.key("s3").as_str()),
        "the resumes finish the whole worklist, including the retained key"
    );
    case.quiesce().await;
}

/// A legacy store row is re-sealed only through a catalogue-declared owner.
#[tokio::test]
async fn a_legacy_store_row_is_sealed_only_through_the_catalogue() {
    let case = case!("legacyrow");
    let unmapped = format!("operator_note_{}", case.tag);
    let record_scoped = format!("{}g7.{}", key_names::GIT_PROVIDER_TOKEN_PREFIX, case.tag);
    for (key_name, value) in [
        (key_names::DOCLING_VLM_API_KEY, case.key("singleton-row")),
        (record_scoped.as_str(), case.key("record-row")),
        (unmapped.as_str(), case.key("unmapped-row")),
    ] {
        case.seed_legacy_store_row(key_name, &value).await;
    }

    let report = apply(&case).await;
    assert_eq!(
        (report.unmapped, report.sealed),
        (1, 2),
        "a singleton and a record-scoped key are owned; an unknown one is counted"
    );
    for (purpose, key_name, expected) in [
        (
            SecretPurpose::DoclingVlmApiKey,
            key_names::DOCLING_VLM_API_KEY,
            case.key("singleton-row"),
        ),
        (
            SecretPurpose::GitProviderToken,
            record_scoped.as_str(),
            case.key("record-row"),
        ),
    ] {
        let (owner, version, _) = case.metadata(key_name).await.expect("a sealed row");
        assert_eq!(
            (owner.as_deref(), version),
            (Some(purpose.as_str()), 1),
            "{key_name} is re-sealed under its own purpose"
        );
        assert_eq!(
            case.stored(purpose, key_name).await.as_deref(),
            Some(expected.as_str()),
            "the re-sealed row opens to the value it held"
        );
    }
    assert_eq!(
        case.metadata(&unmapped).await,
        Some((None, 0, 0)),
        "an unmapped row is left exactly as it was"
    );
    case.quiesce().await;
}

/// The first failure stops the run, keeps the legacy value, and resumes after repair.
#[tokio::test]
async fn a_store_row_this_purpose_does_not_own_stops_the_run_before_any_clear() {
    let case = case!("crossed");
    case.seed().await;
    sqlx::query(CROSSED_ROW)
        .bind(key_names::EMBEDDING_API_KEY)
        .bind(b"sealed-by-another-purpose".as_slice())
        .execute(case.db.pool())
        .await
        .expect("seed a row another purpose owns");

    let failure = migrate(&case, true, 100)
        .await
        .expect_err("a crossed row stops the run");
    assert!(
        failure.error.to_string().contains("embedding.api_key"),
        "the failure names the purpose and never a value: {}",
        failure.error
    );
    assert_eq!(failure.report.cleared, 0, "no legacy column was cleared");
    let runtime = case.runtime().await;
    assert_eq!(
        runtime.embedding.api_key,
        Some(case.key("embedding")),
        "the failing item keeps its legacy value"
    );

    sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
        .bind(key_names::EMBEDDING_API_KEY)
        .execute(case.db.pool())
        .await
        .expect("repair the crossed row");
    let report = apply(&case).await;
    let connection = case.connection().await;
    assert_eq!(
        (
            report.cleared,
            connection.database_url_secret_key.as_deref()
        ),
        (4, Some(case.connection_key(&connection).await.as_str())),
        "the same run resumes, clears the worklist, and repairs the reference"
    );
    case.quiesce().await;
}
