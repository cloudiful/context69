//! Required-extraction result gate (issue #89 P3).
//!
//! The extraction worker returns terminal jobs as data (`Ok`), so the outer
//! stage sees no `Err` and would finalize a failed required extraction. This
//! helper inspects typed job outcomes and decides whether the owning item may
//! finalize. It never performs writes: the owned task stage owns the CAS mark,
//! the unowned immediate path never marks.

use crate::contracts::{ExtractionJobResponse, ExtractionJobStatus};
use uuid::Uuid;

/// Typed file-outcome report beside `convert_file_extractions`. The owned task
/// stage consumes `decision`; the unowned immediate path ignores it and never
/// marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractionGateReport {
    pub directive_present: bool,
    pub required: bool,
    pub template_key: String,
    pub job_count: usize,
    pub had_documents: bool,
    pub decision: ExtractionGateDecision,
}

/// Outcome of the required-extraction gate for one file conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExtractionGateDecision {
    /// No directive: extraction is a no-op and the stage may progress.
    NoDirective,
    /// All jobs succeeded (or no blocking condition); the stage may progress.
    Pass,
    /// Optional terminal failure/skip/empty: warn and progress. Carries only
    /// safe identifiers, never raw provider error/payload.
    OptionalWarning {
        template_key: String,
        job_id: Option<Uuid>,
        reason: &'static str,
    },
    /// Required terminal failure/skip/empty: the owned stage must CAS-mark the
    /// file failed and return `Failed(stage=extraction)`. The message is a
    /// fixed safe explanation with identifiers only.
    RequiredBlocked {
        template_key: String,
        job_id: Option<Uuid>,
        reason: &'static str,
        message: String,
    },
    /// Unexpected nonterminal job: invariant violation, never healthy
    /// completion. The stage must not finalize regardless of `required`.
    InvariantViolation { message: String },
}

/// Evaluate one file's extraction outcomes.
///
/// * `directive` is `None` when the file carries no extraction template key.
/// * `required` is the file-side dedicated flag (legacy/omitted `true`).
/// * `template_key` names the requested template for safe diagnostics.
/// * `jobs` aggregates every `convert` response across the file's documents.
/// * `had_documents` is false when the file maps to zero documents.
pub(crate) fn evaluate_extraction_gate(
    directive_present: bool,
    required: bool,
    template_key: &str,
    jobs: &[ExtractionJobResponse],
    had_documents: bool,
) -> ExtractionGateDecision {
    if !directive_present {
        return ExtractionGateDecision::NoDirective;
    }
    if !had_documents {
        return block_or_warn(
            required,
            template_key,
            None,
            "missing_document",
            format!("required extraction has no mapped documents for template '{template_key}'"),
            format!("optional extraction has no mapped documents for template '{template_key}'"),
        );
    }
    if jobs.is_empty() {
        return block_or_warn(
            required,
            template_key,
            None,
            "empty",
            format!("required extraction produced no jobs for template '{template_key}'"),
            format!("optional extraction produced no jobs for template '{template_key}'"),
        );
    }
    let mut first_warning: Option<ExtractionGateDecision> = None;
    for job in jobs {
        match job.status {
            ExtractionJobStatus::Succeeded => continue,
            ExtractionJobStatus::Failed => {
                let decision = block_or_warn(
                    required,
                    template_key,
                    Some(job.job_id),
                    "failed",
                    format!(
                        "required extraction failed for template '{}' (job {})",
                        template_key, job.job_id
                    ),
                    format!(
                        "optional extraction failed for template '{}' (job {})",
                        template_key, job.job_id
                    ),
                );
                if matches!(decision, ExtractionGateDecision::RequiredBlocked { .. }) {
                    return decision;
                }
                if first_warning.is_none() {
                    first_warning = Some(decision);
                }
            }
            ExtractionJobStatus::Skipped => {
                let decision = block_or_warn(
                    required,
                    template_key,
                    Some(job.job_id),
                    "skipped",
                    format!(
                        "required extraction skipped for template '{}' (job {})",
                        template_key, job.job_id
                    ),
                    format!(
                        "optional extraction skipped for template '{}' (job {})",
                        template_key, job.job_id
                    ),
                );
                if matches!(decision, ExtractionGateDecision::RequiredBlocked { .. }) {
                    return decision;
                }
                if first_warning.is_none() {
                    first_warning = Some(decision);
                }
            }
            ExtractionJobStatus::Queued | ExtractionJobStatus::Running => {
                return ExtractionGateDecision::InvariantViolation {
                    message: format!(
                        "extraction job {} for template '{template_key}' did not reach a terminal status",
                        job.job_id
                    ),
                };
            }
        }
    }
    if let Some(warning) = first_warning {
        warning
    } else {
        ExtractionGateDecision::Pass
    }
}

