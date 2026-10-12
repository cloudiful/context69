//! The termination policy for one test child's whole process tree.
//!
//! Releasing the advisory lock is only safe once nothing can still touch the
//! designated database. A test child receives the resolved URL in its
//! environment, so every process it spawns inherits that access; a descendant
//! that survived would keep mutating the same rows after the lock moved on.
//!
//! Termination is therefore a property of the whole tree, reached two ways:
//!
//! * every child is started as its own process-group leader, so a signal to the
//!   group reaches processes that stayed in it; and
//! * the runner asks the kernel to reparent orphans to itself
//!   ([`crate::runner::process_signal::enable_child_subreaper`]), so a descendant
//!   that left the group with `setsid` is still a child the runner can wait for.
//!
//! This module holds the decision half only, so it stays free of platform calls
//! and is what the unit tests exercise. The signalling itself lives in
//! [`super::process_signal`].

use std::time::Duration;

/// How long a tree is given to stop after `SIGTERM` before `SIGKILL`.
pub const TERM_GRACE: Duration = Duration::from_secs(5);

/// How long a signalled tree is given to disappear before the run is blocked.
pub const KILL_GRACE: Duration = Duration::from_secs(5);

/// How often the tree is polled between signals.
pub const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// What the runner should do next for one child's process tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing is left; reap and let the run continue.
    Finish,
    /// Ask the tree to stop, then wait out [`TERM_GRACE`].
    RequestStop,
    /// The stop grace expired; signal unconditionally.
    ForceStop,
    /// The force grace expired and something is still there.
    Blocked,
}

/// The next action, given whether the tree still holds a process, whether a
/// stop has already been requested, and whether the applicable grace expired.
///
/// [`Step::Blocked`] is the answer that matters for safety: it is the only state
/// in which the advisory lock must not be released.
#[must_use]
pub fn escalate(tree_holds_process: bool, stop_requested: bool, grace_expired: bool) -> Step {
    if !tree_holds_process {
        return Step::Finish;
    }
    if !stop_requested {
        return Step::RequestStop;
    }
    if grace_expired {
        Step::Blocked
    } else {
        Step::ForceStop
    }
}

/// How a tree's termination ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupOutcome {
    /// Every process that inherited the database access is gone.
    Reaped,
    /// Something outlived `SIGKILL` inside the budget, so it may still hold the
    /// resolved database URL.
    Blocked,
}

impl GroupOutcome {
    /// Whether nothing can still reach the designated database.
    #[must_use]
    pub fn is_safe(self) -> bool {
        matches!(self, Self::Reaped)
    }
}

/// What terminating one child achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Termination {
    pub tree: GroupOutcome,
    /// Orphans reparented to the runner that were still alive at the end.
    pub adopted_still_running: usize,
}

impl Termination {
    /// Whether the advisory lock may be released.
    #[must_use]
    pub fn is_safe(self) -> bool {
        self.tree.is_safe() && self.adopted_still_running == 0
    }
}

#[cfg(test)]
mod tests {
    use super::{Step, escalate};

    #[test]
    fn an_empty_tree_finishes_without_any_signal() {
        assert_eq!(escalate(false, false, false), Step::Finish);
        assert_eq!(escalate(false, true, true), Step::Finish);
    }

    #[test]
    fn a_live_tree_is_stopped_before_it_is_forced() {
        assert_eq!(escalate(true, false, false), Step::RequestStop);
        assert_eq!(escalate(true, false, true), Step::RequestStop);
        assert_eq!(escalate(true, true, false), Step::ForceStop);
    }

    #[test]
    fn only_a_surviving_process_after_both_graces_blocks() {
        assert_eq!(escalate(true, true, true), Step::Blocked);
    }
}
