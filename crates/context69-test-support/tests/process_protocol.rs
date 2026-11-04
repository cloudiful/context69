//! Runner protocol: invocation parsing, child environment hygiene, process
//! lifecycle, and advisory-lock admission.
//!
//! The process tests spawn short-lived shell children that touch nothing but
//! this machine's process table, mirroring the safety semantics the previous
//! Python runner suite pinned.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use context69_test_support::config::Endpoint;
use context69_test_support::runner::lock::{self, Admission, LockKeys};
use context69_test_support::runner::output::{RunDir, RunScope, RunStatus, RunSummary};
use context69_test_support::runner::process;
use context69_test_support::runner::{self, Mode};

fn endpoint() -> Endpoint {
    Endpoint::parse("postgres://synthetic-user:synthetic-secret@db.test:5433/synthetic_db")
        .expect("synthetic fixture target")
}

fn invocation(args: &[&str]) -> Result<runner::Invocation, String> {
    runner::parse_invocation(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
}

#[test]
fn invocation_defaults_to_a_run_that_is_not_reset_authorized() {
    let parsed = invocation(&[]).expect("no flags is valid");

    assert_eq!(parsed.mode, Mode::Run);
    assert!(!parsed.reset_authorized);
    assert!(!parsed.allow_idle_connections);
}

#[test]
fn the_checkpoint_flag_is_opt_in_and_modes_do_not_combine() {
    let authorized = invocation(&["--authorize-reset"]).expect("valid");
    assert!(authorized.reset_authorized);
    assert_eq!(authorized.mode, Mode::Run);

    assert_eq!(
        invocation(&["--preflight"]).expect("valid").mode,
        Mode::Preflight
    );
    assert_eq!(invocation(&["--list"]).expect("valid").mode, Mode::List);
    assert!(
        invocation(&["--preflight", "--list"]).is_err(),
        "a mode conflict must not silently pick one"
    );
}

#[test]
fn idle_policy_is_explicit_not_reset_consent_and_invalid_for_list() {
    for args in [
        vec!["--allow-idle-connections"],
        vec!["--preflight", "--allow-idle-connections"],
    ] {
        let parsed = invocation(&args).unwrap();
        assert!(parsed.allow_idle_connections);
        assert!(!parsed.reset_authorized);
    }
    let run = invocation(&["--authorize-reset", "--allow-idle-connections"]).unwrap();
    assert!(run.reset_authorized && run.allow_idle_connections);
    assert!(invocation(&["--list", "--allow-idle-connections"]).is_err());
    assert!(invocation(&["--allow-idle-connections", "--list"]).is_err());
}

#[test]
fn an_unknown_flag_is_refused_rather_than_forwarded() {
    let error = invocation(&["--database-url", "postgres://u:p@h/db"]).expect_err("must refuse");

    assert!(error.contains("unsupported argument"), "{error}");
}

#[test]
fn the_child_environment_carries_the_url_only_in_the_test_variable() {
    let endpoint = endpoint();
    let environment: HashMap<String, String> =
        runner::child_environment(&endpoint).into_iter().collect();

    assert_eq!(
        environment.get(context69_test_support::TEST_DATABASE_URL_VAR),
        Some(&endpoint.expose().to_owned())
    );
    assert!(
        !environment.contains_key("DATABASE_URL"),
        "an ambient production URL must be removed before the child runs"
    );
    assert!(
        environment.keys().all(|key| !key.starts_with("PG")),
        "libpq variables must be removed so they cannot redirect the child"
    );
    assert!(
        !environment
            .keys()
            .any(|key| key.starts_with("CONTEXT69_APP__"))
    );
    assert_eq!(
        environment.get("SQLX_OFFLINE").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        environment
            .get("CONTEXT69_TEST_REPO_ROOT")
            .map(String::as_str)
            .map(std::path::Path::new),
        Some(context69_test_support::repo_root().as_path())
    );
}

#[test]
fn advisory_lock_keys_follow_the_target_database_identity() {
    let first = LockKeys::new(4242);
    let same = LockKeys::new(4242);
    let other = LockKeys::new(4243);

    assert!(first.same_lock(&same));
    assert!(!first.same_lock(&other), "two databases must never contend");
    assert_ne!(first.namespace, 0);
}

#[test]
fn advisory_lock_key_sql_uses_valid_comments_and_preserves_database_scoping() {
    let sql = include_str!("../src/sql/advisory_lock_keys.sql");

    assert!(
        !sql.lines().any(|line| line.trim_start().starts_with("//")),
        "SQL line comments must use PostgreSQL syntax"
    );
    assert!(sql.contains("hashtext('context69-backend-test-runner')"));
    assert!(sql.contains("(SELECT oid::int4 FROM pg_database WHERE datname = current_database())"));
}

#[test]
fn a_runner_without_the_lock_is_refused_instead_of_resetting() {
    assert_eq!(lock::admit(None), Admission::RefuseAnotherRunnerActive);
}

#[test]
fn run_artifacts_live_under_the_target_directory_and_expire() {
    let repo_root =
        std::env::temp_dir().join(format!("context69-run-artifacts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo_root);

    let stale = RunDir::create(&repo_root, "stale-run").expect("first run directory");
    let recent = RunDir::create(&repo_root, "recent-run").expect("second run directory");
    age(stale.path(), Duration::from_secs(8 * 24 * 60 * 60));

    let current = RunDir::create(&repo_root, "current-run").expect("third run directory");

    assert!(
        !stale.path().exists(),
        "an expired run directory is removed"
    );
    assert!(
        recent.path().exists(),
        "a concurrent run's directory must never be removed"
    );
    assert!(current.path().exists());
    assert!(current.path().starts_with(repo_root.join("target")));
    let _ = std::fs::remove_dir_all(&repo_root);
}

fn age(path: &Path, by: Duration) {
    let stamp = std::time::SystemTime::now() - by;
    std::fs::File::open(path)
        .and_then(|file| file.set_modified(stamp))
        .expect("run directory timestamp is settable");
}

#[cfg(unix)]
#[test]
fn a_timed_out_child_is_killed_and_waited_for_before_the_caller_continues() {
    let marker = std::env::temp_dir().join(format!("context69-timeout-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let script = format!("echo $$ > {}; sleep 30", marker.display());

    let outcome = process::run_logged(
        Command::new("sh").arg("-c").arg(&script),
        Duration::from_millis(150),
    )
    .expect("child spawns");

    assert!(outcome.timed_out);
    assert!(!outcome.succeeded());
    let pid: i32 = std::fs::read_to_string(&marker)
        .expect("child recorded its pid")
        .trim()
        .parse()
        .expect("pid is numeric");
    assert!(
        wait_for_exit(pid, Duration::from_secs(5)),
        "the timed-out child must already be gone when run_logged returns"
    );
}

#[cfg(unix)]
#[test]
fn a_child_that_leaves_a_descendant_is_failed_and_cleaned_up_on_exit() {
    let script = "setsid sleep 30 </dev/null >/dev/null 2>&1 & echo $!; exit 0";

    let outcome = process::run_logged(
        Command::new("sh").arg("-c").arg(script),
        Duration::from_secs(10),
    )
    .expect("child spawns");

    assert!(
        !outcome.succeeded(),
        "a leaked descendant is a test failure"
    );
    assert!(
        outcome.leaked_descendants,
        "the child left a process behind"
    );
    assert!(
        outcome.is_quiescent(),
        "the process tree is settled on return"
    );
    let pid: i32 = outcome
        .stdout
        .trim()
        .parse()
        .expect("the child printed its descendant pid");
    assert!(
        wait_for_exit(pid, Duration::from_secs(5)),
        "a descendant may not retain database access after the target exits"
    );
}

#[cfg(unix)]
#[test]
fn captured_output_is_returned_and_nothing_is_written_to_a_file_by_the_runner() {
    let outcome = process::run_logged(
        Command::new("sh")
            .arg("-c")
            .arg("echo out; echo err 1>&2; exit 3"),
        Duration::from_secs(10),
    )
    .expect("child spawns");

    assert_eq!(outcome.exit_code, Some(3));
    assert_eq!(outcome.stdout.trim(), "out");
    assert_eq!(outcome.stderr.trim(), "err");
    assert_eq!(outcome.exit_label(), "3");
}

#[cfg(unix)]
#[test]
fn bounded_capture_truncates_both_streams_and_still_drains_the_child() {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("printf 'out-prefix'; printf 'err-prefix' >&2");
    let outcome = process::run_logged_bounded(&mut command, Duration::from_secs(10), 4)
        .expect("child spawns");
    let marker = "\n...[captured output truncated]...\n";

    assert!(outcome.succeeded());
    assert!(outcome.is_quiescent());
    assert_eq!(outcome.stdout, format!("out-{marker}"));
    assert_eq!(outcome.stderr, format!("err-{marker}"));
}

fn process_state(pid: i32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.split_once(')')?.1;
    tail.split_whitespace().next()?.chars().next()
}

fn wait_for_exit(pid: i32, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        if !matches!(process_state(pid), Some('R') | Some('S') | Some('D')) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn target_scope_defaults_to_the_full_plan() {
    assert!(invocation(&[]).expect("valid").selected_targets.is_empty());
}

#[test]
fn repeated_exact_targets_preserve_request_order_up_to_the_bound() {
    let parsed = invocation(&[
        "--target",
        "context69-aaa_storage",
        "--target",
        "context69-bbb_queue",
    ])
    .expect("two exact labels are valid");
    assert_eq!(
        parsed.selected_targets,
        vec![
            "context69-aaa_storage".to_owned(),
            "context69-bbb_queue".to_owned()
        ]
    );

    let many: Vec<String> = (0..20)
        .flat_map(|index| ["--target".to_owned(), format!("context69-target{index}")])
        .collect();
    assert_eq!(
        runner::parse_invocation(&many)
            .expect("twenty labels are valid")
            .selected_targets
            .len(),
        20
    );
    let mut too_many = many;
    too_many.push("--target".to_owned());
    too_many.push("context69-target20".to_owned());
    let error = runner::parse_invocation(&too_many).expect_err("twenty-one labels are refused");
    assert!(error.contains("at most 20"), "{error}");
}

#[test]
fn malformed_repeated_or_mode_conflicting_target_scope_is_refused() {
    assert!(invocation(&["--target"]).is_err());
    assert!(invocation(&["--target", ""]).is_err());
    assert!(invocation(&["--target", "--authorize-reset"]).is_err());
    assert!(
        invocation(&[
            "--target",
            "context69-aaa_storage",
            "--target",
            "context69-aaa_storage"
        ])
        .is_err()
    );
    for args in [
        vec!["--list", "--target", "context69-aaa_storage"],
        vec!["--target", "context69-aaa_storage", "--list"],
        vec!["--preflight", "--target", "context69-aaa_storage"],
    ] {
        assert!(invocation(&args).is_err(), "{args:?}");
    }
    assert!(
        invocation(&["--target", "context69-aaa_storage", "--authorize-reset"])
            .expect("a scoped authorized run is valid")
            .reset_authorized
    );
}

#[test]
fn unsupported_scoping_bypasses_remain_refused() {
    for flag in ["--from-target", "--skip-pure", "--no-reset"] {
        let error = invocation(&[flag]).expect_err("bypass flags stay unsupported");
        assert!(error.contains("unsupported argument"), "{error}");
    }
}

#[test]
fn run_summary_distinguishes_full_and_scoped_reports() {
    let mut full = RunSummary::new("full-run".to_owned());
    assert_eq!(full.scope, RunScope::Full);
    full.record(full.scope.completion_label());
    let rendered = serde_json::to_value(&full).expect("full summary serializes");
    assert_eq!(rendered["scope"], "full");
    assert_eq!(rendered["actions"][0], "sequence:complete");

    let mut scoped = RunSummary::new("scoped-run".to_owned());
    scoped.scope = RunScope::Targets;
    scoped.requested_targets = vec![
        "context69-bbb_queue".to_owned(),
        "context69-aaa_storage".to_owned(),
    ];
    scoped.resolved_targets = vec![
        "context69-aaa_storage".to_owned(),
        "context69-bbb_queue".to_owned(),
    ];
    scoped.selected_ordinary_tests = 3;
    scoped.database_tests = 730;
    scoped.record(scoped.scope.completion_label());
    let rendered = serde_json::to_value(&scoped).expect("scoped summary serializes");
    assert_eq!(rendered["scope"], "targets");
    assert_eq!(rendered["requested_targets"][0], "context69-bbb_queue");
    assert_eq!(rendered["resolved_targets"][0], "context69-aaa_storage");
    assert_eq!(rendered["selected_ordinary_tests"], 3);
    assert_eq!(
        rendered["database_tests"], 730,
        "the existing whole-manifest classification field is unchanged"
    );
    assert_eq!(rendered["actions"][0], "sequence:scoped-complete");
    assert_ne!(
        RunScope::Full.completion_label(),
        RunScope::Targets.completion_label(),
        "a scoped pass can never carry the full-gate completion label"
    );

    scoped.status = RunStatus::Failed;
    scoped.failures.push("target failed".to_owned());
    assert!(
        !scoped
            .actions
            .iter()
            .any(|action| action == "sequence:complete"),
        "a failed scoped run never carries the full-gate completion label"
    );
    let rendered = serde_json::to_value(&scoped).expect("failed scoped summary serializes");
    assert_eq!(rendered["status"], "failed");
}
