//! Fixture isolation for the durable execution admission policy.
//!
//! A runtime-settings save now applies the global and per-resource queue
//! policies in the same transaction that writes the settings. A test that saves
//! runtime settings therefore changes shared queue state, so every such fixture
//! captures the policy that existed before it starts and restores that exact
//! baseline afterwards, even when the case panics.
//!
//! The helper reads and writes only through the public `cloudiful_tasks` queue
//! API. Restore runs in one caller transaction, restores the captured global
//! policy and every captured resource pair, and clears only resource keys that
//! did not exist in the baseline.

use cloudiful_tasks::{Queue, QueuePolicy, Result};
use sqlx::{PgPool, Postgres, Transaction};

/// The durable queue policy of one owned execution queue at fixture start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPolicyBaseline {
    global: QueuePolicy,
    resources: Vec<(String, u32)>,
}

impl LibraryPolicyBaseline {
    /// Captures the current global and resource policies of `queue`.
    pub async fn capture(queue: &Queue) -> Result<Self> {
        Ok(Self {
            global: queue.policy().await?,
            resources: queue.resource_policy().await?,
        })
    }

    /// Restores the captured policy in one caller transaction.
    ///
    /// The captured global policy and every captured resource pair are written
    /// with the library's transaction-aware setters. A resource key that is
    /// currently present but absent from the baseline is cleared, so no fixture
    /// can leak a resource cap into the next one. Keys of any other queue are
    /// never touched. Restoring the same baseline repeatedly is idempotent.
    pub async fn restore(&self, pool: &PgPool, queue: &Queue) -> Result<()> {
        let current = queue.resource_policy().await?;
        let extras: Vec<String> = current
            .into_iter()
            .map(|(resource, _)| resource)
            .filter(|resource| {
                !self
                    .resources
                    .iter()
                    .any(|(baseline, _)| baseline == resource)
            })
            .collect();

        let mut transaction: Transaction<'_, Postgres> = pool.begin().await?;
        let outcome = self.restore_in(&mut transaction, queue, &extras).await;
        match outcome {
            Ok(()) => {
                transaction.commit().await?;
                Ok(())
            }
            Err(error) => {
                transaction.rollback().await.ok();
                Err(error)
            }
        }
    }

    async fn restore_in(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        queue: &Queue,
        extras: &[String],
    ) -> Result<()> {
        queue.set_policy_in(transaction, self.global).await?;
        for (resource, max) in &self.resources {
            queue
                .set_resource_policy_in(transaction, resource, Some(*max))
                .await?;
        }
        for resource in extras {
            queue
                .set_resource_policy_in(transaction, resource, None)
                .await?;
        }
        Ok(())
    }
}

/// Renders a caught panic payload for a cleanup failure report.
pub fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Finishes a fixture after its body has been caught.
///
/// The original panic is resumed after the cleanup and policy-restore failures
/// are reported. Without an original panic, any cleanup or restore failure is
/// the test failure. This keeps a cleanup failure from masking or replacing the
/// body's panic with a second panic.
pub fn finish_fixture(
    outcome: std::thread::Result<()>,
    cleanup_error: Option<String>,
    restore: Result<()>,
) {
    let mut failures = Vec::new();
    if let Some(error) = cleanup_error {
        failures.push(format!("fixture cleanup failed: {error}"));
    }
    if let Err(error) = restore {
        failures.push(format!("library policy restore failed: {error}"));
    }
    match outcome {
        Ok(()) => {
            if !failures.is_empty() {
                panic!("{}", failures.join("; "));
            }
        }
        Err(payload) => {
            for failure in failures {
                eprintln!("{failure}");
            }
            std::panic::resume_unwind(payload);
        }
    }
}
