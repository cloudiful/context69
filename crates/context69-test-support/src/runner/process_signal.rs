//! The Unix primitives behind child-tree termination.
//!
//! One wrapper per kernel operation the runner needs — wait for a child, list
//! what is left in a process group, list what the kernel reparented here, send a
//! signal, ask for orphan adoption. No policy lives here: the decisions are in
//! [`super::process_group`] and the sequence that applies them is in
//! [`super::process_tree`], so each can be read on its own.
//!
//! The runner requires Linux subreaper and child-enumeration support; unsupported
//! platforms refuse before starting any child instead of claiming an empty tree.

use std::io;
use std::process::Child;

/// Whether a process this runner started has exited, reaping it if so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildState {
    /// Still running.
    Running,
    /// Exited; the code is `None` when it was killed by a signal.
    Exited(Option<i32>),
}

/// The process-group id of a child, which equals the child's own id because
/// [`isolate_process_group`](super::process::run_logged) makes it a leader.
///
/// # Errors
///
/// Returns the platform error when the child's id does not fit an `i32`.
pub fn group_of(child: &Child) -> Result<i32, io::Error> {
    child.id().try_into().map_err(io::Error::other)
}

/// Check — and reap — one direct child without disturbing any other child.
///
/// Adopted descendants are reaped by explicit pid, while this function owns the
/// active command leader's status. That keeps cleanup from consuming the status
/// before the runner has recorded the target's exit code.
///
/// # Errors
///
/// Returns the platform error when the wait itself fails.
pub fn poll_child(pid: i32) -> Result<ChildState, io::Error> {
    let mut status: libc::c_int = 0;
    // SAFETY: `waitpid` writes the provided status word and `WNOHANG` never
    // blocks, so no runner memory beyond `status` is touched.
    let result = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
    if result == 0 {
        return Ok(ChildState::Running);
    }
    if result > 0 {
        return Ok(ChildState::Exited(exit_code_of(status)));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ECHILD) {
        // Already reaped — by [`reap_adopted`] or a previous poll. From the
        // runner's point of view the child is gone.
        Ok(ChildState::Exited(None))
    } else {
        Err(error)
    }
}

/// The exit code carried by a `waitpid` status word, or `None` when the child
/// was stopped by a signal.
fn exit_code_of(status: libc::c_int) -> Option<i32> {
    // A wait status is either a signal in the low seven bits, or the exit code
    // shifted up by eight with those bits cleared.
    if status & 0x7f == 0 {
        Some((status >> 8) & 0xff)
    } else {
        None
    }
}

/// Reap every exited child except the active command leader.
///
/// Waiting by explicit pid leaves the leader's status with [`poll_child`] and
/// cannot steal an unrelated direct child's status from another process owner.
///
/// # Errors
///
/// Returns the platform error when the wait itself fails.
pub fn reap_adopted(except: Option<i32>) -> Result<usize, io::Error> {
    let mut reaped = 0;
    for pid in adopted_children()? {
        if Some(pid) == except {
            continue;
        }
        // SAFETY: `pid` came from this process's direct-child listing, and
        // `WNOHANG` never blocks or writes anywhere beyond the local status.
        let result = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        if result > 0 {
            reaped += 1;
        } else if result < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ECHILD) {
                return Err(error);
            }
        }
    }
    Ok(reaped)
}

/// Whether any process is still a member of `group`.
///
/// # Errors
///
/// Returns the platform error when the probe cannot be performed.
#[cfg(unix)]
pub fn group_holds_process(group: i32) -> io::Result<bool> {
    // Signal 0 runs the kernel's existence and permission checks without
    // delivering anything, so a surviving descendant keeps this true.
    // SAFETY: signal 0 has no effect on any process.
    if unsafe { libc::killpg(group, 0) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Ok(true)
    }
}

/// Refuse to claim an empty process group when the platform cannot inspect it.
#[cfg(not(unix))]
pub fn group_holds_process(_group: i32) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "safe test-child cleanup requires process-group inspection",
    ))
}

/// Ask the kernel to reparent orphaned descendants to this process.
///
/// Without it, a descendant that called `setsid` is reparented to init: the
/// runner can neither signal nor wait for it, and its inherited database access
/// becomes untraceable. This must be set once, before any child is spawned.
///
/// # Errors
///
/// Returns the platform error when the kernel refuses the request.
#[cfg(target_os = "linux")]
pub fn enable_child_subreaper() -> Result<(), io::Error> {
    // SAFETY: `prctl` takes plain integers and touches no runner memory.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Refuse process execution when the kernel cannot make orphaned descendants
/// waitable by the runner.
#[cfg(not(target_os = "linux"))]
pub fn enable_child_subreaper() -> Result<(), io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "safe test-child cleanup requires Linux child subreaper support",
    ))
}

/// Process ids the kernel reparented to this runner that have not exited.
///
/// A descendant that called `setsid` leaves its parent's process group, so
/// `killpg` cannot reach it; with the subreaper set it is still a child of this
/// process and is therefore enumerable here.
///
/// # Errors
///
/// Returns the platform error when the child list cannot be read at all.
#[cfg(target_os = "linux")]
pub fn adopted_children() -> Result<Vec<i32>, io::Error> {
    let tasks = std::fs::read_dir("/proc/self/task").map_err(io::Error::other)?;
    let mut children = Vec::new();
    for task in tasks {
        let task = task.map_err(io::Error::other)?;
        let listing = match std::fs::read_to_string(task.path().join("children")) {
            Ok(listing) => listing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for pid in listing.split_whitespace() {
            children.push(pid.parse::<i32>().map_err(io::Error::other)?);
        }
    }
    children.sort_unstable();
    children.dedup();
    Ok(children)
}

/// Refuse cleanup when the runner cannot enumerate the children it adopted.
#[cfg(not(target_os = "linux"))]
pub fn adopted_children() -> Result<Vec<i32>, io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "safe test-child cleanup requires Linux child enumeration",
    ))
}

/// Deliver `signal` to every member of `group`.
///
/// # Errors
///
/// Returns the platform error when the signal could not be delivered. A group
/// that has already emptied is not an error: the caller asked for a best effort
/// across a set that is known to be racing with exit.
pub fn signal_group(group: i32, signal: i32) -> io::Result<()> {
    // SAFETY: `killpg` takes an integer group and a signal, both supplied here.
    if unsafe { libc::killpg(group, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

/// Deliver `signal` to each listed process.
///
/// A process that has already exited is not a failure — it is the outcome the
/// caller wanted — so `ESRCH` is absorbed. Only a real refusal is reported, and
/// then only when no process at all could be signalled.
///
/// # Errors
///
/// Returns the platform error when no process could be signalled for a reason
/// other than the target already being gone.
pub fn signal_processes(processes: &[i32], signal: i32) -> Result<(), io::Error> {
    let mut delivered = false;
    let mut refused = None;
    for pid in processes {
        // SAFETY: `kill` takes a process id and a signal, both supplied here.
        if unsafe { libc::kill(*pid, signal) } == 0 {
            delivered = true;
            continue;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            refused = Some(error);
        }
    }
    if delivered || processes.is_empty() {
        Ok(())
    } else if let Some(error) = refused {
        Err(error)
    } else {
        // Every listed process had already exited, which is the wanted outcome.
        Ok(())
    }
}
