use anyhow::{Result, anyhow};
use config::{ReadOptions, read};
use db_init::{DbInitOptions, init_database, load_dotenv_if_exists, resolve_database_url};
use serde::{Deserialize, Serialize};
use sqlx::migrate::Migrator;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

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
    load_dotenv_if_exists(".env")?;
    let resolution = resolve_database_url(
        None,
        &["DATABASE_URL", "CONTEXT69_APP_DB__URL"],
        load_config_database_url,
    )?;
    let _pool = init_database(
        &resolution.database_url,
        &MIGRATOR,
        DbInitOptions::default(),
    )
    .await?;
    println!("database init completed");
    Ok(())
}

fn load_config_database_url() -> Result<Option<String>> {
    let config: DbInitConfig = read(
        "context69",
        Some(ReadOptions::with_env_prefix("CONTEXT69_")),
    )
    .map_err(|error| anyhow!(error).context("failed to load application config"))?;

    Ok(config
        .app_db
        .and_then(|app_db| app_db.url)
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty()))
}
