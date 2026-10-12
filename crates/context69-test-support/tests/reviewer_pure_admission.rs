use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn actual_runner_stops_at_failed_pure_stage_before_target_resolution() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "context69-review-pure-admission-{}-{nonce}",
        std::process::id()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let source = scratch.0.join("fake_cargo.rs");
    let cargo = scratch.0.join("cargo");
    std::fs::write(
        &source,
        "fn main() { assert_eq!(std::env::args().skip(1).collect::<Vec<_>>(), [\"test\", \"--workspace\", \"--no-fail-fast\", \"--offline\"]); std::process::exit(101); }",
    ).unwrap();
    let compilation = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .args(["--edition=2024"])
        .arg(&source)
        .arg("-o")
        .arg(&cargo)
        .output()
        .unwrap();
    assert!(compilation.status.success());
    let mut paths = vec![scratch.0.clone()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_context69-backend-test-runner"))
        .env_clear()
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env(
            "CONTEXT69_TEST_DATABASE_URL",
            "mysql://synthetic_user:synthetic_password@db.test/not_a_target",
        )
        .arg("--authorize-reset")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("pure:workspace-tests=failed:exit=101"),
        "{stdout}"
    );
    assert!(
        stdout.contains("ordinary workspace tests failed before database admission"),
        "{stdout}"
    );
    assert!(!stdout.contains("target:"), "{stdout}");
    assert!(!stdout.contains("preflight:"), "{stdout}");
    assert!(!stdout.contains("lock:"), "{stdout}");
    assert!(!stdout.contains("synthetic_password"), "{stdout}");
    assert!(!stderr.contains("synthetic_password"), "{stderr}");
}
