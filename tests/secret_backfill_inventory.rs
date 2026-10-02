//! The inventory-facing cases: what a run reports before it writes, what a dry run
//! leaves untouched, and where the mode may be reached from.
//!
//! Every assertion and every cleanup comes from the shared fixture in
//! `tests/secret_backfill.rs`, included by path because cargo builds each
//! `tests/*.rs` as a separate target.

#[path = "secret_backfill.rs"]
pub mod fixture;

use fixture::*;

/// The default posture is a read-only inventory: it reports the whole worklist and
/// changes nothing, and it still needs the deployment's master key, because an
/// unencrypted store cannot open the sealed rows it would have to inventory.
#[tokio::test]
async fn a_dry_run_reports_the_inventory_and_writes_nothing() {
    let case = case!("dry");
    case.seed().await;
    let rehearsal = run(
        &case.db,
        &store(&case.db, Some(MASTER_KEY)),
        &BackfillOptions::default(),
    )
    .await
    .expect("a dry run completes");
    let report = migrate(&case, false, 100)
        .await
        .expect("a dry run completes");

    assert_eq!(
        report, rehearsal,
        "the default mode is a dry run of the same worklist"
    );
    assert_eq!(
        (
            report.scanned,
            report.sealed,
            report.cleared,
            report.referenced,
            report.retained
        ),
        (6, 6, 4, 1, 1),
        "five singleton categories and one connection, of which four are clearable"
    );
    assert_eq!(
        (report.absent, report.already_sealed, report.unmapped),
        (0, 0, 0)
    );
    assert!(
        !report.limit_reached,
        "a generous limit leaves nothing listed"
    );

    let runtime = case.runtime().await;
    for (purpose, key_name, category) in CLEARABLE {
        assert!(
            case.stored(purpose, key_name).await.is_none(),
            "a dry run seals nothing for {purpose}"
        );
        let legacy = match category {
            "embedding" => runtime.embedding.api_key.clone(),
            "search" => case
                .db
                .get_search_settings()
                .await
                .expect("read search settings")
                .and_then(|settings| settings.api_key),
            "docling" => case
                .db
                .get_docling_settings()
                .await
                .expect("read docling settings")
                .and_then(|settings| settings.api_key),
            _ => case
                .db
                .llm_provider_api_key()
                .await
                .expect("read the shared row")
                .flatten(),
        };
        assert_eq!(
            legacy,
            Some(case.key(category)),
            "{key_name} still holds its value"
        );
    }
    assert_eq!(
        case.connection().await.database_url_secret_key,
        None,
        "a dry run writes no reference"
    );
    case.quiesce().await;
}

/// The mode is a read-only inventory bounded by a limit until `--apply` says
/// otherwise, and an argument that could silently do nothing is refused.
#[test]
fn the_mode_is_read_only_and_bounded_until_it_is_told_otherwise() {
    let parse = |args: &[&str]| BackfillOptions::parse(args.iter().map(|arg| (*arg).to_string()));
    let default = parse(&[]).expect("no arguments is the default mode");
    assert_eq!(
        (default.apply, default.limit),
        (false, DEFAULT_LIMIT),
        "a run commits nothing without --apply, and is bounded without --limit"
    );
    assert_eq!(
        parse(&["--apply", "--limit", "3"]).expect("--apply with a bound"),
        BackfillOptions {
            apply: true,
            limit: 3
        }
    );
    for args in [
        vec!["--limit", "0"],
        vec!["--limit", "-1"],
        vec!["--limit", "many"],
        vec!["--limit"],
        vec!["--dry-run"],
        vec!["--apply=false"],
        vec!["--limit=3"],
    ] {
        assert!(
            parse(&args).is_err(),
            "an argument that could do nothing or do more than asked is refused: {args:?}"
        );
    }
}