fn block_or_warn(
    required: bool,
    template_key: &str,
    job_id: Option<Uuid>,
    reason: &'static str,
    required_message: String,
    _optional_message: String,
) -> ExtractionGateDecision {
    if required {
        ExtractionGateDecision::RequiredBlocked {
            template_key: template_key.to_string(),
            job_id,
            reason,
            message: required_message,
        }
    } else {
        ExtractionGateDecision::OptionalWarning {
            template_key: template_key.to_string(),
            job_id,
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn job(status: ExtractionJobStatus) -> ExtractionJobResponse {
        ExtractionJobResponse {
            job_id: Uuid::new_v4(),
            document_id: 1,
            template_key: "t-1".to_string(),
            template_version: 1,
            source_record_hash: "h".to_string(),
            status,
            attempt_count: 1,
            error_message: Some("raw provider boom (must never surface)".to_string()),
            failure_class: None,
            next_attempt_at: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn no_directive_is_noop() {
        assert_eq!(
            evaluate_extraction_gate(false, true, "t-1", &[], false),
            ExtractionGateDecision::NoDirective
        );
    }

    #[test]
    fn required_failed_skipped_empty_block_without_raw_error() {
        for (jobs, reason) in [
            (vec![job(ExtractionJobStatus::Failed)], "failed"),
            (vec![job(ExtractionJobStatus::Skipped)], "skipped"),
            (Vec::new(), "empty"),
        ] {
            let had_documents = !jobs.is_empty() || reason == "empty";
            let decision = evaluate_extraction_gate(true, true, "t-1", &jobs, had_documents);
            match decision {
                ExtractionGateDecision::RequiredBlocked {
                    reason: got,
                    message,
                    ..
                } => {
                    assert_eq!(got, reason);
                    assert!(!message.contains("raw provider boom"));
                }
                other => panic!("expected block for {reason}, got {other:?}"),
            }
        }
        match evaluate_extraction_gate(true, true, "t-1", &[], false) {
            ExtractionGateDecision::RequiredBlocked { reason, .. } => {
                assert_eq!(reason, "missing_document")
            }
            other => panic!("expected missing_document block, got {other:?}"),
        }
    }

    #[test]
    fn optional_terminal_warns_and_passes_without_raw_error() {
        for status in [ExtractionJobStatus::Failed, ExtractionJobStatus::Skipped] {
            let decision = evaluate_extraction_gate(true, false, "t-1", &[job(status)], true);
            match decision {
                ExtractionGateDecision::OptionalWarning { reason, .. } => {
                    assert!(matches!(reason, "failed" | "skipped"));
                }
                other => panic!("expected warning, got {other:?}"),
            }
            assert!(matches!(
                decision,
                ExtractionGateDecision::OptionalWarning { .. }
            ));
        }
        match evaluate_extraction_gate(true, false, "t-1", &[], true) {
            ExtractionGateDecision::OptionalWarning { reason, .. } => assert_eq!(reason, "empty"),
            other => panic!("expected empty warning, got {other:?}"),
        }
    }

    #[test]
    fn nonterminal_is_invariant_violation_not_progress() {
        for status in [ExtractionJobStatus::Queued, ExtractionJobStatus::Running] {
            for required in [true, false] {
                let decision =
                    evaluate_extraction_gate(true, required, "t-1", &[job(status)], true);
                match decision {
                    ExtractionGateDecision::InvariantViolation { .. } => {}
                    other => panic!("expected invariant, got {other:?}"),
                }
                assert!(matches!(
                    decision,
                    ExtractionGateDecision::InvariantViolation { .. }
                ));
            }
        }
    }

    #[test]
    fn success_passes_and_mixed_required_prefers_block() {
        assert_eq!(
            evaluate_extraction_gate(
                true,
                true,
                "t-1",
                &[job(ExtractionJobStatus::Succeeded)],
                true
            ),
            ExtractionGateDecision::Pass
        );
        match evaluate_extraction_gate(
            true,
            true,
            "t-1",
            &[
                job(ExtractionJobStatus::Succeeded),
                job(ExtractionJobStatus::Failed),
            ],
            true,
        ) {
            ExtractionGateDecision::RequiredBlocked { .. } => {}
            other => panic!("expected block, got {other:?}"),
        }
    }
}
