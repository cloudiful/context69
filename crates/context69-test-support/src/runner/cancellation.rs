//! Parent-process cancellation without abandoning an active test child.
//!
//! The signal handler only sets an atomic flag. The runner's normal control flow
//! observes it and terminates/reaps the active child tree before it can release
//! the database advisory lock.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Install handlers for interactive and supervisor cancellation.
///
/// # Errors
///
/// Returns an error when the platform cannot install the handlers needed by the
/// process-tree cleanup contract.
#[cfg(target_os = "linux")]
pub fn install_signal_handlers() -> io::Result<()> {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: every field is initialized before the action is passed to the
        // kernel, and the zeroed representation is valid for this C struct.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: the signal set and handler are valid, and the previous action
        // is not needed because this standalone runner owns these signals.
        if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
            return Err(io::Error::last_os_error());
        }
        action.sa_sigaction = record_cancellation as *const () as libc::sighandler_t;
        action.sa_flags = 0;
        // SAFETY: `action` is initialized above and remains valid for the call.
        if unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Refuse interruption-sensitive runner modes when safe signal handling is
/// unavailable.
#[cfg(not(target_os = "linux"))]
pub fn install_signal_handlers() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "safe test-child cancellation requires Linux signal handling",
    ))
}

/// Whether the parent received an interrupt or termination request.
#[must_use]
pub fn requested() -> bool {
    REQUESTED.load(Ordering::Relaxed)
}

extern "C" fn record_cancellation(_signal: libc::c_int) {
    REQUESTED.store(true, Ordering::Relaxed);
}
