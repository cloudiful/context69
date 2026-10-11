//! The startup sealing step, in the application.
//!
//! The conversion itself belongs to the secret store; what lives here is the
//! decision this application makes about it. Sealing runs once, immediately
//! after the store is built and before anything resolves a credential, because
//! the order is the point: a deployment that has just configured a master key
//! must have its stored secrets encrypted *before* the first read, not after
//! the first successful use.
//!
//! A refusal stops startup. A deployment holding a secret whose key name this
//! build does not own has a credential that cannot be sealed under any purpose
//! and therefore cannot be protected by the configured key either; continuing
//! would boot a process that reads plaintext out of the table and reports a
//! healthy one. The error is bubbled as the store's own category, which carries
//! no key name, no value, and no database or cryptographic cause.

use anyhow::Result;
use tracing::info;

use crate::services::secret_store::SecretStore;

/// Seals every stored secret this deployment still holds in the legacy plaintext
/// representation.
///
/// Does nothing at all when no master key is configured: the store reports zero
/// work without touching the database, and the legacy read path is unchanged.
///
/// # Errors
///
/// The store's refusal category, when a stored secret cannot be sealed. The
/// message names what kind of row stopped the run and nothing else; the
/// committed work is what the operator's next startup resumes from.
pub(crate) async fn seal_legacy_plaintext(secrets: &SecretStore) -> Result<()> {
    let report = secrets
        .seal_legacy_plaintext()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    if report.sealed > 0 {
        info!(
            secrets = report.sealed,
            batches = report.batches,
            "sealed stored secrets that were still stored in the plaintext representation"
        );
    }
    Ok(())
}
