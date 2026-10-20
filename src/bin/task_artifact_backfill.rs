//! Standalone entry for the audited task-artifact maintenance tool.
//!
//! `context69 task-artifacts` runs the same shared implementation from the
//! application binary; this wrapper exists for development use. It owns only
//! the process exit status and the `fatal:` prefix.

#[tokio::main]
async fn main() {
    if let Err(error) =
        context69::services::library::artifact_backfill::run_cli(std::env::args().skip(1)).await
    {
        eprintln!("fatal: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use context69::services::library::artifact_backfill::{ArtifactDirection, parse_args};

    fn parse(
        raw: &[&str],
    ) -> anyhow::Result<context69::services::library::artifact_backfill::ArtifactBackfillArgs> {
        parse_args(raw.iter().copied())
    }

    #[test]
    fn default_invocation_is_preview_only() {
        let args = parse(&[]).expect("defaults parse");
        assert!(!args.apply);
        assert_eq!(args.direction(), ArtifactDirection::Forward);
    }

    #[test]
    fn apply_requires_the_acknowledgement_flag() {
        assert!(parse(&["--apply"]).is_err());
        assert!(parse(&["--apply", "--acknowledge-reader-upgrade"]).is_ok());
    }

    #[test]
    fn false_values_never_enable_a_destructive_flag() {
        assert!(parse(&["--apply=false"]).is_err());
        assert!(parse(&["--apply=false", "--acknowledge-reader-upgrade=false"]).is_err());
        assert!(parse(&["--restore-inline=false"]).is_err());
        assert!(
            parse(&[
                "--restore-inline=false",
                "--apply=false",
                "--acknowledge-inline-growth=false"
            ])
            .is_err()
        );
        assert!(parse(&["--acknowledge-inline-growth=false"]).is_err());
    }

    #[test]
    fn restore_is_preview_by_default_and_needs_the_growth_acknowledgement() {
        let preview = parse(&["--restore-inline"]).expect("preview parses");
        assert!(preview.restore_inline);
        assert!(!preview.apply);
        assert_eq!(preview.direction(), ArtifactDirection::RestoreInline);
        assert!(parse(&["--restore-inline", "--apply"]).is_err());
        assert!(parse(&["--restore-inline", "--apply", "--acknowledge-inline-growth"]).is_ok());
        assert!(
            parse(&[
                "--restore-inline",
                "--apply",
                "--acknowledge-inline-growth",
                "--acknowledge-reader-upgrade"
            ])
            .is_err()
        );
    }
}
