//! One typed retry policy for every task-facing failure.
//!
//! The worker used to decide whether a failure may be retried by scanning the
//! failure message for words such as "invalid" or "not found". That both missed
//! the failures that matter most — a body that is too large, a permission
//! error, a rejected upload — and hid transient failures whose message merely
//! mentioned one of those words.
//!
//! This module classifies a failure from its typed error chain instead, so the
//! decision never depends on the text a provider happened to use. Every
//! task-facing failure path shares the decision: a deterministic failure stops
//! after its first attempt, an unknown failure keeps the existing transient
//! behaviour and backoff.

use std::io;

use anyhow::Error;

use crate::domain_errors::DomainError;

/// How the task worker treats one failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    /// The same request fails the same way again, so the item fails without an
    /// automatic retry. A manual retry stays available once the cause is fixed.
    Deterministic,
    /// The failure may succeed on a later attempt, so the existing backoff
    /// applies.
    Transient,
}

/// A failure whose producer proved it deterministic.
///
/// The marker exists for the few failures whose wire identity must not change:
/// a deterministic upstream failure keeps its own error code and message, and
/// only the retry decision differs.
#[derive(Debug)]
pub(crate) struct DeterministicFailure {
    source: Error,
}

impl std::fmt::Display for DeterministicFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for DeterministicFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Mark one failure as deterministic without changing its message or its typed
/// cause.
pub(crate) fn mark_deterministic(error: impl Into<Error>) -> Error {
    Error::new(DeterministicFailure {
        source: error.into(),
    })
}

