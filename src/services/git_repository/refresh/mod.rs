//! Independent Git refresh admission, convergence, and retention (issue #28
//! phase P4).
//!
//! Manual sync, the periodic scheduler, and authenticated webhook deliveries
//! all converge through the same path: a durable demand marker is recorded on
//! the source, and admission is single-flight per repository. A request that
//! arrives while a task is already past its target resolution therefore leaves
//! a marker the next run observes instead of being dropped.
//!
//! Retention and deletion remove remote code vectors before their metadata: a
//! generation whose vector cleanup fails stays with `cleanup_pending`, and a
//! source deletion marks itself disabled and retryable instead of losing the
//! only record of the points still to remove.

use std::time::Duration;

mod cleanup;
mod scheduler;

#[cfg(test)]
pub(crate) use cleanup::{
    cleanup_deleted_source, prune_generations, remove_generation_vectors,
    run_pending_generation_cleanup,
};
pub(crate) use cleanup::{complete_refresh, delete_repository};
pub(crate) use scheduler::{request_and_enqueue_refresh, run_git_refresh_scheduler};

/// How often the independent Git refresh scheduler looks for due work.
pub(crate) const GIT_REFRESH_TICK: Duration = Duration::from_secs(30);

/// How many sources one scheduler pass admits.
pub(super) const GIT_REFRESH_BATCH: i64 = 16;

#[cfg(test)]
mod tests;
