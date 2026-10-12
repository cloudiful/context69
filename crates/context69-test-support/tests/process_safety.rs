//! Process-safety invariants for the runner's child handling.
//!
//! Nothing here opens a database. The cases cover the escalation decision, the
//! two subprocess shapes that decide whether a leaked process could still hold
//! the resolved database URL, and the ordinary exit path whose exit code the
//! runner reports.

use std::process::{Command, Stdio};
use std::time::Duration;

use context69_test_support::runner::process;
use context69_test_support::runner::process_group::{
    GroupOutcome, KILL_GRACE, POLL_INTERVAL, Step, TERM_GRACE, Termination, escalate,
};
use context69_test_support::runner::process_signal;

#[test]
fn the_escalation_asks_before_it_forces_and_blocks_before_it_gives_up() {
    // Nothing in the tree: finish without spending a signal.
    assert_eq!(escalate(false, false, false), Step::Finish);
    // Something alive and nothing sent yet: ask it to stop.
    assert_eq!(escalate(true, false, false), Step::RequestStop);
    // Asked, still alive, grace left: escalate.
    assert_eq!(escalate(true, true, false), Step::ForceStop);
    // Asked, escalated, grace spent: block rather than release the lock.
    assert_eq!(escalate(true, true, true), Step::Blocked);
}

#[test]
fn an_unreaped_orphan_alone_makes_the_tree_unsafe() {
    let group_reaped_but_orphan_running = Termination {
        tree: GroupOutcome::Reaped,
        adopted_still_running: 2,
    };
    assert!(
        !group_reaped_but_orphan_running.is_safe(),
        "a reparented process that is still running can still hold the database URL"
    );

    let fully_gone = Termination {
        tree: GroupOutcome::Reaped,
        adopted_still_running: 0,
    };
    assert!(fully_gone.is_safe());
}

#[test]
fn a_blocked_tree_is_never_safe_even_with_no_orphans() {
    let blocked = Termination {
        tree: GroupOutcome::Blocked,
        adopted_still_running: 0,
    };
    assert!(
        !blocked.is_safe(),
        "a process that outlived SIGKILL is the case the lock release depends on"
    );
}

#[test]
fn the_grace_budgets_are_long_enough_to_be_meaningful() {
    assert!(
        TERM_GRACE >= Duration::from_secs(1),
        "a stop request needs at least a second to be honoured"
    );
    assert!(
        KILL_GRACE >= TERM_GRACE,
        "the forced stop must be given at least as long as the polite one"
    );
}

#[test]
fn a_child_that_exits_on_its_own_reports_its_code_and_is_quiescent() {
    let outcome = run_sh("exit 0", Duration::from_secs(30));
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "a target's exit code must survive the runner's own reaping"
    );
    assert!(outcome.succeeded());
    assert!(
        outcome.is_quiescent(),
        "a child that finished on its own terminated nothing, so nothing can be left"
    );
    assert!(outcome.termination.is_none());
}

#[test]
fn a_child_that_fails_reports_its_code() {
    let outcome = run_sh("exit 3", Duration::from_secs(30));
    assert_eq!(outcome.exit_code, Some(3));
    assert!(!outcome.succeeded());
    assert!(outcome.is_quiescent());
}

#[cfg(unix)]
#[test]
fn a_timeout_terminates_the_descendant_with_the_direct_child() {
    // The shell backgrounds a long sleep and then waits forever. A runner that
    // only killed the direct child would leave the sleep holding the group, and
    // the sleep would still carry the resolved database URL.
    let outcome = run_sh("sleep 120 & wait", Duration::from_millis(300));
    assert!(outcome.timed_out, "the probe outlives its budget");
    assert!(
        !outcome.succeeded(),
        "a terminated child is never a successful target"
    );

    let termination = outcome.termination.expect("a timeout terminates something");
    assert_eq!(
        termination.tree,
        GroupOutcome::Reaped,
        "the backgrounded descendant must be signalled with the shell"
    );
    assert_eq!(termination.adopted_still_running, 0);
    assert!(
        outcome.is_quiescent(),
        "the advisory lock is only releasable once nothing inherits the URL"
    );
}

#[cfg(unix)]
#[test]
fn a_timeout_reaches_a_descendant_that_left_the_process_group() {
    // `setsid` puts the sleep in its own session, so it leaves the shell's
    // process group and a group signal alone cannot reach it. Only the
    // subreaper keeps it waitable.
    let outcome = run_sh(
        "setsid sleep 120 </dev/null >/dev/null 2>&1 & sleep 120",
        Duration::from_millis(300),
    );
    assert!(outcome.timed_out);

    let termination = outcome.termination.expect("a timeout terminates something");
    assert_eq!(
        termination.tree,
        GroupOutcome::Reaped,
        "a descendant that left the group must still be reached"
    );
    assert_eq!(
        termination.adopted_still_running, 0,
        "no reparented process may remain with the inherited database URL"
    );
    assert!(outcome.is_quiescent());
}

#[test]
fn the_subreaper_is_available_or_reported() {
    // The runner cannot make the guarantee without it, so a refusal is surfaced
    // as an error rather than silently leaving orphans untraceable.
    if let Err(error) = process_signal::enable_child_subreaper() {
        panic!("the runner needs orphan adoption to release the lock safely: {error}");
    }
}

#[test]
fn force_stop_budgets_are_bounded_before_the_runner_waits_fail_closed() {
    // One escalation pass spends at most the stop grace, then the force grace,
    // and the detached pass repeats that. A survivor is reported as blocked
    // within a predictable window before the runner retains its lock and waits.
    let worst_case = (TERM_GRACE + KILL_GRACE) * 2;
    assert!(
        worst_case <= Duration::from_secs(30),
        "a blocked report must arrive well inside the runner's own budget"
    );
    assert!(
        POLL_INTERVAL < TERM_GRACE,
        "polling must be finer than the grace it is waiting out"
    );
}

/// Start `script` in its own process group with the runner's pipe handling, and
/// let [`process::run_logged`] apply the budget.
fn run_sh(script: &str, timeout: Duration) -> process::ChildOutcome {
    process_signal::enable_child_subreaper().expect("orphan adoption is available");
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::run_logged(&mut command, timeout).expect("the probe shell runs")
}
