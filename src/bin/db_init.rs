use anyhow::Result;
use context69::{
    config::{APP_DB_URL_ENV_VAR, load_app_db_url},
    db::MIGRATOR,
};
use db_init::{
    DatabaseUrlSource, DbInitOptions, init_database, load_dotenv_if_exists, resolve_database_url,
};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("fatal: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli_database_url = parse_database_url_arg()?;
    load_dotenv_if_exists(".env")?;
    let resolution = resolve_database_url(
        cli_database_url,
        &[APP_DB_URL_ENV_VAR, "DATABASE_URL"],
        load_app_db_url,
    )?;
    println!(
        "initializing database using {}",
        describe_database_url_source(&resolution.source)
    );
    let _pool = init_database(
        &resolution.database_url,
        &MIGRATOR,
        DbInitOptions::default(),
    )
    .await?;
    println!("database init completed");
    Ok(())
}

fn parse_database_url_arg() -> Result<Option<String>> {
    let mut args = std::env::args().skip(1);
    let mut database_url = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--database-url" | "-d" => {
                let value = args.next().ok_or_else(|| {
                    anyhow::anyhow!(
                        "missing value for {arg}; expected --database-url <postgres-url>"
                    )
                })?;
                database_url = Some(value);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                anyhow::bail!(
                    "unsupported argument: {other}; run `cargo run --bin db_init -- --help`"
                )
            }
        }
    }

    Ok(database_url)
}

fn describe_database_url_source(source: &DatabaseUrlSource) -> String {
    match source {
        DatabaseUrlSource::CliArgument => "--database-url".to_string(),
        DatabaseUrlSource::EnvVar(name) => format!("environment variable {name}"),
        DatabaseUrlSource::ConfigValue => "application config app_db.url".to_string(),
    }
}

fn print_help() {
    println!("Usage: db_init [--database-url <postgres-url>]");
    println!();
    println!("Resolution order:");
    println!("  1. --database-url");
    println!("  2. {APP_DB_URL_ENV_VAR}");
    println!("  3. DATABASE_URL");
    println!("  4. app_db.url from config");
}
