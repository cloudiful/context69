//! Thin composition of the backend database-test runner.
//!
//! Every decision lives in the library; this binary only parses flags, builds
//! the run identifier, and maps the summary to an exit code. It never receives
//! a connection URL on the command line.

use std::process::ExitCode;

use context69_test_support::runner::{self, exit};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let invocation = match runner::parse_invocation(&args) {
        Ok(invocation) => invocation,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "usage: context69-backend-test-runner [--preflight | --list] [--authorize-reset]"
            );
            return ExitCode::from(exit::USAGE as u8);
        }
    };

    let repo_root = context69_test_support::repo_root();
    let run_id = run_identifier();
    match runner::run::execute(&repo_root, &run_id, invocation).await {
        Ok(summary) => {
            report(&summary);
            ExitCode::from(code_for(summary.status) as u8)
        }
        Err(message) => {
            eprintln!("backend test run {run_id} could not start: {message}");
            ExitCode::from(exit::FAILED as u8)
        }
    }
}

fn code_for(status: runner::output::RunStatus) -> i32 {
    match status {
        runner::output::RunStatus::Passed => exit::OK,
        runner::output::RunStatus::Refused => exit::REFUSED,
        runner::output::RunStatus::Failed => exit::FAILED,
    }
}

fn report(summary: &runner::output::RunSummary) {
    println!("backend test run {}: {:?}", summary.run_id, summary.status);
    if let Some(fingerprint) = &summary.endpoint_fingerprint {
        println!(
            "target: {fingerprint} via {}",
            summary.endpoint_source.as_deref().unwrap_or("-")
        );
    }
    for action in &summary.actions {
        println!("  {action}");
    }
    for failure in &summary.failures {
        println!("  failure: {failure}");
    }
}

fn run_identifier() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or_else(
            |_| "run".to_owned(),
            |elapsed| format!("{:x}-{:x}", elapsed.as_secs(), elapsed.subsec_nanos()),
        )
}
