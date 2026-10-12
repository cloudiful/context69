//! Parent cancellation is handled inside the runner process so active test
//! children are gone before their database lock can be released.

#[cfg(target_os = "linux")]
#[test]
fn parent_signal_stops_the_active_tree_before_return() {
    const CHILD_PROBE: &str = "CONTEXT69_TEST_SUPPORT_CANCELLATION_PROBE";
    if std::env::var_os(CHILD_PROBE).is_some() {
        cancellation_probe();
        return;
    }

    use std::process::Command;
    use std::time::Duration;

    use context69_test_support::runner::process;

    let executable = std::env::current_exe().expect("test executable path");
    let outcome = process::run_logged(
        Command::new(executable)
            .arg("--exact")
            .arg("parent_signal_stops_the_active_tree_before_return")
            .arg("--nocapture")
            .env(CHILD_PROBE, "1"),
        Duration::from_secs(20),
    )
    .expect("isolated signal probe starts");

    assert!(outcome.succeeded(), "isolated probe failed: {outcome:?}");
    assert!(outcome.stdout.contains("test result: ok"), "{outcome:?}");
}

#[cfg(target_os = "linux")]
fn cancellation_probe() {
    use std::process::Command;
    use std::time::Duration;

    use context69_test_support::runner::{cancellation, process, process_signal};

    cancellation::install_signal_handlers().expect("runner signal handlers install");
    process_signal::enable_child_subreaper().expect("orphan adoption is available");

    let sender = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(150));
        // SAFETY: signal only the current isolated probe process; its handler
        // records cancellation and leaves child cleanup to runner control flow.
        let result = unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
        assert_eq!(result, 0, "the test signal was delivered");
    });
    let outcome = process::run_logged(
        Command::new("/bin/sh").arg("-c").arg("sleep 30 & wait"),
        Duration::from_secs(15),
    )
    .expect("the interrupted child is terminated and reaped");
    sender.join().expect("signal sender completes");

    assert!(outcome.interrupted, "the parent cancellation is reported");
    assert!(!outcome.succeeded(), "an interrupted target cannot pass");
    assert!(
        outcome.is_quiescent(),
        "the child tree is gone before return"
    );
}