/// Classify one failure.
///
/// The order of the rules is the policy: an explicit marker wins over every
/// typed default, a hard domain variant wins over an HTTP status, and an
/// unknown failure stays transient.
pub(crate) fn classify(error: &Error) -> FailureClass {
    let causes: Vec<&(dyn std::error::Error + 'static)> = error.chain().collect();

    // An explicit retry marker keeps its existing meaning: the caller proved
    // the failure is worth another attempt.
    if causes.iter().any(|cause| {
        cause
            .downcast_ref::<crate::retry::RetryableError>()
            .is_some()
    }) {
        return FailureClass::Transient;
    }

    // A deterministic marker wins over the typed defaults.
    if causes
        .iter()
        .any(|cause| cause.downcast_ref::<DeterministicFailure>().is_some())
    {
        return FailureClass::Deterministic;
    }

    // A hard domain variant is deterministic: the request itself is wrong.
    for domain in domain_errors(error) {
        match domain {
            DomainError::InvalidArgument(_)
            | DomainError::Unauthorized(_)
            | DomainError::Forbidden(_)
            | DomainError::NotFound(_)
            | DomainError::PayloadTooLarge(_)
            | DomainError::UnprocessableEntity(_) => return FailureClass::Deterministic,
            _ => {}
        }
    }

    // An explicit HTTP status refines a broad upstream failure. It never
    // overrides a hard domain variant, which was decided above.
    if let Some(status) = http_status(&causes) {
        return status_class(status);
    }

    // A request that could not even be built fails the same way again.
    for cause in &causes {
        if let Some(error) = cause.downcast_ref::<reqwest::Error>()
            && error.is_builder()
        {
            return FailureClass::Deterministic;
        }
    }

    // Typed provider failures: a rejected parameter and a missing
    // configuration value never become valid on their own. A parse error
    // keeps its provenance erased — the installed client maps every unreadable
    // response body to it, so an interrupted body collection and an invalid
    // answer are indistinguishable — and an unknown operation failure says
    // nothing about the next attempt. Both stay transient.
    for cause in &causes {
        if let Some(provider) = cause.downcast_ref::<docling_convert::PdfConvertError>() {
            match provider {
                docling_convert::PdfConvertError::ValidationError { .. }
                | docling_convert::PdfConvertError::EnvError { .. } => {
                    return FailureClass::Deterministic;
                }
                docling_convert::PdfConvertError::ParseError { .. }
                | docling_convert::PdfConvertError::ApiError { .. }
                | docling_convert::PdfConvertError::OperationError { .. } => {
                    return FailureClass::Transient;
                }
                // The wrapped I/O failure decides this variant.
                docling_convert::PdfConvertError::IoError { .. } => break,
            }
        }
    }

    // Filesystem failures: a missing, unreadable or malformed input does not
    // become valid on its own. A reset, refused or aborted connection and an
    // unknown kind stay transient.
    for cause in &causes {
        if let Some(error) = cause.downcast_ref::<io::Error>() {
            return io_class(error.kind());
        }
    }

    FailureClass::Transient
}

/// Whether the worker may retry the failure automatically.
pub(crate) fn is_transient(error: &Error) -> bool {
    classify(error) == FailureClass::Transient
}

/// Every [`DomainError`] in the chain, including one attached as an anyhow
/// context.
fn domain_errors(error: &Error) -> Vec<&DomainError> {
    let mut found = Vec::new();
    if let Some(domain) = error.downcast_ref::<DomainError>() {
        found.push(domain);
    }
    found.extend(
        error
            .chain()
            .filter_map(|cause| cause.downcast_ref::<DomainError>()),
    );
    found
}

/// The first explicit HTTP status carried by the chain.
fn http_status(causes: &[&(dyn std::error::Error + 'static)]) -> Option<u16> {
    for cause in causes {
        if let Some(error) = cause.downcast_ref::<reqwest::Error>()
            && let Some(status) = error.status()
        {
            return Some(status.as_u16());
        }
        if let Some(error) = cause.downcast_ref::<docling_convert::PdfConvertError>()
            && let Some(status) = error.status_code()
        {
            return Some(status);
        }
    }
    None
}

/// How one HTTP status is treated.
///
/// A request timeout, a too-early request, a rate limit, a conflict and every
/// server error may succeed later. Every other client error is a request the
/// server refused and stays refused.
fn status_class(status: u16) -> FailureClass {
    match status {
        408 | 425 | 429 | 409 => FailureClass::Transient,
        400..=499 => FailureClass::Deterministic,
        _ => FailureClass::Transient,
    }
}

/// How one filesystem failure is treated.
fn io_class(kind: io::ErrorKind) -> FailureClass {
    match kind {
        io::ErrorKind::NotFound
        | io::ErrorKind::PermissionDenied
        | io::ErrorKind::InvalidInput
        | io::ErrorKind::InvalidData => FailureClass::Deterministic,
        _ => FailureClass::Transient,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use super::{FailureClass, classify, is_transient, mark_deterministic};
    use crate::domain_errors::DomainError;
    use crate::retry::mark_retryable;

    fn assert_deterministic(error: Error) {
        assert_eq!(
            classify(&error),
            FailureClass::Deterministic,
            "expected a deterministic decision for: {error}"
        );
        assert!(!is_transient(&error));
    }

    fn assert_transient(error: Error) {
        assert_eq!(
            classify(&error),
            FailureClass::Transient,
            "expected a transient decision for: {error}"
        );
        assert!(is_transient(&error));
    }

    /// Every hard domain variant stops the automatic retry.
    #[test]
    fn hard_domain_variants_are_deterministic() {
        for error in [
            DomainError::invalid_argument("remote_url_blocked"),
            DomainError::unauthorized("missing credentials"),
            DomainError::forbidden("access denied"),
            DomainError::not_found("document 42 not found"),
            DomainError::payload_too_large("remote_file_too_large"),
            DomainError::unprocessable_entity("unsupported schema"),
        ] {
            assert_deterministic(error.into());
        }
    }

    /// The broad domain variants keep their transient default.
    #[test]
    fn broad_domain_variants_stay_transient() {
        for error in [
            DomainError::internal("chunk persistence failed"),
            DomainError::conflict("stale target"),
            DomainError::upstream_error("remote_http_status_503"),
            DomainError::upstream_timeout("remote_download_failed"),
            DomainError::rate_limited("slow down"),
            DomainError::unavailable("dependency down"),
        ] {
            assert_transient(error.into());
        }
    }

    /// A hard domain variant keeps its decision through an anyhow context.
    #[test]
    fn a_nested_domain_error_keeps_its_decision() {
        let error: Error = Error::from(DomainError::payload_too_large("remote_file_too_large"))
            .context("download stage");
        assert_deterministic(error);

        let error: Error = Error::from(DomainError::not_found("missing file"))
            .context("outer context")
            .context("inner context");
        assert_deterministic(error);
    }

    /// A domain error attached as an anyhow context is still typed.
    #[test]
    fn a_context_attached_domain_error_is_deterministic() {
        let error: Error = Error::from(std::io::Error::other("transport"))
            .context(DomainError::forbidden("blocked"));
        assert_deterministic(error);
    }

    /// The message never decides: an unknown failure that merely mentions a
    /// deterministic word stays transient.
    #[test]
    fn an_unknown_message_never_becomes_deterministic() {
        for message in [
            "the source is missing",
            "invalid response body",
            "file not found in the archive",
            "unsupported schema",
            "requires authentication",
        ] {
            assert_transient(anyhow::anyhow!("{message}"));
        }
    }

    /// An unknown failure stays transient, and so does an explicit retry
    /// marker.
    #[test]
    fn unknown_and_explicitly_retryable_failures_stay_transient() {
        assert_transient(anyhow::anyhow!("database connection reset"));
        assert_transient(mark_retryable(anyhow::anyhow!("explicit retry")));
    }

    /// The deterministic marker keeps the message and the typed cause.
    #[test]
    fn the_deterministic_marker_keeps_the_wire_identity() {
        let error = mark_deterministic(DomainError::upstream_error("remote_redirect_limit"));
        assert_eq!(error.to_string(), "remote_redirect_limit");
        assert!(
            crate::domain_errors::find_domain_error(&error).is_some(),
            "the marker must not hide the typed domain error"
        );
        assert_deterministic(error);
    }

    /// A filesystem failure that cannot fix itself is deterministic.
    #[test]
    fn filesystem_failures_follow_their_kind() {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidInput,
            std::io::ErrorKind::InvalidData,
        ] {
            assert_deterministic(Error::from(std::io::Error::new(kind, "io failure")));
        }
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::WouldBlock,
            std::io::ErrorKind::UnexpectedEof,
        ] {
            assert_transient(Error::from(std::io::Error::new(kind, "io failure")));
        }
        assert_transient(Error::from(std::io::Error::other("unknown io failure")));
    }

    /// A provider failure follows its own variant.
    #[test]
    fn provider_failures_follow_their_variant() {
        assert_deterministic(Error::from(
            docling_convert::PdfConvertError::validation_error("file", "unsupported"),
        ));
        assert_deterministic(Error::from(docling_convert::PdfConvertError::env_error(
            "DOCLING_URL",
            "not set",
        )));
        assert_transient(Error::from(
            docling_convert::PdfConvertError::operation_error("poll task", "unknown failure"),
        ));
        assert_transient(Error::from(docling_convert::PdfConvertError::api_error(
            None,
            "transport failed",
        )));
    }

    /// A parse error keeps its provenance erased, so it stays transient.
    ///
    /// The installed client maps every unreadable response body to a parse
    /// error: an interrupted body collection and an invalid answer arrive the
    /// same way. Guessing that such a failure is permanent would stop a
    /// submission that a later attempt can complete, so the ambiguous failure
    /// keeps the transient default.
    #[test]
    fn a_parse_error_stays_transient() {
        assert_transient(Error::from(docling_convert::PdfConvertError::parse_error(
            "Docling async submission response",
            "error decoding response body",
        )));
    }

    /// The explicit retry override wins over a hard domain variant.
    #[test]
    fn the_explicit_retry_override_wins_over_a_hard_domain_variant() {
        let error = mark_retryable(anyhow::Error::from(DomainError::payload_too_large(
            "remote_file_too_large",
        )));
        assert_transient(error);
    }

    /// A hard domain variant wins over a nested transient status.
    #[test]
    fn a_hard_domain_variant_wins_over_a_nested_transient_status() {
        // The request itself is wrong, so the failure stays deterministic even
        // though the chain carries a temporary provider status.
        let error: Error = Error::from(docling_convert::PdfConvertError::api_error(
            Some(503),
            "provider failed temporarily",
        ))
        .context("docling submit")
        .context(DomainError::payload_too_large("remote_file_too_large"));
        assert_deterministic(error);
    }

    /// A provider failure with an explicit status follows that status.
    #[test]
    fn provider_statuses_follow_the_status_matrix() {
        for status in [400u16, 401, 403, 404, 413, 422] {
            let error = Error::from(docling_convert::PdfConvertError::api_error(
                Some(status),
                "provider refused the request",
            ));
            assert_deterministic(error);
        }
        for status in [408u16, 409, 425, 429, 500, 503] {
            let error = Error::from(docling_convert::PdfConvertError::api_error(
                Some(status),
                "provider failed temporarily",
            ));
            assert_transient(error);
        }
    }

    /// A provider cause nested under an anyhow context keeps its status
    /// decision.
    #[test]
    fn a_nested_provider_cause_keeps_its_status_decision() {
        let refused: Error = Error::from(docling_convert::PdfConvertError::api_error(
            Some(404),
            "provider refused the request",
        ))
        .context("docling submit");
        assert_deterministic(refused);

        let temporary: Error = Error::from(docling_convert::PdfConvertError::api_error(
            Some(503),
            "provider failed temporarily",
        ))
        .context("docling submit");
        assert_transient(temporary);
    }
}
