//! Controlled `file_library` version backfill (issue 139, phase 4).
//!
//! Default mode is a read-only preflight over `source_key = 'file_library'`
//! documents missing a version for the current `record_hash`. No writes,
//! no migrations, no task retries; output is counts and document ids only.
//!
//! Apply mode (`--apply`) requires an explicit `--database-url` and an
//! explicit `--expected-eligible-count`, runs a fresh deterministic
//! preflight, aborts without writes on scope/count drift or any unsafe
//! candidate, then backfills one transaction per document with `FOR UPDATE`
//! locking, ordered-chunk reconstruction, application-side SHA-256
//! verification, idempotent `ON CONFLICT DO NOTHING` inserts, and
//! pre-commit verification. Apply mode never falls back to repository
//! `.env` / `DATABASE_URL` for writes and never runs migrations.

use anyhow::{Context, Result};
use context69::db::{
    BackfillPreflight, Database, apply_file_library_backfill, check_backfill_preflight,
    preflight_file_library_backfill, resolve_apply_database_url,
};
use db_init::{load_dotenv_if_exists, resolve_database_url};
use serde::Serialize;

const DEFAULT_PAGE_SIZE: i64 = 200;
const DEFAULT_MAX_DOCUMENTS: i64 = 5000;
const DEFAULT_SAMPLE_SIZE: usize = 20;

#[derive(Debug, Clone, Serialize)]
struct PreviewReport {
    read_only: bool,
    apply: bool,
    database_url_source: String,
    page_size: i64,
    max_documents: i64,
    sample_size: usize,
    #[serde(flatten)]
    preflight: BackfillPreflight,
}

#[derive(Debug, Clone, Serialize)]
struct ApplyReport {
    read_only: bool,
    apply: bool,
    database_url_source: String,
    expected_eligible: usize,
    preflight: BackfillPreflight,
    result: context69::db::BackfillApplySummary,
}

#[derive(Debug, Clone, Serialize)]
struct AbortedReport {
    read_only: bool,
    apply: bool,
    database_url_source: String,
    expected_eligible: usize,
    guard_error: String,
    preflight: BackfillPreflight,
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
    if args.apply {
        run_apply(args).await
    } else {
        run_preview(args).await
    }
}

async fn run_preview(args: Args) -> Result<()> {
    load_dotenv_if_exists(".env").context("load repository .env for read-only preview")?;
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
    eprintln!("read-only backfill preview: no migrations, no writes; using {source}");

    // Explicit no-migration read-only connection. This path must never call
    // `Database::connect` (migrates) nor any INSERT/UPDATE/DELETE.
    let db = Database::connect_read_only(&resolution.database_url)
        .await
        .context("connect read-only preview pool")?;
    let preflight = preflight_file_library_backfill(
        db.pool(),
        args.page_size,
        args.max_documents,
        args.sample_size,
    )
    .await
    .context("run file_library backfill preflight")?;
    let report = PreviewReport {
        read_only: true,
        apply: false,
        database_url_source: source,
        page_size: preflight.page_size,
        max_documents: preflight.max_documents,
        sample_size: args.sample_size,
        preflight,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn run_apply(args: Args) -> Result<()> {
    // Apply mode never loads `.env` and never consults environment/config:
    // the production target must be supplied explicitly on the command line.
    let database_url = resolve_apply_database_url(args.database_url.as_deref())
        .context("apply mode requires an explicit --database-url")?;
    let Some(expected_eligible) = args.expected_eligible_count else {
        anyhow::bail!(
            "--apply requires --expected-eligible-count <n> (approved initial scope is 482); refusing to write without an explicit expected count"
        );
    };
    eprintln!(
        "controlled backfill apply: no migrations, per-document transactions; using --database-url (explicit, no .env fallback)"
    );

    // Write-capable but no-migration connection. Production schema changes
    // are forbidden here; normal startup keeps using `Database::connect`.
    let db = Database::connect_without_migrations(&database_url)
        .await
        .context("connect backfill pool without migrations")?;
    let preflight = preflight_file_library_backfill(
        db.pool(),
        args.page_size,
        args.max_documents,
        args.sample_size,
    )
    .await
    .context("run fresh backfill preflight before any insert")?;
    if let Err(guard) = check_backfill_preflight(&preflight, expected_eligible) {
        let report = AbortedReport {
            read_only: false,
            apply: true,
            database_url_source: "--database-url (explicit, no .env fallback)".to_string(),
            expected_eligible,
            guard_error: format!("{guard:#}"),
            preflight,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        anyhow::bail!("backfill preflight guard aborted apply without writes: {guard:#}");
    }
    let result = apply_file_library_backfill(db.pool(), &preflight.eligible_ids)
        .await
        .context("apply file_library backfill")?;
    let report = ApplyReport {
        read_only: false,
        apply: true,
        database_url_source: "--database-url (explicit, no .env fallback)".to_string(),
        expected_eligible,
        preflight,
        result,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[derive(Debug, Default)]
struct Args {
    database_url: Option<String>,
    apply: bool,
    expected_eligible_count: Option<usize>,
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
            "--apply" => {
                args.apply = true;
            }
            "--expected-eligible-count" | "--expected-count" => {
                let value = raw.next().ok_or_else(|| {
                    anyhow::anyhow!(
                        "missing value for {arg}; expected --expected-eligible-count <n>"
                    )
                })?;
                args.expected_eligible_count = Some(
                    value
                        .parse()
                        .context("--expected-eligible-count must be an integer")?,
                );
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
                    "unsupported argument: {other}; run with --help for backfill options"
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
    println!("document_versions_backfill (controlled, issue 139 phase 4)");
    println!();
    println!("Default is a read-only file_library preflight (no writes, no migrations).");
    println!("Apply mode requires --apply plus explicit scope guards and never");
    println!("falls back to .env/DATABASE_URL for writes.");
    println!();
    println!("Usage (preview, read-only):");
    println!("  document_versions_backfill [--database-url <postgres-url>]");
    println!("    [--page-size <1-1000, default 200>]");
    println!("    [--max-documents <1-100000, default 5000>]");
    println!("    [--sample-size <0-100, default 20>]");
    println!();
    println!("Usage (apply, controlled writes):");
    println!("  document_versions_backfill --apply --database-url <postgres-url>");
    println!("    --expected-eligible-count <n> [--page-size <n>] [--max-documents <n>]");
    println!("    [--sample-size <n>]");
    println!();
    println!("Preview database URL resolution (read-only):");
    println!("  1. --database-url");
    println!("  2. CONTEXT69_APP_DB__URL");
    println!("  3. DATABASE_URL (repository .env is loaded if present)");
    println!();
    println!("Apply database URL resolution (writes):");
    println!("  1. --database-url (required; no .env/environment fallback)");
    println!("  Approved initial scope: --expected-eligible-count 482.");
    println!();
    println!("Output: JSON summaries with counts and document-id lists only.");
    println!("Document bodies and chunk texts are never printed.");
}
