//! The application's manual maintenance modes (issue #681 secret-store work).
//!
//! The mode here runs over the application database *before* the application is
//! built, which is what keeps it out of every serving path: a maintenance run
//! opens no Valkey, no Qdrant, no API, and no MCP server, and it resolves no
//! runtime credential to do its work. `main` dispatches it from argv and owns
//! nothing else.
//!
//! It is not reached automatically. There is no scheduler job, no startup hook,
//! and no HTTP or MCP route here: an operator asks for it by name, and the
//! runbook that drives it lives in `docs/configuration.md`.

use std::env;

use anyhow::{Context, Result};
use context69_secret_store::{SecretDatabase, SecretStore};
use tracing::info;

use crate::{config::Config, db::Database, services::secret_store};

/// The master secret a `rewrap-secrets` run seals into.
///
/// Deliberately a different variable from `CONTEXT69_APP__MASTER_SECRET`: the
/// incoming key must never be able to reach a config file, and a command-line
/// argument would put it in the process table and the shell history. It is read
/// once, held in the cipher's zeroizing buffer, and never persisted or logged.
const REWRAP_TARGET_MASTER_SECRET_ENV_VAR: &str = "CONTEXT69_APP__NEXT_MASTER_SECRET";
/// The key version that master secret is registered under, and the version every
/// re-sealed row moves to. It has to be strictly above the source version, which the
/// store refuses if it is not. It stays in the store's own scope because it
/// versions ciphertext rather than naming a secret.
const REWRAP_TARGET_KEY_VERSION_ENV_VAR: &str = "CONTEXT69_SECRET_STORE__NEXT_KEY_VERSION";

/// Re-seals every sealed secret from the configured master secret to the next one.
///
/// The outgoing deployment is read from the ordinary configuration, exactly as a
/// serving process reads it, so the run always opens rows with the key that actually
/// sealed them. Only the incoming key is taken from the environment. The run stops at
/// the first row it cannot re-seal, deletes nothing, and reports bounded counts: no
/// key name, no value, no ciphertext, and no credential ever reaches stdout, the log,
/// or the exit status.
///
/// Neither deployment key is written anywhere. The runbook that drives this is in
/// `docs/configuration.md`; it owns the backup, the restore rehearsal, the
/// metadata-only verification, and the cutover, none of which this mode performs.
pub async fn rewrap_secrets() -> Result<()> {
    let config = Config::load()?;
    let target_master_secret = required_env(REWRAP_TARGET_MASTER_SECRET_ENV_VAR)?;
    let target_key_version = required_env(REWRAP_TARGET_KEY_VERSION_ENV_VAR)?
        .parse::<u32>()
        .context("rewrap target key version must be a positive integer")?;
    if target_key_version == 0 {
        return Err(anyhow::anyhow!(
            "rewrap target key version must be greater than 0"
        ));
    }

    let db = Database::connect(&config.app_db.url).await?;
    let source = secret_store::build(&db, &config)?;
    let target = SecretStore::new(
        SecretDatabase::new(db.pool().clone()),
        Some(&target_master_secret),
        target_key_version,
    )?;
    let report = target.rewrap_secrets_from(&source).await?;
    info!(
        source_key_version = config.secret_store.key_version,
        target_key_version,
        rewrapped = report.rewrapped,
        purposes = report.purposes,
        "secret rewrap finished"
    );
    Ok(())
}

/// Reads one deployment input that the rewrap cannot run without.
fn required_env(name: &str) -> Result<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} must be set to run a secret rewrap"))
}
