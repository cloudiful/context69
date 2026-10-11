use anyhow::Result;
use context69::contracts::{SearchHit, SearchRequest};
use context69::db::Database;
use context69::domain::AccessScope;
use serde_json::Value;
use sqlx::Row;
use std::path::{Component, PathBuf};
use std::time::Instant;

use super::oracle;

pub async fn database_facts(db: &Database) -> Result<(String, Value)> {
    let row = sqlx::query(
        "SELECT version(), jsonb_build_object( \
         'shared_buffers', current_setting('shared_buffers'), \
         'work_mem', current_setting('work_mem'), \
         'random_page_cost', current_setting('random_page_cost'), \
         'effective_cache_size', current_setting('effective_cache_size'), \
         'statement_timeout', current_setting('statement_timeout')) AS settings",
    )
    .fetch_one(db.pool())
    .await?;
    Ok((row.try_get(0)?, row.try_get(1)?))
}

pub async fn sample(
    db: &Database,
    request: &SearchRequest,
    scope: &AccessScope,
    limit: usize,
    current: bool,
) -> Result<(f64, Vec<SearchHit>)> {
    let started = Instant::now();
    let hits = if current {
        db.keyword_search(request, scope, limit).await?
    } else {
        oracle::search(db, request, scope, limit).await?
    };
    Ok((started.elapsed().as_secs_f64() * 1000.0, hits))
}

pub async fn explain(
    db: &Database,
    sql: &str,
    request: &SearchRequest,
    scope: &AccessScope,
    limit: usize,
) -> Result<Value> {
    let statement =
        Box::leak(format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}").into_boxed_str());
    let row = bind(statement, request, scope, limit)
        .fetch_one(db.pool())
        .await?;
    Ok(row.try_get::<Value, _>(0)?)
}

fn bind(
    sql: &'static str,
    request: &SearchRequest,
    scope: &AccessScope,
    limit: usize,
) -> sqlx::query::Query<'static, sqlx::Postgres, sqlx::postgres::PgArguments> {
    let normalized = request.query.trim().to_lowercase();
    let pattern = format!("%{normalized}%");
    let terms = normalized
        .split(|ch: char| ch.is_whitespace() || ch.is_ascii_punctuation())
        .filter(|term| !term.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let filters =
        serde_json::to_value(&request.metadata_filters).expect("serialize benchmark filters");
    sqlx::query(sql)
        .bind(normalized)
        .bind(pattern)
        .bind(terms)
        .bind(request.source_key.as_deref())
        .bind(request.group_path.as_deref())
        .bind(request.published_after)
        .bind(request.published_before)
        .bind(scope.private_group_ids.clone())
        .bind(i64::try_from(limit).expect("bounded benchmark limit"))
        .bind(request.locale.as_deref())
        .bind(filters)
}

pub fn serialize_hits(hits: &[SearchHit]) -> Result<Vec<Value>> {
    hits.iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

pub async fn cleanup_group(db: &Database, group_key: &str) -> Result<()> {
    sqlx::query(include_str!("../fixtures/cleanup.sql"))
        .bind(group_key)
        .execute(db.pool())
        .await?;
    Ok(())
}

pub fn output_path() -> PathBuf {
    let root = PathBuf::from(
        std::env::var("HOME").expect("HOME is required for issue-local cache output"),
    )
    .join(".cache/opencode/context69-62");
    let output = std::env::var_os("CONTEXT69_KEYWORD_BENCHMARK_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("keyword-search-p2.json"));
    ensure_safe_output(&root, &output);
    output
}

fn ensure_safe_output(root: &std::path::Path, output: &std::path::Path) {
    assert!(
        output.is_absolute(),
        "benchmark report path must be absolute"
    );
    assert!(
        output.starts_with(root),
        "benchmark output must stay under the issue cache"
    );
    assert!(
        output
            .components()
            .all(|component| !matches!(component, Component::ParentDir)),
        "benchmark output must not contain parent traversal"
    );
}
