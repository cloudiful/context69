use std::env;

use anyhow::{Result, anyhow};
use config::{ReadOptions, read};
use context69::db::Database;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize)]
struct DbInitConfig {
    app_db: Option<DbInitAppDbConfig>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct DbInitAppDbConfig {
    url: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("fatal: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let database_url = load_database_url()?;
    let _db = Database::connect(&database_url).await?;
    println!("database init completed");
    Ok(())
}

fn load_database_url() -> Result<String> {
    if let Ok(database_url) = env::var("DATABASE_URL") {
        let trimmed = database_url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let config: DbInitConfig = read(
        "context69",
        Some(ReadOptions::with_env_prefix("CONTEXT69_")),
    )
    .map_err(|error| anyhow!(error).context("failed to load application config"))?;

    config
        .app_db
        .and_then(|app_db| app_db.url)
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "missing DATABASE_URL or CONTEXT69_APP_DB__URL (or `app_db.url` in config file)"
            )
        })
}
