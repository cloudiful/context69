//! The runner's pure Cargo stage controls whether database work is admitted.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use context69_test_support::runner::{process, pure};

const SYNTHETIC_TESTS: &str = r#"
#[test]
fn selected_pure_case_passes() {}

#[test]
fn selected_pure_case_fails() {
    panic!("synthetic pure-suite failure");
}
"#;

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock follows the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "context69-pure-gate-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("synthetic harness directory is unique");
        Self(path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn a_failed_synthetic_pure_test_blocks_database_actions() {
    let (_scratch, binary) = compile_harness();
    let database_actions = AtomicUsize::new(0);
    let result = pure::run_before_database(
        || run_case(&binary, "selected_pure_case_fails"),
        |_| async {
            database_actions.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;

    assert!(matches!(
        result,
        Err(pure::GateFailure::PureFailed(outcome)) if outcome.exit_code.is_some_and(|code| code != 0)
    ));
    assert_eq!(database_actions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_successful_synthetic_pure_test_admits_database_actions() {
    let (_scratch, binary) = compile_harness();
    let database_actions = AtomicUsize::new(0);
    let result = pure::run_before_database(
        || run_case(&binary, "selected_pure_case_passes"),
        |_| async {
            database_actions.fetch_add(1, Ordering::SeqCst);
            Ok("database stage admitted")
        },
    )
    .await
    .expect("a successful pure stage admits the next stage");

    assert_eq!(result, "database stage admitted");
    assert_eq!(database_actions.load(Ordering::SeqCst), 1);
}

fn compile_harness() -> (ScratchDir, PathBuf) {
    let scratch = ScratchDir::new();
    let source = scratch.0.join("pure_gate_tests.rs");
    let binary = scratch.0.join("pure_gate_tests");
    std::fs::write(&source, SYNTHETIC_TESTS).expect("synthetic harness source is writable");

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compilation = Command::new(rustc)
        .args(["--edition=2024", "--test"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("rustc is available to Cargo tests");
    assert!(
        compilation.status.success(),
        "synthetic harness compilation failed: {}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    (scratch, binary)
}

fn run_case(binary: &Path, test_name: &str) -> Result<process::ChildOutcome, String> {
    let mut command = Command::new(binary);
    command.env_clear().args(["--exact", test_name]);
    process::run_logged(&mut command, Duration::from_secs(10))
        .map_err(|error| format!("synthetic pure stage could not start ({})", error.kind()))
}
