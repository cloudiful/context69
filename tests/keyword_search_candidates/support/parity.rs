use anyhow::{Context, Result, ensure};
use context69::contracts::{SearchHit, TranslationStatus};
use context69::db::Database;
use serde_json::Value;

use super::{TEST_DATABASE, connect_owned_database};
use super::{fixtures, oracle};

pub async fn run(database_url: &str) {
    let db = connect_owned_database(database_url, TEST_DATABASE).await;
    let fixture = fixtures::seed::seed(&db).await;
    let result = verify_cases(&db, &fixture).await;
    let cleanup = cleanup_fixture(&db, &fixture).await;
    db.pool().close().await;
    cleanup.expect("remove only the issue-owned synthetic fixture rows");
    result.expect("frozen HEAD keyword-search parity matrix");
}

async fn verify_cases(db: &Database, fixture: &fixtures::SeededFixture) -> Result<()> {
    let cases = fixtures::search_cases(fixture);
    ensure!(
        !cases.is_empty(),
        "parity matrix must execute at least one scenario"
    );
    for case in cases {
        let expected = oracle::search(db, &case.request, &case.scope, case.limit)
            .await
            .with_context(|| format!("frozen oracle scenario: {}", case.name))?;
        let actual = db
            .keyword_search(&case.request, &case.scope, case.limit)
            .await
            .with_context(|| format!("public keyword search scenario: {}", case.name))?;
        ensure!(
            serialize_hits(&actual)? == serialize_hits(&expected)?,
            "{} differs from the frozen HEAD oracle",
            case.name
        );
        ensure!(
            actual.len() == case.expected_count,
            "{} expected {} hits, got {}",
            case.name,
            case.expected_count,
            actual.len()
        );
        if let Some(reason) = case.expected_reason {
            ensure!(
                actual
                    .iter()
                    .all(|hit| hit.match_reason.as_deref() == Some(reason)),
                "{} did not retain match reason {reason}",
                case.name
            );
        }
        verify_special_fields(case.name, &actual)?;
    }
    Ok(())
}

fn serialize_hits(hits: &[SearchHit]) -> Result<Vec<Value>> {
    hits.iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn verify_special_fields(name: &str, hits: &[SearchHit]) -> Result<()> {
    match name {
        "title phrase expands all document chunks" => {
            ensure!(
                hits.len() == 2,
                "title-only expansion must return both chunks"
            );
            ensure!(
                hits.iter().all(|hit| hit.summary.is_none()),
                "non-meaningful summary must stay absent"
            );
            ensure!(
                hits.iter().all(|hit| hit.is_library_file),
                "library metadata must map to each hit"
            );
            ensure!(
                hits.iter()
                    .all(|hit| hit.library_path.as_deref() == Some("synthetic/path.md")),
                "library path metadata must be projected"
            );
        }
        "same UUID in original and translated chunk tables stays distinct" => {
            ensure!(
                hits.len() == 2,
                "expected original and current translated rows"
            );
            ensure!(
                hits[0].chunk_id == hits[1].chunk_id,
                "fixture must exercise cross-table UUID collision"
            );
            ensure!(
                hits[0].content_locale.as_deref() == Some("original"),
                "original row must sort before its translated row"
            );
            ensure!(
                hits[1].content_locale.as_deref() == Some("fr"),
                "translated row must remain distinct"
            );
            ensure!(
                hits.iter()
                    .all(|hit| hit.translation_status == Some(TranslationStatus::Succeeded)),
                "latest current translation status must be retained"
            );
        }
        "null locale performs no translation work" => {
            ensure!(
                hits.len() == 1,
                "null locale should return only the original row"
            );
            ensure!(
                hits[0].translation_status.is_none(),
                "null locale must skip job status lookup"
            );
            ensure!(
                hits[0].requested_locale.is_none(),
                "null locale stays absent in result mapping"
            );
        }
        "original fallback carries latest translation job status" => {
            ensure!(
                hits.len() == 1,
                "fallback query should return one original row"
            );
            ensure!(
                hits[0].is_fallback,
                "requested translated locale should mark fallback"
            );
            ensure!(
                hits[0].content_locale.as_deref() == Some("original"),
                "fallback row keeps its original content locale"
            );
            ensure!(
                hits[0].translation_status == Some(TranslationStatus::Failed),
                "latest translation job status must be preserved"
            );
        }
        "global ranked limit retains the full candidate boundary" => {
            ensure!(
                hits.first()
                    .is_some_and(|hit| hit.external_id == "boundary-90"),
                "most recent ranked row must lead the bounded result"
            );
            ensure!(
                hits.last()
                    .is_some_and(|hit| hit.external_id == "boundary-11"),
                "80-row boundary must retain the expected final ranked row"
            );
        }
        _ => {}
    }
    Ok(())
}

async fn cleanup_fixture(db: &Database, fixture: &fixtures::SeededFixture) -> Result<()> {
    for group_key in [&fixture.group_key, &fixture.private_group_key] {
        sqlx::query(include_str!("../fixtures/cleanup.sql"))
            .bind(group_key)
            .execute(db.pool())
            .await?;
    }
    Ok(())
}
