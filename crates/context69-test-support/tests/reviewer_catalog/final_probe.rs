use context69_test_support::config::resolve_default;
use context69_test_support::runner::{lock::SessionLock, preflight};
use sqlx::{Connection, postgres::PgConnection};
use std::time::Duration;

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("audit runtime");
    if runtime.block_on(probe()).is_err() {
        eprintln!("bounded lock/cleanup audit failed; private diagnostic withheld");
        std::process::exit(1);
    }
}

async fn probe() -> Result<(), Box<dyn std::error::Error>> {
    let target = resolve_default()?;
    let endpoint = target.endpoint();
    let observation = preflight::collect(endpoint).await?;
    if !preflight::decide_reset(&observation, endpoint).allowed {
        return Err("preflight refused probe".into());
    }
    let first = SessionLock::try_acquire(endpoint).await?.ok_or("another runner active")?;
    if SessionLock::try_acquire(endpoint).await?.is_some() {
        return Err("competing nonblocking lock admitted".into());
    }
    println!("lock:nonblocking_contender_refused=true");

    let waiting = SessionLock::acquire(endpoint);
    tokio::pin!(waiting);
    if tokio::time::timeout(Duration::from_millis(300), &mut waiting).await.is_ok() {
        return Err("blocking contender admitted while lock held".into());
    }
    println!("lock:blocking_contender_waited=true");
    first.release().await?;
    let second = tokio::time::timeout(Duration::from_secs(5), &mut waiting).await??;
    println!("lock:contender_admitted_after_release=true");
    second.release().await?;
    let after = SessionLock::try_acquire(endpoint).await?.ok_or("lock not released")?;

    let options = endpoint.expose().parse::<sqlx::postgres::PgConnectOptions>()?
        .application_name(preflight::RUNNER_APPLICATION_NAME)
        .options([("default_transaction_read_only", "on")]);
    let mut connection = PgConnection::connect_with(&options).await?;
    let (identity_matches, read_only): (bool, bool) = sqlx::query_as(
        "SELECT current_database() = $1, current_setting('default_transaction_read_only') = 'on'",
    )
    .bind(endpoint.database())
    .fetch_one(&mut connection)
    .await?;
    if !identity_matches || !read_only {
        return Err("read-only identity validation failed".into());
    }
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT c.relname::text FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'context69' AND c.relkind = 'r' ORDER BY c.relname",
    )
    .fetch_all(&mut connection)
    .await?;
    println!("cleanup:owned_tables_checked={}", tables.len());
    let mut nonempty = 0;
    for (table,) in tables {
        let quoted = table.replace('"', "\"\"");
        let statement = format!("SELECT count(*)::bigint FROM context69.\"{quoted}\"");
        let (count,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(&*statement))
            .fetch_one(&mut connection)
            .await?;
        if count > 0 {
            nonempty += 1;
            println!("cleanup:nonempty_table={table} rows={count}");
        }
    }
    println!("cleanup:nonempty_tables={nonempty} identity_matches=true read_only=true");
    let (deviations,): (i64,) = sqlx::query_as(
        "SELECT \
         (SELECT count(*) FROM context69.groups WHERE group_key <> 'public' OR name <> 'Public' OR visibility <> 'public' OR kind <> 'shared') + \
         (SELECT count(*) FROM context69.search_generations WHERE scope <> 'global' OR generation <> 0) + \
         (SELECT count(*) FROM context69.library_dependency_gates WHERE dependency_key NOT IN ('s3','docling','embedding_vector') OR state <> 'closed' OR failure_count <> 0 OR last_error IS NOT NULL OR probe_lease_token IS NOT NULL) + \
         (SELECT count(*) FROM context69.translation_provider_settings WHERE enabled OR api_key IS NOT NULL OR model IS NOT NULL OR llm_api_kind IS NOT NULL OR \
          ((provider_key = 'deepl' AND priority = 10 AND endpoint = 'https://api-free.deepl.com' AND deepl_plan = 'free' AND monthly_character_limit = 1000000) OR \
               (provider_key = 'llm' AND priority = 20 AND endpoint IS NULL AND deepl_plan IS NULL AND monthly_character_limit IS NULL) OR \
               (provider_key = 'libretranslate' AND priority = 30 AND endpoint IS NULL AND deepl_plan IS NULL AND monthly_character_limit IS NULL)) IS NOT TRUE)",
    )
    .fetch_one(&mut connection)
    .await?;
    if nonempty != 4 || deviations != 0 {
        return Err("remaining rows differ from migrated seed baseline".into());
    }
    let migrations = preflight::repository_migrations();
    let ledger_complete = observation.public_ledger_rows.len() == migrations.len()
        && observation.public_ledger_rows.iter().all(|row| {
            row.success && migrations.iter().any(|(version, checksum)|
                row.version == *version && row.checksum == *checksum)
        });
    if !ledger_complete {
        return Err("migration ledger is incomplete".into());
    }
    println!("cleanup:seed_deviations={deviations} ledger_complete=true migrations={}", migrations.len());
    connection.close().await?;
    after.release().await?;
    println!("lock:probe_released=true");
    Ok(())
}
