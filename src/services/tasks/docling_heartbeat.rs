use std::future::Future;
use std::time::Duration;

use anyhow::Result;

use crate::db::{Database, StoredDoclingRemoteJob};

/// Interval between lease renewals while a long poll is in flight. Well
/// under the sweep lease so several renewals fit into one poll worst case
/// (`?wait=30` long polls with bounded transport retries).
pub(crate) const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// Lease installed per due-row claim, renewed every heartbeat while a
/// poll/fetch is in flight. Covers one `?wait=30` long poll plus transport
/// retries; longer conversions stay owned via renewal, never via a longer
/// static lease that would delay restart recovery.
pub(crate) const SWEEP_LEASE_SECS: i32 = 90;

/// Runs `request` while renewing the remote-row lease underneath it.
///
/// Returns `Ok(None)` when the lease is lost mid-flight (revoked by a
/// concurrent timeout/cancel finalization, or the row went terminal): the
/// in-flight HTTP future is dropped — cancelling the request — and the
/// caller must write nothing, so a second sweep instance can never complete
/// a duplicate status request for the same remote task. A heartbeat that
/// still holds the lease keeps exactly one in-flight request alive.
pub(crate) async fn with_remote_lease_heartbeat<T, E>(
    db: &Database,
    job: &StoredDoclingRemoteJob,
    lease_secs: i32,
    request: impl Future<Output = std::result::Result<T, E>>,
) -> Result<Option<std::result::Result<T, E>>> {
    let Some(lease) = job.lease_token else {
        anyhow::bail!("sweep job missing lease");
    };
    let mut request = Box::pin(request);
    loop {
        tokio::select! {
            outcome = &mut request => return Ok(Some(outcome)),
            () = tokio::time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)) => {
                let renewed = db
                    .heartbeat_docling_remote_job(job.id, lease, lease_secs)
                    .await?;
                if renewed.is_none() {
                    return Ok(None);
                }
            }
        }
    }
}
