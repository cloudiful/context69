#[path = "benchmark_helpers.rs"]
mod helpers;

use super::{BENCHMARK_DATABASE, connect_owned_database, unique_group_key};
use super::{fixtures, oracle};
use anyhow::{Context, Result, ensure};
use context69::db::Database;
use serde::Serialize;
use serde_json::Value;

const CURRENT_SQL: &str = include_str!("../../../src/sql/db/documents/keyword_search.sql");
const CHUNKS_PER_DOCUMENT: i32 = 8;
const MAX_DOCUMENTS: i64 = 60_000;
const MAX_GENERATED_TEXT_BYTES: i64 = 512 * 1024 * 1024;

#[derive(Serialize)]
struct BenchmarkReport {
    git_sha: String,
    postgres_version: String,
    database_settings: Value,
    seed_tag: String,
    document_count: i64,
    chunk_count: i64,
    profiles: Vec<ProfileReport>,
}

#[derive(Serialize)]
struct ProfileReport {
    name: &'static str,
    query: String,
    locale: Option<String>,
    returned_rows: usize,
    baseline_median_ms: f64,
    current_median_ms: f64,
    baseline_samples_ms: Vec<f64>,
    current_samples_ms: Vec<f64>,
    baseline_plan: Value,
    current_plan: Value,
}

struct Profile {
    name: &'static str,
    query: &'static str,
    locale: Option<&'static str>,
}

pub async fn run(database_url: &str) {
    let db = connect_owned_database(database_url, BENCHMARK_DATABASE).await;
    let seed_tag = unique_group_key("benchmark");
    let group_path = format!("issue62/{seed_tag}/benchmark");
    let result = collect_report(&db, &seed_tag, &group_path).await;
    let cleanup = helpers::cleanup_group(&db, &seed_tag).await;
    db.pool().close().await;
    cleanup.expect("remove only the benchmark's synthetic group and rows");
    let report = result.expect("bounded synthetic keyword benchmark");
    let output = helpers::output_path();
    std::fs::create_dir_all(output.parent().expect("benchmark output parent"))
        .expect("create issue-local benchmark output directory");
    std::fs::write(
        &output,
        serde_json::to_vec_pretty(&report).expect("serialize benchmark report"),
    )
    .expect("write benchmark report only under the issue cache");
    println!("keyword candidate baseline report: {}", output.display());
}

async fn collect_report(
    db: &Database,
    seed_tag: &str,
    group_path: &str,
) -> Result<BenchmarkReport> {
    let documents = std::env::var("CONTEXT69_KEYWORD_BENCHMARK_DOCUMENTS")
        .ok()
        .map(|value| value.parse::<i64>())
        .transpose()
        .context("benchmark document count must be an integer")?
        .unwrap_or(10_000);
    ensure!(
        (10..=MAX_DOCUMENTS).contains(&documents),
        "benchmark document count must be between 10 and 60,000"
    );
    ensure!(
        documents * i64::from(CHUNKS_PER_DOCUMENT) * 1024 <= MAX_GENERATED_TEXT_BYTES,
        "benchmark seed would exceed the 512 MiB generated-text cap"
    );

    let (actual_documents, actual_chunks): (i64, i64) =
        sqlx::query_as(include_str!("../fixtures/benchmark_seed.sql"))
            .bind(seed_tag)
            .bind(group_path)
            .bind(seed_tag)
            .bind(documents)
            .fetch_one(db.pool())
            .await?;
    let expected_chunks = documents * i64::from(CHUNKS_PER_DOCUMENT);
    ensure!(
        actual_documents == documents,
        "benchmark seed inserted an unexpected document count"
    );
    ensure!(
        actual_chunks == expected_chunks,
        "benchmark seed inserted an unexpected chunk count"
    );
    sqlx::query("ANALYZE context69.documents")
        .execute(db.pool())
        .await?;
    sqlx::query("ANALYZE context69.document_chunks")
        .execute(db.pool())
        .await?;

    let (postgres_version, database_settings) = helpers::database_facts(db).await?;
    let profiles = profiles();
    let mut results = Vec::with_capacity(profiles.len());
    for profile in profiles {
        results.push(measure_profile(db, profile, group_path).await?);
    }
    Ok(BenchmarkReport {
        git_sha: std::env::var("CONTEXT69_KEYWORD_GIT_SHA")
            .context("CONTEXT69_KEYWORD_GIT_SHA must identify the measured revision")?,
        postgres_version,
        database_settings,
        seed_tag: seed_tag.to_owned(),
        document_count: actual_documents,
        chunk_count: actual_chunks,
        profiles: results,
    })
}

fn profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "rare-single-word",
            query: "rareneedle",
            locale: None,
        },
        Profile {
            name: "rare-single-word-locale",
            query: "rareneedle",
            locale: Some("fr"),
        },
        Profile {
            name: "rare-two-terms-cross-field",
            query: "twotermalpha twotermbeta",
            locale: None,
        },
        Profile {
            name: "rare-three-terms-cross-field",
            query: "threetermalpha threetermbeta threetermgamma",
            locale: None,
        },
        Profile {
            name: "common-word",
            query: "commonneedle",
            locale: None,
        },
        Profile {
            name: "short-cjk",
            query: "中",
            locale: None,
        },
        Profile {
            name: "wildcard-pattern",
            query: "common%needle",
            locale: None,
        },
    ]
}

async fn measure_profile(
    db: &Database,
    profile: Profile,
    group_path: &str,
) -> Result<ProfileReport> {
    let mut request = fixtures::request(profile.query, group_path);
    request.source_key = Some("keyword-benchmark".to_owned());
    request.locale = profile.locale.map(ToOwned::to_owned);
    let scope = fixtures::public_scope();
    let limit = 100;
    let baseline_plan =
        helpers::explain(db, oracle::FROZEN_HEAD_SQL, &request, &scope, limit).await?;
    let current_plan = helpers::explain(db, CURRENT_SQL, &request, &scope, limit).await?;
    let _ = helpers::sample(db, &request, &scope, limit, false).await?;
    let _ = helpers::sample(db, &request, &scope, limit, true).await?;

    let mut baseline_samples_ms = Vec::with_capacity(3);
    let mut current_samples_ms = Vec::with_capacity(3);
    let mut returned_rows = 0;
    for iteration in 0..3 {
        let (baseline, current) = if iteration % 2 == 0 {
            let baseline = helpers::sample(db, &request, &scope, limit, false).await?;
            let current = helpers::sample(db, &request, &scope, limit, true).await?;
            (baseline, current)
        } else {
            let current = helpers::sample(db, &request, &scope, limit, true).await?;
            let baseline = helpers::sample(db, &request, &scope, limit, false).await?;
            (baseline, current)
        };
        ensure!(
            !baseline.1.is_empty(),
            "{} synthetic profile must return matches",
            profile.name
        );
        ensure!(
            helpers::serialize_hits(&baseline.1)? == helpers::serialize_hits(&current.1)?,
            "{} differs from the frozen oracle on the synthetic workload",
            profile.name
        );
        returned_rows = baseline.1.len();
        baseline_samples_ms.push(baseline.0);
        current_samples_ms.push(current.0);
    }
    Ok(ProfileReport {
        name: profile.name,
        query: profile.query.to_owned(),
        locale: profile.locale.map(ToOwned::to_owned),
        returned_rows,
        baseline_median_ms: helpers::median(&baseline_samples_ms),
        current_median_ms: helpers::median(&current_samples_ms),
        baseline_samples_ms,
        current_samples_ms,
        baseline_plan,
        current_plan,
    })
}
