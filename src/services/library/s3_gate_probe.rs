//! Bounded recovery for the S3 dependency gate.
//!
//! Storage failures trip the S3 gate, but no normal operation can lift it:
//! every storage call refuses to touch the backend while the gate is not
//! closed, and only a half-open probe-lease owner is allowed through. This
//! module owns that missing recovery loop. It reserves the existing half-open
//! probe lease (which preserves the `configuration:` pin and the exponential
//! backoff encoded in `reserve_probe.sql`), runs one bounded read-only backend
//! check, and records the outcome on the gate.
//!
//! A transient timeout/transport failure therefore heals without a settings
//! edit, while a genuine configuration failure stays pinned until the
//! configuration fingerprint changes.

use std::time::Duration;

use anyhow::Result;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::dependency_errors::{is_configuration_error, redact_dependency_error};
use super::dependency_storage::is_storage_gate_failure;
use super::s3_gate_cache::observe_s3_gate_transition;
use super::{
    LIBRARY_DEPENDENCY_PROBE_LEASE_TTL_SECS, LibraryDependency, LibraryService,
    log_dependency_transition,
};
use crate::library_store::LibraryStore;

/// Cadence at which the recovery loop re-checks whether an open S3 gate is due
/// for a probe. `reserve_probe.sql` owns the exponential backoff, so a tick that
/// lands before the gate is due is a cheap no-op.
const S3_PROBE_INTERVAL: Duration = Duration::from_secs(30);

/// Result of one S3 recovery probe attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum S3ProbeOutcome {
    /// No probe ran: the backend is not S3, the gate is closed, or the
    /// backoff/`configuration:` pin has not elapsed.
    NotDue,
    /// The bounded backend check succeeded and the gate is closed.
    Succeeded,
    /// The bounded backend check failed and the gate stayed open with backoff.
    Failed,
}

impl LibraryService {
    /// Reserve and run one bounded S3 recovery probe, then record the outcome.
    ///
    /// A no-op unless the active backend is S3. The probe uses the read-only
    /// object-store `check`, so it never reads or mutates a stored object.
    /// Returns `true` when a probe was attempted.
    pub async fn probe_s3_gate(&self) -> Result<bool> {
        let Some(configuration_fingerprint) = self.s3_configuration_fingerprint.as_deref() else {
            return Ok(false);
        };
        let outcome = probe_s3_gate_with(
            &self.store,
            self.storage.backend(),
            configuration_fingerprint,
            || self.storage.check(),
        )
        .await?;
        Ok(matches!(
            outcome,
            S3ProbeOutcome::Succeeded | S3ProbeOutcome::Failed
        ))
    }

    /// Run the S3 recovery probe once at startup and then on a fixed cadence
    /// until `shutdown` cancels. Returns immediately when S3 is not configured,
    /// so a local-storage process never spins a recovery task.
    pub(crate) async fn run_s3_gate_recovery(&self, shutdown: CancellationToken) {
        if self.s3_configuration_fingerprint.is_none() {
            return;
        }
        self.probe_s3_gate_once().await;
        let mut tick = tokio::time::interval(S3_PROBE_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first interval tick completes immediately; consume it so the first
        // periodic probe runs a full interval after the startup probe above.
        tick.tick().await;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tick.tick() => self.probe_s3_gate_once().await,
            }
        }
    }

    /// One recovery cycle: success and failure detail is logged by
    /// [`probe_s3_gate_with`], so this only surfaces a not-due skip and a
    /// persistence error.
    async fn probe_s3_gate_once(&self) {
        match self.probe_s3_gate().await {
            Ok(true) => {}
            Ok(false) => debug!("s3 dependency gate probe not due"),
            Err(error) => warn!(%error, "s3 dependency gate recovery probe failed"),
        }
    }
}

/// Drive one S3 recovery probe against the shared gate record.
///
/// The reserve/submit ordering mirrors the operation path: reserve the
/// half-open lease, test the backend, then persist the result with the lease
/// token so a concurrent recovery cannot steal the outcome.
async fn probe_s3_gate_with<F, Fut>(
    store: &LibraryStore,
    backend: &str,
    configuration_fingerprint: &str,
    check: F,
) -> Result<S3ProbeOutcome>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    if backend != "s3" {
        return Ok(S3ProbeOutcome::NotDue);
    }
    let probe_token = Uuid::new_v4();
    let Some(transition) = store
        .reserve_dependency_probe(
            LibraryDependency::S3.canonical_str(),
            probe_token,
            LIBRARY_DEPENDENCY_PROBE_LEASE_TTL_SECS,
        )
        .await?
    else {
        return Ok(S3ProbeOutcome::NotDue);
    };
    log_dependency_transition(&transition);
    observe_s3_gate_transition(LibraryDependency::S3, Some(&transition));

    match check().await {
        Ok(()) => {
            record_s3_probe_success(store, probe_token).await;
            info!(
                dependency = LibraryDependency::S3.canonical_str(),
                state = "closed",
                "library dependency gate probe succeeded"
            );
            Ok(S3ProbeOutcome::Succeeded)
        }
        Err(error) => {
            record_s3_probe_failure(
                store,
                backend,
                configuration_fingerprint,
                probe_token,
                &error,
            )
            .await;
            warn!(
                dependency = LibraryDependency::S3.canonical_str(),
                state = "open",
                error = %error,
                "library dependency gate probe failed"
            );
            Ok(S3ProbeOutcome::Failed)
        }
    }
}

/// Close the gate on a successful probe.
async fn record_s3_probe_success(store: &LibraryStore, probe_token: Uuid) {
    match store
        .record_dependency_success(LibraryDependency::S3.canonical_str(), probe_token)
        .await
    {
        Ok(Some(transition)) => {
            log_dependency_transition(&transition);
            observe_s3_gate_transition(LibraryDependency::S3, Some(&transition));
        }
        Ok(None) => {}
        Err(error) => warn!(
            dependency = LibraryDependency::S3.canonical_str(),
            %error,
            "failed to persist library dependency gate recovery"
        ),
    }
}

/// Record a failed probe using the same routing the operation path uses: a
/// configuration failure pins the gate with `configuration:`, a transient one
/// reopens it with exponential backoff, and anything else abandons the lease so
/// the gate returns to `open` instead of sitting half-open until the lease TTL.
async fn record_s3_probe_failure(
    store: &LibraryStore,
    backend: &str,
    configuration_fingerprint: &str,
    probe_token: Uuid,
    error: &anyhow::Error,
) {
    let error_message = redact_dependency_error(error);
    let result = if is_configuration_error(error) {
        store
            .configure_dependency_gate(
                LibraryDependency::S3.canonical_str(),
                false,
                Some(&format!("configuration: {error_message}")),
                configuration_fingerprint,
            )
            .await
    } else if is_storage_gate_failure(backend, error) {
        store
            .record_dependency_failure(
                LibraryDependency::S3.canonical_str(),
                probe_token,
                &error_message,
            )
            .await
    } else {
        store
            .abandon_dependency_probe(LibraryDependency::S3.canonical_str(), probe_token)
            .await
    };
    match result {
        Ok(Some(transition)) => {
            log_dependency_transition(&transition);
            observe_s3_gate_transition(LibraryDependency::S3, Some(&transition));
        }
        Ok(None) => {}
        Err(record_error) => warn!(
            dependency = LibraryDependency::S3.canonical_str(),
            error = %record_error,
            "failed to persist library dependency gate failure"
        ),
    }
}
