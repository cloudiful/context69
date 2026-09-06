//! Read-only repair-preview audit for missing `document_versions` rows.
//!
//! Scans documents whose current `documents.record_hash` has no matching
//! `document_versions` row, pages deterministically by document id,
//! reconstructs each body from ordered chunks, and classifies with the same
//! application normalization/hash semantics as the write path.
//!
//! Read-only by construction: connects with `Database::connect_read_only`
//! (no migrations) and only runs SELECT queries through
//! `audit_missing_versions`. There is no apply/backfill, task-retry, or
//! write flag. Output is a concise JSON summary with identifier counts and
//! id samples; document bodies and chunk texts are never printed.

use anyhow::{Context, Result};
use context69::db::Database;
use db_init::{load_dotenv_if_exists, resolve_database_url};
use serde::Serialize;

const DEFAULT_PAGE_SIZE: i64 = 200;
const DEFAULT_MAX_DOCUMENTS: i64 = 5000;
const DEFAULT_SAMPLE_SIZE: usize = 20;

#[derive(Debug, Clone, Serialize)]
struct AuditReport {
    read_only: bool,
    database_url_source: String,
    page_size: i64,
    max_documents: i64,
    sample_size: usize,
    #[serde(flatten)]
    summary: context69::db::AuditSummary,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("fatal: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args = parse_args()?;
    if args.help {
        print_help();
        return Ok(());
    }
    load_dotenv_if_exists(".env").context("load repository .env for read-only audit")?;
    let resolution = resolve_database_url(
        args.database_url,
        &["CONTEXT69_APP_DB__URL", "DATABASE_URL"],
        || Ok(None),
    )
    .context(
        "missing database URL; pass --database-url or set CONTEXT69_APP_DB__URL/DATABASE_URL",
    )?;
    let source = match &resolution.source {
        db_init::DatabaseUrlSource::CliArgument => "--database-url".to_string(),
        db_init::DatabaseUrlSource::EnvVar(name) => format!("env:{name}"),
        db_init::DatabaseUrlSource::ConfigValue => "config".to_string(),
    };
    eprintln!("read-only audit: no migrations, no writes; using {source}");

    // Explicit no-migration connection. This binary must never call
    // `Database::connect` (which runs migrations) nor any INSERT/UPDATE/DELETE.
    let db = Database::connect_read_only(&resolution.database_url)
        .await
        .context("connect read-only audit pool")?;
    let summary = context69::db::audit_missing_versions(
        db.pool(),
        args.page_size,
        args.max_documents,
        args.sample_size,
    )
    .await
    .context("run read-only missing-version audit")?;

    let report = AuditReport {
        read_only: true,
        database_url_source: source,
        page_size: summary.page_size,
        max_documents: summary.max_documents,
        sample_size: args.sample_size,
        summary,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[derive(Debug, Default)]
struct Args {
    database_url: Option<String>,
    page_size: i64,
    max_documents: i64,
    sample_size: usize,
    help: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        page_size: DEFAULT_PAGE_SIZE,
        max_documents: DEFAULT_MAX_DOCUMENTS,
        sample_size: DEFAULT_SAMPLE_SIZE,
        ..Args::default()
    };
    let mut raw = std::env::args().skip(1).peekable();
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "--database-url" | "-d" => {
                let value = raw.next().ok_or_else(|| {
                    anyhow::anyhow!(
                        "missing value for {arg}; expected --database-url <postgres-url>"
                    )
                })?;
                args.database_url = Some(value);
            }
            "--page-size" => {
                let value = raw
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing value for --page-size <n>"))?;
                args.page_size = value.parse().context("--page-size must be an integer")?;
            }
            "--max-documents" => {
                let value = raw
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing value for --max-documents <n>"))?;
                args.max_documents = value
                    .parse()
                    .context("--max-documents must be an integer")?;
            }
            "--sample-size" => {
                let value = raw
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing value for --sample-size <n>"))?;
                args.sample_size = value.parse().context("--sample-size must be an integer")?;
            }
            "--help" | "-h" => {
                args.help = true;
                return Ok(args);
            }
            other => {
                anyhow::bail!(
                    "unsupported argument: {other}; run with --help for read-only audit options"
                );
            }
        }
    }
    if args.page_size < 1 || args.page_size > 1000 {
        anyhow::bail!("--page-size must be between 1 and 1000");
    }
    if args.max_documents < 1 || args.max_documents > 100_000 {
        anyhow::bail!("--max-documents must be between 1 and 100000");
    }
    if args.sample_size > 100 {
        anyhow::bail!("--sample-size must be between 0 and 100");
    }
    Ok(args)
}

fn print_help() {
    println!("document_versions_audit (read-only, issue 139 phase 3)");
    println!();
    println!("Scans documents missing a matching document_versions row, pages by");
    println!("document id, and classifies each candidate without writing or migrating.");
    println!();
    println!("Usage: document_versions_audit [--database-url <postgres-url>]");
    println!("       [--page-size <1-1000, default 200>]");
    println!("       [--max-documents <1-100000, default 5000>]");
    println!("       [--sample-size <0-100, default 20>]");
    println!();
    println!("Database URL resolution (read-only):");
    println!("  1. --database-url");
    println!("  2. CONTEXT69_APP_DB__URL");
    println!("  3. DATABASE_URL (repository .env is loaded if present)");
    println!();
    println!("Output: JSON summary with counts and document-id samples only.");
    println!("Document bodies and chunk texts are never printed.");
    println!("No apply/backfill or task-retry behavior is available in this phase.");
}
