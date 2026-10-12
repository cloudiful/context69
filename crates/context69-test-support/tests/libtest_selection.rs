use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use context69_test_support::runner::execute;

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock follows the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "context69-libtest-selection-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("synthetic test directory is unique");
        Self(path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn runner_selects_multiple_ignored_cases_from_a_real_libtest_binary() {
    const SYNTHETIC_TESTS: &str = r#"
#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn selected_db_case_one() {}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn selected_db_case_two() {}

#[test]
fn unselected_pure_case() {
    panic!("unselected synthetic case ran");
}

#[test]
#[ignore = "requires configured reusable PostgreSQL"]
fn selected_db_case_one_suffix() {
    panic!("unselected ignored prefix sibling ran");
}
"#;

    let scratch = ScratchDir::new();
    let source = scratch.0.join("synthetic_tests.rs");
    let executable = scratch.0.join("synthetic_tests");
    std::fs::write(&source, SYNTHETIC_TESTS).expect("synthetic source is writable");

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compilation = Command::new(rustc)
        .args(["--edition=2024", "--test"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("rustc is available to Cargo tests");
    assert!(
        compilation.status.success(),
        "synthetic libtest compilation failed: {}",
        String::from_utf8_lossy(&compilation.stderr)
    );

    let selected = vec![
        "selected_db_case_one".to_owned(),
        "selected_db_case_two".to_owned(),
    ];
    let arguments = execute::test_arguments(&selected).expect("two tests are selected");
    assert_eq!(
        arguments
            .iter()
            .filter(|argument| *argument == "--exact")
            .count(),
        1
    );

    let outcome = Command::new(executable)
        .env_clear()
        .args(arguments)
        .output()
        .expect("synthetic libtest binary starts");
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    assert!(
        outcome.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("running 2 tests"), "{stdout}");
    assert!(
        stdout.contains("test selected_db_case_one ... ok"),
        "{stdout}"
    );
    assert!(
        stdout.contains("test selected_db_case_two ... ok"),
        "{stdout}"
    );
    assert!(!stdout.contains("unselected_pure_case"), "{stdout}");
    assert!(!stdout.contains("selected_db_case_one_suffix"), "{stdout}");
    assert!(
        !stderr.contains("unselected synthetic case ran"),
        "{stderr}"
    );
    assert!(
        stdout.contains("2 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out"),
        "{stdout}"
    );
}

#[test]
fn empty_selection_is_refused_instead_of_running_every_ignored_case() {
    assert!(execute::test_arguments(&[]).is_err());
}