/// Every run needs a configured master key: an unkeyed store cannot open what it is
/// asked to inventory, so an apply and a rehearsal alike are refused before the first
/// read, and nothing is written.
#[tokio::test]
async fn a_run_without_a_master_key_is_refused_before_any_read() {
    let case = case!("nokey");
    case.seed().await;
    let unkeyed = store(&case.db, None);
    let failure = run(
        &case.db,
        &unkeyed,
        &BackfillOptions {
            apply: true,
            limit: 100,
        },
    )
    .await
    .expect_err("an unencrypted store cannot seal a legacy value");

    assert!(
        failure.error.to_string().contains("master_key"),
        "the refusal names the deployment input an operator has to set: {}",
        failure.error
    );
    assert_eq!(
        failure.report.scanned, 0,
        "the refusal precedes the first read"
    );
    let runtime = case.runtime().await;
    for (purpose, key_name, _) in CLEARABLE {
        assert!(
            case.stored(purpose, key_name).await.is_none(),
            "{key_name} was not sealed"
        );
    }
    assert_eq!(runtime.embedding.api_key, Some(case.key("embedding")));
    let rehearsal = run(&case.db, &unkeyed, &BackfillOptions::default())
        .await
        .expect_err("a dry run cannot claim an inventory it cannot open");
    assert!(
        rehearsal.error.to_string().contains("master_key"),
        "the refusal is the same for a rehearsal: {}",
        rehearsal.error
    );
    assert_eq!(
        (rehearsal.report.scanned, rehearsal.report.sealed),
        (0, 0),
        "an unkeyed run reports nothing it could not substantiate"
    );
    case.quiesce().await;
}

/// The shared provider row is the only credential in its table this phase owns, and
/// its siblings keep their own values.
#[tokio::test]
async fn the_shared_provider_key_migrates_without_its_siblings() {
    let case = case!("siblings");
    case.seed().await;
    for sibling in ["deepl", "libretranslate"] {
        sqlx::query(
            "UPDATE context69.translation_provider_settings SET api_key = $1 WHERE provider_key = $2",
        )
        .bind(case.key(sibling))
        .bind(sibling)
        .execute(case.db.pool())
        .await
        .expect("seed a sibling provider key");
    }
    let report = migrate(&case, true, 100)
        .await
        .expect("an apply run completes");

    assert_eq!(
        case.stored(
            SecretPurpose::TranslationProviderApiKey,
            key_names::TRANSLATION_PROVIDER_API_KEY
        )
        .await
        .as_deref(),
        Some(case.key("llm").as_str()),
        "the shared row is sealed under the shared key"
    );
    let owned: Vec<String> = sqlx::query_scalar(
        "SELECT key FROM context69.internal_secrets WHERE purpose = 'translation.api_key'",
    )
    .fetch_all(case.db.pool())
    .await
    .expect("count the sealed shared rows");
    assert_eq!(
        owned,
        vec![key_names::TRANSLATION_PROVIDER_API_KEY],
        "one owned row"
    );

    for sibling in ["deepl", "libretranslate"] {
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT api_key FROM context69.translation_provider_settings WHERE provider_key = $1",
        )
        .bind(sibling)
        .fetch_one(case.db.pool())
        .await
        .expect("read the sibling row");
        assert_eq!(
            stored,
            Some(case.key(sibling)),
            "{sibling} keeps its own credential"
        );
    }
    assert_eq!(
        report.cleared, 4,
        "only the four eligible columns are cleared"
    );
    case.quiesce().await;
}

/// The mode is reachable only as an explicit argument, and it is dispatched before the
/// application is built, so it can never run on a serving process's startup path and
/// never opens Valkey, Qdrant, the API, or the MCP server.
#[test]
fn the_backfill_is_dispatched_before_the_application_is_built() {
    const MAIN: &str = include_str!("../src/main.rs");
    let dispatch = MAIN
        .find("\"backfill-secrets\"")
        .expect("the mode is dispatched from argv");
    let app = MAIN
        .find("Context69App::new(")
        .expect("the serving process builds the application");
    assert!(
        dispatch < app,
        "the backfill is a maintenance mode, so it must be dispatched before Context69App::new is ever called"
    );
    assert_eq!(
        MAIN.matches("\"backfill-secrets\"").count(),
        1,
        "the mode is reached from exactly one place: no second, automatic path"
    );
}
