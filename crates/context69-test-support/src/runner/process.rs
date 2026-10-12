//! Child process lifecycle for runner commands and test executables.
//!
//! Each command runs in its own process group under a Linux subreaper. Timeout,
//! parent cancellation, and unexpected descendants are settled before this
//! function returns, so the caller may release its database advisory lock.

use std::io;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::cancellation;
use super::process_group::{GroupOutcome, Termination};
use super::process_signal::{self, ChildState};
use super::process_tree;

/// Upper bound on how long the runner waits for a child's pipes to reach
/// end-of-stream after its process tree has been settled.
const PIPE_DRAIN_BUDGET: Duration = Duration::from_secs(2);

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Process-wide serialization keeps the subreaper's adopted-child list scoped
/// to one command, including when pure runner tests call this from many threads.
static PROCESS_LOCK: Mutex<()> = Mutex::new(());

/// What a finished (or terminated) child produced.
#[derive(Debug)]
pub struct ChildOutcome {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub interrupted: bool,
    pub leaked_descendants: bool,
    pub elapsed: Duration,
    pub stdout: String,
    pub stderr: String,
    /// How terminating a child tree ended. It is safe before this function
    /// returns, because an unsafe result is held until the tree is quiescent.
    pub termination: Option<Termination>,
}

impl ChildOutcome {
    /// Process exit code, or a readable label when the child was killed by a
    /// signal.
    #[must_use]
    pub fn exit_label(&self) -> String {
        self.exit_code
            .map_or_else(|| "none".to_owned(), |code| code.to_string())
    }

    /// Whether the child finished successfully without cancellation or a leak.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
            && !self.timed_out
            && !self.interrupted
            && !self.leaked_descendants
    }

    /// Whether nothing that inherited database access can still be running.
    #[must_use]
    pub fn is_quiescent(&self) -> bool {
        self.termination.is_none_or(Termination::is_safe)
    }
}

/// Run `command`, reading its output through pipes and enforcing `timeout`.
///
/// # Errors
///
/// Returns the underlying I/O error when the child cannot be spawned or waited
/// for. A non-zero exit is reported through [`ChildOutcome`], not through `Err`.
pub fn run_logged(command: &mut Command, timeout: Duration) -> io::Result<ChildOutcome> {
    run_logged_with_limit(command, timeout, usize::MAX)
}

/// Run a child with a per-stream capture limit while continuing to drain both
/// pipes, so large failure output cannot grow the runner's memory or artifacts.
///
/// # Errors
///
/// Returns the same process and I/O errors as [`run_logged`].
pub fn run_logged_bounded(
    command: &mut Command,
    timeout: Duration,
    max_output_bytes_per_stream: usize,
) -> io::Result<ChildOutcome> {
    run_logged_with_limit(command, timeout, max_output_bytes_per_stream)
}

fn run_logged_with_limit(
    command: &mut Command,
    timeout: Duration,
    max_output_bytes_per_stream: usize,
) -> io::Result<ChildOutcome> {
    let _serial = process_lock();
    process_signal::enable_child_subreaper()?;
    // Verify the kernel interface before starting a command whose descendants
    // would otherwise be impossible to account for.
    process_signal::adopted_children()?;

    if cancellation::requested() {
        return Ok(empty_interrupted());
    }

    let started = Instant::now();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_process_group(command);

    let mut child = command.spawn()?;
    let group = process_signal::group_of(&child)?;
    let pid = i32::try_from(child.id()).map_err(io::Error::other)?;
    let stdout = child
        .stdout
        .take()
        .map(|reader| spawn_reader(reader, max_output_bytes_per_stream));
    let stderr = child
        .stderr
        .take()
        .map(|reader| spawn_reader(reader, max_output_bytes_per_stream));

    let (timed_out, interrupted, leaked_descendants, exit_code, termination) = loop {
        match process_signal::poll_child(pid) {
            Ok(ChildState::Exited(code)) => {
                let interrupted = cancellation::requested();
                let (leaked, termination) = settle_successful_exit(group);
                break (false, interrupted, leaked, code, termination);
            }
            Ok(ChildState::Running) if cancellation::requested() => {
                let termination = terminate_or_hold(pid, group);
                let code = match process_signal::poll_child(pid) {
                    Ok(ChildState::Exited(code)) => code,
                    Ok(ChildState::Running) | Err(_) => None,
                };
                break (false, true, false, code, Some(termination));
            }
            Ok(ChildState::Running) if started.elapsed() >= timeout => {
                let termination = terminate_or_hold(pid, group);
                let code = match process_signal::poll_child(pid) {
                    Ok(ChildState::Exited(code)) => code,
                    Ok(ChildState::Running) | Err(_) => None,
                };
                break (true, false, false, code, Some(termination));
            }
            Ok(ChildState::Running) => std::thread::sleep(POLL_INTERVAL),
            Err(error) => {
                let _ = terminate_or_hold(pid, group);
                return Err(error);
            }
        }
    };

    Ok(ChildOutcome {
        exit_code,
        timed_out,
        interrupted,
        leaked_descendants,
        elapsed: started.elapsed(),
        stdout: String::from_utf8_lossy(&drain(stdout)).into_owned(),
        stderr: String::from_utf8_lossy(&drain(stderr)).into_owned(),
        termination,
    })
}

fn process_lock() -> MutexGuard<'static, ()> {
    PROCESS_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn empty_interrupted() -> ChildOutcome {
    ChildOutcome {
        exit_code: None,
        timed_out: false,
        interrupted: true,
        leaked_descendants: false,
        elapsed: Duration::ZERO,
        stdout: String::new(),
        stderr: String::new(),
        termination: None,
    }
}

fn terminate_or_hold(pid: i32, group: i32) -> Termination {
    let result = process_tree::terminate_child(pid, group);
    finish_termination(result, Some(pid), group)
}

fn settle_successful_exit(group: i32) -> (bool, Option<Termination>) {
    match process_tree::terminate_descendants(group) {
        Ok(None) => (false, None),
        Ok(Some(termination)) => (true, Some(finish_termination(Ok(termination), None, group))),
        Err(error) => (true, Some(finish_termination(Err(error), None, group))),
    }
}

fn finish_termination(
    result: Result<Termination, io::Error>,
    leader: Option<i32>,
    group: i32,
) -> Termination {
    match result {
        Ok(termination) if termination.is_safe() => termination,
        _ => {
            process_tree::wait_until_quiescent(leader, group);
            Termination {
                tree: GroupOutcome::Reaped,
                adopted_still_running: 0,
            }
        }
    }
}

fn spawn_reader<R: io::Read + Send + 'static>(
    mut reader: R,
    max_output_bytes: usize,
) -> JoinHandle<io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let mut chunk = [0; 8192];
        let mut truncated = false;
        loop {
            let count = reader.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            let keep = max_output_bytes.saturating_sub(buffer.len()).min(count);
            buffer.extend_from_slice(&chunk[..keep]);
            truncated |= keep < count;
        }
        if truncated {
            buffer.extend_from_slice(b"\n...[captured output truncated]...\n");
        }
        Ok(buffer)
    })
}

/// Collect a reader thread's bytes, giving up once the budget elapses.
fn drain(handle: Option<JoinHandle<io::Result<Vec<u8>>>>) -> Vec<u8> {
    let Some(handle) = handle else {
        return Vec::new();
    };
    let deadline = Instant::now() + PIPE_DRAIN_BUDGET;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return Vec::new();
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    handle.join().ok().and_then(Result::ok).unwrap_or_default()
}

#[cfg(unix)]
fn isolate_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn isolate_process_group(_command: &mut Command) {}
