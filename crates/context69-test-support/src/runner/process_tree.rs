//! Terminating one test child's entire process tree.
//!
//! Test processes inherit the resolved database URL. The runner is a Linux
//! subreaper and starts each command in a private process group, so timeout,
//! interruption, and unexpected surviving descendants can all be reached and
//! reaped before the session advisory lock is released.

use std::io;
use std::time::Instant;

use super::process_group::{
    GroupOutcome, KILL_GRACE, POLL_INTERVAL, Step, TERM_GRACE, Termination, escalate,
};
use super::process_signal::{
    ChildState, adopted_children, group_holds_process, poll_child, reap_adopted, signal_group,
    signal_processes,
};

/// The set of processes one escalation pass has to deal with.
trait Survivors {
    /// Whether any of them is still alive.
    fn any_alive(&self) -> io::Result<bool>;

    /// Ask all of them to stop.
    fn signal_all(&self, signal: i32) -> io::Result<()>;
}

/// The child's process group, including descendants that stayed in it.
struct Group {
    group: i32,
}

impl Survivors for Group {
    fn any_alive(&self) -> io::Result<bool> {
        group_holds_process(self.group)
    }

    fn signal_all(&self, signal: i32) -> io::Result<()> {
        signal_group(self.group, signal)
    }
}

/// Descendants reparented to this runner after leaving the command's group.
struct Adopted;

impl Survivors for Adopted {
    fn any_alive(&self) -> io::Result<bool> {
        Ok(!adopted_children()?.is_empty())
    }

    fn signal_all(&self, signal: i32) -> io::Result<()> {
        signal_processes(&adopted_children()?, signal)
    }
}

/// Drive the termination policy against one set of survivors until it empties
/// or the bounded force-stop window expires.
fn escalate_tree(
    survivors: &impl Survivors,
    leader: Option<i32>,
) -> Result<GroupOutcome, io::Error> {
    let mut stop_requested = false;
    let mut forced = false;
    let mut stop_deadline = Instant::now();
    let mut kill_deadline = stop_deadline;

    loop {
        let leader_alive = if let Some(pid) = leader {
            matches!(poll_child(pid)?, ChildState::Running)
        } else {
            false
        };
        reap_adopted(leader)?;
        let now = Instant::now();
        let grace_expired = if forced {
            now >= kill_deadline
        } else {
            now >= stop_deadline
        };
        match escalate(
            leader_alive || survivors.any_alive()?,
            stop_requested,
            grace_expired,
        ) {
            Step::Finish => return Ok(GroupOutcome::Reaped),
            Step::RequestStop => {
                survivors.signal_all(libc::SIGTERM)?;
                stop_requested = true;
                stop_deadline = now + TERM_GRACE;
                kill_deadline = stop_deadline + KILL_GRACE;
            }
            Step::ForceStop => {
                if !forced {
                    survivors.signal_all(libc::SIGKILL)?;
                    forced = true;
                    kill_deadline = now + KILL_GRACE;
                }
            }
            Step::Blocked => return Ok(GroupOutcome::Blocked),
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Terminate a target after timeout or cancellation.
///
/// The returned `Termination` may be blocked after the bounded grace periods.
/// Callers must keep the database lock and invoke [`wait_until_quiescent`] in
/// that case; returning while a child can still use the database is forbidden.
pub fn terminate_child(pid: i32, group: i32) -> Result<Termination, io::Error> {
    let grouped = escalate_tree(&Group { group }, Some(pid))?;
    let detached = escalate_tree(&Adopted, None)?;
    Ok(combine(grouped, detached, adopted_children()?.len()))
}

/// Terminate descendants left behind by a command that exited successfully.
///
/// Returns `None` when the process group and adopted-child set are already
/// empty. A returned termination means the command leaked a descendant and the
/// caller should report the target as failed even if its exit code was zero.
pub fn terminate_descendants(group: i32) -> Result<Option<Termination>, io::Error> {
    if !group_holds_process(group)? && adopted_children()?.is_empty() {
        return Ok(None);
    }
    let grouped = escalate_tree(&Group { group }, None)?;
    let detached = escalate_tree(&Adopted, None)?;
    Ok(Some(combine(grouped, detached, adopted_children()?.len())))
}

fn combine(
    grouped: GroupOutcome,
    detached: GroupOutcome,
    adopted_still_running: usize,
) -> Termination {
    Termination {
        tree: match (grouped, detached) {
            (GroupOutcome::Blocked, _) | (_, GroupOutcome::Blocked) => GroupOutcome::Blocked,
            _ => GroupOutcome::Reaped,
        },
        adopted_still_running,
    }
}

/// Keep the runner alive — and therefore keep its advisory lock alive — until
/// the target tree can be proven empty. This is the fail-closed path for a
/// process that did not disappear during the bounded SIGTERM/SIGKILL windows.
pub fn wait_until_quiescent(leader: Option<i32>, group: i32) {
    eprintln!("a test process survived SIGKILL; retaining the database lock until it exits");
    loop {
        let _ = signal_group(group, libc::SIGKILL);
        let children = adopted_children().unwrap_or_default();
        let _ = signal_processes(&children, libc::SIGKILL);
        if let Some(pid) = leader {
            let _ = poll_child(pid);
        }
        let _ = reap_adopted(leader);

        let leader_alive =
            leader.is_some_and(|pid| matches!(poll_child(pid), Ok(ChildState::Running) | Err(_)));
        let group_alive = group_holds_process(group).unwrap_or(true);
        let adopted_alive = match adopted_children() {
            Ok(children) => !children.is_empty(),
            Err(_) => true,
        };
        if !leader_alive && !group_alive && !adopted_alive {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}
