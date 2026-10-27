//! Standalone entry for the audited task execution cutover tool.
//!
//! The tool is read-only by default (`inventory`); the two handover commands
//! require `--workers-stopped`, either an explicit item list or the bounded
//! `adopt --all`, and a live-lease check.
//! It connects without running migrations and without upgrade admission, so it
//! never performs DDL. The database is selected only through `DATABASE_URL`:
//! there is no `.env` fallback and no URL argument, and both the configured and
//! the server-reported name must equal the mandatory `--expect-database`.
//!
//! Exit status: `0` success, `2` usage or target mismatch, `3` database or
//! safety refusal.

use context69::config::DATABASE_URL_ENV_VAR;
use context69::db::Database;
use context69::services::tasks::cutover::{
    CutoverCliError, CutoverCommand, Sanitized, configured_database_name, parse_cutover_args,
    run_cutover,
};

#[tokio::main]
async fn main() {
    std::process::exit(run().await);
}

async fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse_cutover_args(&args) {
        Ok(cli) => cli,
        Err(error) => return refuse(&error),
    };

    let Ok(database_url) = std::env::var(DATABASE_URL_ENV_VAR) else {
        eprintln!("DATABASE_URL is required and is read only from the environment");
        return 2;
    };
    let configured = match configured_database_name(&database_url) {
        Ok(name) => name,
        Err(error) => return refuse(&error),
    };
    if configured != cli.expect_database {
        eprintln!(
            "configured database `{configured}` is not the expected `{}`",
            cli.expect_database
        );
        return 2;
    }

    // The operator connection runs no migration and admits no pending upgrade;
    // the read-only variant adds PostgreSQL's own write refusal for inventory.
    let read_only = matches!(cli.command, CutoverCommand::Inventory { .. });
    let database = match Database::connect_operator(&database_url, read_only).await {
        Ok(database) => database,
        Err(error) => {
            eprintln!(
                "could not connect to `{}`: {}",
                cli.expect_database,
                Sanitized(&error.to_string())
            );
            return 3;
        }
    };

    match run_cutover(&database, &cli).await {
        Ok(outcome) => {
            let metadata = serde_json::json!({
                "status": "ok",
                "command": outcome.command,
                "affected": outcome.affected,
                "result": outcome.metadata,
            });
            match serde_json::to_string(&metadata) {
                Ok(rendered) => println!("{rendered}"),
                Err(error) => {
                    eprintln!("could not render the result: {error}");
                    return 3;
                }
            }
            0
        }
        Err(error) => refuse(&error),
    }
}

fn refuse(error: &CutoverCliError) -> i32 {
    eprintln!("{}", error.message);
    error.exit_code()
}
