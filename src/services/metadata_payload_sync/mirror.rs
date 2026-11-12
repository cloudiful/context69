//! Bounded mirror pass for the durable per-file metadata marker.
//!
//! Each candidate takes a per-file session advisory lock on a dedicated pooled
//! connection, rereads the marker and the current canonical payloads on that
//! connection, proves the captured runtime snapshot against the active vector
//! pointer, and only then sends one external payload update. The marker is
//! completed with an exact file/version compare, so a concurrent enqueue makes
//! the completion a no-op instead of erasing the newer publication.

use anyhow::{Context, Result};
use sqlx::PgConnection;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::LibraryMetadataMirrorMarker;
use crate::library_store::LibraryStore;

use super::{MetadataPayloadSyncWorker, retry_delay, sanitize_failure_reason};

/// Bounded mirror candidates of one pass, so a backlog cannot monopolize the
/// shared worker.
pub const MAX_MIRROR_CANDIDATES_PER_PASS: usize = 16;

/// The decision of one locked mirror attempt.
enum MirrorOutcome {
    /// The marker disappeared between the due read and the session lock.
    Gone,
    /// The exact observed version was completed.
    Complete(LibraryMetadataMirrorMarker),
    /// The attempt could not publish; only the observed version is delayed.
    Backoff {
        marker: LibraryMetadataMirrorMarker,
        reason: &'static str,
    },
    /// A newer enqueue owns the marker; leave it pending.
    NewerPending,
}

impl MetadataPayloadSyncWorker {
    /// Drain up to [`MAX_MIRROR_CANDIDATES_PER_PASS`] due markers sequentially.
    ///
    /// At most one candidate connection and one external payload call are held
    /// at any time. A candidate failure never stops the remaining candidates.
    pub(super) async fn drain_mirror_pass(&self, shutdown: &CancellationToken) -> Result<()> {
        let candidates = self
            .db
            .read_due_library_metadata_mirrors(MAX_MIRROR_CANDIDATES_PER_PASS as i64)
            .await
            .context("read due library metadata mirrors")?;
        for candidate in candidates {
            if shutdown.is_cancelled() {
                break;
            }
            if let Err(error) = self.mirror_candidate(candidate.file_id).await {
                warn!(
                    file_id = %candidate.file_id,
                    error = %sanitize_failure_reason(&error),
                    "library metadata mirror candidate failed"
                );
            }
        }
        Ok(())
    }

    async fn mirror_candidate(&self, file_id: Uuid) -> Result<()> {
        let Some(mut lock) = self.db.try_lock_library_metadata_mirror(file_id).await? else {
            // Another worker or instance owns this file right now.
            return Ok(());
        };
        let outcome = self.mirror_locked(lock.connection(), file_id).await;
        match outcome {
            Ok(MirrorOutcome::Gone) => {
                lock.release().await;
            }
            Ok(MirrorOutcome::Complete(marker)) => {
                info!(
                    file_id = %file_id,
                    version = marker.version,
                    "library metadata mirror completed"
                );
                lock.release().await;
            }
            Ok(MirrorOutcome::NewerPending) => {
                lock.release().await;
            }
            Ok(MirrorOutcome::Backoff { marker, reason }) => {
                let attempts = u32::try_from(marker.attempts.max(0))
                    .unwrap_or(u32::MAX)
                    .saturating_add(1);
                let delay = retry_delay(attempts);
                match self
                    .db
                    .backoff_library_metadata_mirror_in(
                        lock.connection(),
                        file_id,
                        marker.version,
                        delay.as_secs() as i32,
                    )
                    .await
                {
                    Ok(true) => warn!(
                        file_id = %file_id,
                        version = marker.version,
                        attempts,
                        retry_after_seconds = delay.as_secs(),
                        reason,
                        "library metadata mirror deferred; retrying"
                    ),
                    Ok(false) => info!(
                        file_id = %file_id,
                        version = marker.version,
                        "a newer metadata mirror enqueue owns the marker; leaving it pending"
                    ),
                    Err(error) => {
                        warn!(
                            file_id = %file_id,
                            error = %sanitize_failure_reason(&error),
                            "failed to record the library metadata mirror backoff; the session ends"
                        );
                        // The backoff outcome is unknown, so end the session
                        // instead of returning a connection that may still hold
                        // the per-file lock.
                        drop(lock);
                        return Ok(());
                    }
                }
                lock.release().await;
            }
            Err(error) => {
                // The database state is unknown: end the session so the
                // advisory lock is never returned to the pool held.
                drop(lock);
                return Err(error);
            }
        }
        Ok(())
    }

    async fn mirror_locked(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
    ) -> Result<MirrorOutcome> {
        let Some(marker) = self
            .db
            .read_library_metadata_mirror_in(connection, file_id)
            .await?
        else {
            return Ok(MirrorOutcome::Gone);
        };
        let Some(ready) = super::ready_pair(&self.vector) else {
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "vector runtime is not ready",
            });
        };
        if self.db.read_search_registry_unavailable().await? {
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "the derived original index is unavailable",
            });
        }
        let Some(snapshot) = ready.snapshot.as_ref() else {
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "vector runtime has no published pair",
            });
        };
        let Some(fingerprint) = snapshot.configured_fingerprint.as_deref() else {
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "the published vector pair has no identity fingerprint",
            });
        };
        let Some(active) = self
            .db
            .read_active_vector_index_state_in(connection)
            .await?
        else {
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "the deployment has no active vector index",
            });
        };
        if active.collection_name != snapshot.collection || active.fingerprint != fingerprint {
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "the published vector pair does not match the active vector index",
            });
        }

        let payloads = LibraryStore::new(self.db.clone())
            .list_chunk_payloads_for_files_on(connection, &[file_id])
            .await?;
        if !payloads.is_empty()
            && let Err(error) = self
                .run_external(
                    snapshot.index.update_chunk_payloads(&payloads),
                    "update metadata payloads",
                )
                .await
        {
            warn!(
                file_id = %file_id,
                version = marker.version,
                error = %sanitize_failure_reason(&error),
                "library metadata mirror payload send failed"
            );
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "the external payload send failed",
            });
        }

        if !super::ready_pair(&self.vector)
            .is_some_and(|current| current.generation == ready.generation)
        {
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "the vector runtime generation changed during the payload send",
            });
        }
        if self.db.read_search_registry_unavailable().await? {
            return Ok(MirrorOutcome::Backoff {
                marker,
                reason: "the derived original index became unavailable during the payload send",
            });
        }

        if !self
            .db
            .complete_library_metadata_mirror_in(connection, file_id, marker.version)
            .await?
        {
            return Ok(MirrorOutcome::NewerPending);
        }
        Ok(MirrorOutcome::Complete(marker))
    }
}
