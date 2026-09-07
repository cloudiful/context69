use super::LibraryDependency;

pub(super) fn dependency_is_transient(
    dependency: LibraryDependency,
    error: &anyhow::Error,
) -> bool {
    let message = error_chain_message(error);
    if message.contains("dimension mismatch")
        || message.contains("embedding count does not match")
        || message.contains("validation")
        || message.contains("unsupported")
    {
        return false;
    }

    match dependency.canonical() {
        LibraryDependency::S3 => is_s3_transient_error(error),
        LibraryDependency::Docling => {
            message.contains("timeout")
                || message.contains("timed out")
                || message.contains("connect")
                || message.contains("connection")
                || message.contains("transport")
                || message.contains("http 429")
                || message.contains("http 5")
                || message.contains("status 429")
                || message.contains("status 5")
                || status_is_too_many_requests(&message)
                || status_is_server_error(&message)
                || message.contains("temporar")
                // DNS / name-resolution failures must stay transient with
                // backoff (issue #176). They carry no HTTP status and must
                // never be mistaken for `configuration:` auth errors.
                || message.contains("dns")
                || message.contains("resolve")
                || message.contains("getaddrinfo")
                || message.contains("name or service")
                || message.contains("no such host")
                || message.contains("network")
                || message.contains("unreachable")
                || message.contains("eai_")
        }
        LibraryDependency::Embedding => is_embedding_transient(&message),
        LibraryDependency::Qdrant => is_qdrant_transient(&message),
        LibraryDependency::EmbeddingVector => is_embedding_transient(&message),
    }
}

fn is_embedding_transient(message: &str) -> bool {
    message.contains("embedding upstream transport error")
        || message.contains("runtime is unavailable")
        || message.contains("runtime unavailable")
        || (message.contains("embedding request failed")
            && (status_is_too_many_requests(message) || status_is_server_error(message)))
}

fn is_qdrant_transient(message: &str) -> bool {
    // Runtime unavailable is a shared degraded state that should trip both
    // gates when the vector runtime cannot be built, even if the message
    // does not yet carry a qdrant substring.
    if message.contains("runtime is unavailable") || message.contains("runtime unavailable") {
        return true;
    }
    if !message.contains("qdrant") {
        return false;
    }
    message.contains("connect")
        || message.contains("connection")
        || message.contains("timeout")
        || message.contains("timed out")
        || message.contains("transport")
        || status_is_too_many_requests(message)
        || status_is_server_error(message)
}

pub(super) fn is_configuration_error(error: &anyhow::Error) -> bool {
    let message = error_chain_message(error);
    message.contains("not configured")
        || message.contains("missing configuration")
        || message.contains("configuration error")
        || message.contains("configuration:")
        || message.contains("optional vlm runtime config is incomplete")
        || message.contains("configinvalid")
        || message.contains("kind=configinvalid")
        || message.contains("permissiondenied")
        || message.contains("access denied")
        || message.contains("authentication")
        || message.contains("unauthorized")
        || message.contains("forbidden")
        || message.contains("permission denied")
        || status_is_authentication_error(&message)
}

pub(super) fn is_s3_error(error: &anyhow::Error) -> bool {
    let message = error_chain_message(error);
    message.contains("s3 dependency unavailable")
        || message.contains("s3 operation")
        || message.contains("s3 connection")
}

pub(super) fn is_s3_transient_error(error: &anyhow::Error) -> bool {
    let message = error_chain_message(error);
    if is_configuration_error(error) || s3_error_is_permanent(error) {
        return false;
    }

    s3_error_is_transport_failure(error)
        || message.contains("timeout")
        || message.contains("timed out")
        || message.contains("s3 dependency unavailable: state=")
        || message.contains("connect")
        || message.contains("connection")
        || message.contains("transport")
        || message.contains("network")
        || message.contains("kind=ratelimited")
        || status_is_too_many_requests(&message)
        || status_is_server_error(&message)
}

pub(super) fn is_s3_attempt_retryable(error: &opendal::Error) -> bool {
    if s3_opendal_error_is_permanent(error) {
        return false;
    }

    let message = error.to_string().to_ascii_lowercase();
    match error.kind() {
        opendal::ErrorKind::RateLimited => true,
        opendal::ErrorKind::Unexpected => {
            error.is_temporary()
                || contains_transport_signal(&message)
                || status_is_too_many_requests(&message)
                || status_is_server_error(&message)
        }
        _ => false,
    }
}

fn s3_error_is_permanent(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<opendal::Error>()
            .is_some_and(s3_opendal_error_is_permanent)
    }) || {
        let message = error_chain_message(error);
        message.contains("kind=notfound")
            || message.contains("kind=permissiondenied")
            || message.contains("kind=unsupported")
            || message.contains("kind=configinvalid")
            || message.contains("kind=alreadyexists")
            || message.contains("kind=conditionnotmatch")
            || message.contains("authentication")
            || message.contains("unauthorized")
            || message.contains("forbidden")
            || message.contains("permission denied")
            || message.contains("not found")
            || status_is_client_error(&message)
    }
}

fn s3_opendal_error_is_permanent(error: &opendal::Error) -> bool {
    matches!(
        error.kind(),
        opendal::ErrorKind::NotFound
            | opendal::ErrorKind::PermissionDenied
            | opendal::ErrorKind::Unsupported
            | opendal::ErrorKind::ConfigInvalid
            | opendal::ErrorKind::AlreadyExists
            | opendal::ErrorKind::ConditionNotMatch
            | opendal::ErrorKind::IsADirectory
            | opendal::ErrorKind::NotADirectory
            | opendal::ErrorKind::IsSameFile
            | opendal::ErrorKind::RangeNotSatisfied
    )
}

fn error_chain_message(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" | ")
        .to_ascii_lowercase()
}

fn s3_error_is_transport_failure(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if let Some(opendal_error) = cause.downcast_ref::<opendal::Error>() {
            return is_s3_attempt_retryable(opendal_error);
        }
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_error| {
                matches!(
                    io_error.kind(),
                    std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::NotConnected
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::UnexpectedEof
                )
            })
    })
}

fn contains_transport_signal(message: &str) -> bool {
    message.contains("timeout")
        || message.contains("timed out")
        || message.contains("connect")
        || message.contains("connection")
        || message.contains("transport")
        || message.contains("network")
}

/// Split on non-alphanumeric boundaries so hex UUID fragments such as
/// `401de82e` stay a single token and never match a standalone `401`/`403`.
/// The previous `!is_ascii_digit` split treated the `d` in `401de82e` as a
/// delimiter and misclassified DNS errors carrying a UUID as auth failures
/// (issue #176). Ports such as `:5001` stay a single 4-digit token and are
/// excluded by the length checks below.
fn split_status_tokens(message: &str) -> impl Iterator<Item = &str> {
    message.split(|character: char| !character.is_ascii_alphanumeric())
}

fn is_three_digit_code(part: &str) -> bool {
    part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit())
}

fn status_is_too_many_requests(message: &str) -> bool {
    split_status_tokens(message).any(|part| part == "429")
}

fn status_is_authentication_error(message: &str) -> bool {
    split_status_tokens(message).any(|part| part == "401" || part == "403")
}

fn status_is_client_error(message: &str) -> bool {
    split_status_tokens(message)
        .filter(|part| is_three_digit_code(part) && *part != "429")
        .any(|part| part.starts_with('4'))
}

fn status_is_server_error(message: &str) -> bool {
    split_status_tokens(message)
        .filter(|part| is_three_digit_code(part))
        .any(|part| part.starts_with('5'))
}

pub(super) fn redact_dependency_error(error: &anyhow::Error) -> String {
    let message = error.to_string();
    if message.chars().count() <= 1000 {
        message
    } else {
        format!("{}...", message.chars().take(1000).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::{
        dependency_is_transient, is_configuration_error, is_s3_attempt_retryable,
        is_s3_transient_error, s3_error_is_permanent,
    };
    use crate::services::library::LibraryDependency;

    #[test]
    fn classifies_transport_failures_as_transient() {
        for message in [
            "connection refused",
            "network is unreachable",
            "service unavailable (status 503)",
            "request timed out",
            "status 429",
            "status 503",
            "kind=Unexpected: upstream reset the connection",
            "s3 dependency unavailable: state=open",
        ] {
            assert!(
                is_s3_transient_error(&anyhow!(message)),
                "expected transient classification for {message}"
            );
        }
    }

    #[test]
    fn does_not_classify_permanent_storage_errors_as_transient() {
        for message in [
            "kind=NotFound: object does not exist",
            "kind=PermissionDenied: access denied",
            "kind=Unsupported: operation is unsupported",
            "kind=ConfigInvalid: endpoint is missing",
            "kind=AlreadyExists: object already exists",
            "kind=ConditionNotMatch: precondition failed",
            "status 401 unauthorized",
            "status 403 forbidden",
            "status 404 not found",
            "status 409 conflict",
            "authentication failed",
        ] {
            let error = anyhow!(message);
            assert!(
                s3_error_is_permanent(&error),
                "expected permanent error: {message}"
            );
            assert!(!is_s3_transient_error(&error));
        }
    }

    #[test]
    fn classifies_explicit_opendal_transport_error_as_transient() {
        let error = opendal::Error::new(
            opendal::ErrorKind::Unexpected,
            "upstream reset the connection",
        );

        assert!(is_s3_attempt_retryable(&error));
        assert!(is_s3_transient_error(&anyhow::Error::new(error)));
    }

    #[test]
    fn preserves_temporary_opendal_causes_through_s3_context() {
        let cause =
            opendal::Error::new(opendal::ErrorKind::Unexpected, "upstream error").set_temporary();
        let error = anyhow::Error::new(cause).context("s3 dependency unavailable");

        assert!(is_s3_transient_error(&error));
    }

    #[test]
    fn does_not_retry_unclassified_opendal_unexpected_error() {
        let error = opendal::Error::new(opendal::ErrorKind::Unexpected, "request failed");

        assert!(!is_s3_attempt_retryable(&error));
        assert!(!is_s3_transient_error(&anyhow::Error::new(error)));
    }

    #[test]
    fn retries_temporary_opendal_errors_but_not_permanent_kinds() {
        let temporary =
            opendal::Error::new(opendal::ErrorKind::Unexpected, "upstream error").set_temporary();
        let already_exists =
            opendal::Error::new(opendal::ErrorKind::AlreadyExists, "object already exists");
        let condition_not_match =
            opendal::Error::new(opendal::ErrorKind::ConditionNotMatch, "precondition failed");

        assert!(is_s3_attempt_retryable(&temporary));
        assert!(!is_s3_attempt_retryable(&already_exists));
        assert!(!is_s3_attempt_retryable(&condition_not_match));
    }

    #[test]
    fn classifies_transient_qdrant_cause_through_context() {
        let error = anyhow!("status 503 service unavailable")
            .context("qdrant points upsert request failed");

        assert!(dependency_is_transient(LibraryDependency::Qdrant, &error));
        // Legacy alias must still be handled but via embedding for backward
        // compat; qdrant errors should NOT be transient for the legacy alias
        // after the split, proving the gate separation.
        assert!(!dependency_is_transient(
            LibraryDependency::EmbeddingVector,
            &anyhow!("status 503 service unavailable")
                .context("qdrant points upsert request failed")
        ));
        assert!(dependency_is_transient(
            LibraryDependency::Embedding,
            &anyhow!("embedding upstream transport error: operation=send request kind=connect")
        ));
    }

    #[test]
    fn keeps_vector_dimension_errors_permanent_through_context() {
        let error = anyhow!("embedding dimension mismatch: expected 1536, got 768")
            .context("qdrant points upsert request failed");

        assert!(!dependency_is_transient(LibraryDependency::Qdrant, &error));
        assert!(!dependency_is_transient(
            LibraryDependency::Embedding,
            &error
        ));
        assert!(!dependency_is_transient(
            LibraryDependency::EmbeddingVector,
            &error
        ));
    }

    #[test]
    fn treats_unavailable_configured_vector_runtime_as_transient() {
        assert!(dependency_is_transient(
            LibraryDependency::Embedding,
            &anyhow!("embedding/vector runtime is unavailable")
        ));
        assert!(dependency_is_transient(
            LibraryDependency::Qdrant,
            &anyhow!("embedding/vector runtime is unavailable")
        ));
        assert!(dependency_is_transient(
            LibraryDependency::EmbeddingVector,
            &anyhow!("embedding/vector runtime is unavailable")
        ));
        assert!(!dependency_is_transient(
            LibraryDependency::Embedding,
            &anyhow!("embedding/vector runtime is not configured")
        ));
        assert!(!dependency_is_transient(
            LibraryDependency::Qdrant,
            &anyhow!("embedding/vector runtime is not configured")
        ));
    }

    #[test]
    fn classifies_storage_configuration_failures_as_configuration_errors() {
        for message in [
            "kind=ConfigInvalid: endpoint is missing",
            "kind=PermissionDenied: access denied",
            "status 403 forbidden",
            "embedding request failed: status=401",
        ] {
            assert!(
                is_configuration_error(&anyhow!(message)),
                "expected configuration classification for {message}"
            );
        }
    }

    #[test]
    fn classifies_incomplete_vlm_runtime_config_as_configuration_error() {
        let error = anyhow!(
            "Validation error for 'VLM runtime': optional VLM runtime config is incomplete; \
             provide all of: OPENAI_BASE_URL, VLM_PIPELINE_MODEL, PICTURE_DESCRIPTION_MODEL, \
             CODE_FORMULA_MODEL, OPENAI_API_KEY, or leave all unset"
        );
        assert!(is_configuration_error(&error));
        assert!(!dependency_is_transient(LibraryDependency::Docling, &error));
    }

    /// `qdrant library file cleanup request failed` is the exact context
    /// string produced by `QdrantIndex::delete_points_for_library_file` when
    /// the gRPC call errors. After phase 1 these must route to qdrant, not
    /// embedding_vector. The alias `embedding_vector` is kept for backward
    /// reads but must NOT be used for new qdrant failures.
    #[test]
    fn classifies_qdrant_cleanup_transport_error_as_qdrant_transient() {
        for message in [
            "transport error: connection refused",
            "transport error: connection reset",
            "connection refused",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                dependency_is_transient(LibraryDependency::Qdrant, &error),
                "expected transient classification for {message}"
            );
            assert!(
                !dependency_is_transient(LibraryDependency::Embedding, &error),
                "qdrant transport must not be embedding transient for {message}"
            );
        }
    }

    /// Pin the current behavior that "network is unreachable" alone (without
    /// "transport"/"connection"/"timeout" signals) does NOT trigger the
    /// qdrant transient classifier.
    #[test]
    fn classifies_qdrant_cleanup_unreachable_alone_as_qdrant_non_transient() {
        let error =
            anyhow!("network is unreachable").context("qdrant library file cleanup request failed");
        assert!(
            !dependency_is_transient(LibraryDependency::Qdrant, &error),
            "network-only signal should not be transient under current classifier"
        );
        assert!(
            !dependency_is_transient(LibraryDependency::Embedding, &error),
            "network-only should not be embedding transient"
        );
    }

    #[test]
    fn classifies_qdrant_cleanup_timeout_as_qdrant_transient() {
        for message in [
            "request timed out after 30s",
            "qdrant library file cleanup request timed out after 30s",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                dependency_is_transient(LibraryDependency::Qdrant, &error),
                "expected transient classification for {message}"
            );
        }
    }

    #[test]
    fn classifies_qdrant_cleanup_server_status_as_qdrant_transient() {
        for message in [
            "status 503 service unavailable",
            "status 502 bad gateway",
            "status 429 too many requests",
            "kind=Unexpected: status=503",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                dependency_is_transient(LibraryDependency::Qdrant, &error),
                "expected transient classification for {message}"
            );
            assert!(
                !dependency_is_transient(LibraryDependency::Embedding, &error),
                "qdrant server status must not be embedding transient"
            );
        }
    }

    /// Permanent (client-side) qdrant errors are not retried by the
    /// transient gate.
    #[test]
    fn classifies_qdrant_cleanup_permanent_error_as_qdrant_non_transient() {
        for message in [
            "validation error: filter format is invalid",
            "unsupported point id variant",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                !dependency_is_transient(LibraryDependency::Qdrant, &error),
                "expected permanent classification for {message}"
            );
        }
    }

    #[test]
    fn embedding_transient_does_not_trigger_on_qdrant_errors() {
        for message in [
            "transport error: connection refused",
            "status 503 service unavailable",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                !dependency_is_transient(LibraryDependency::Embedding, &error),
                "qdrant error must not be embedding transient for {message}"
            );
        }
    }

    #[test]
    fn qdrant_transient_does_not_trigger_on_embedding_errors() {
        let error =
            anyhow!("embedding upstream transport error: operation=send request kind=connect");
        assert!(dependency_is_transient(
            LibraryDependency::Embedding,
            &error
        ));
        assert!(!dependency_is_transient(LibraryDependency::Qdrant, &error));
    }

    #[test]
    fn does_not_make_unknown_errors_transient() {
        for (dependency, message) in [
            (LibraryDependency::Embedding, "some random failure"),
            (LibraryDependency::Qdrant, "some random failure"),
            (
                LibraryDependency::Qdrant,
                "qdrant points delete request failed: unknown issue",
            ),
        ] {
            assert!(
                !dependency_is_transient(dependency, &anyhow!(message)),
                "unknown error must not be transient for {:?}: {message}",
                dependency
            );
        }
    }

    #[test]
    fn legacy_embedding_vector_alias_still_routes_embedding_transient() {
        let error =
            anyhow!("embedding upstream transport error: operation=send request kind=timeout");
        assert!(dependency_is_transient(
            LibraryDependency::EmbeddingVector,
            &error
        ));
        assert!(dependency_is_transient(
            LibraryDependency::Embedding,
            &error
        ));
        assert!(!dependency_is_transient(LibraryDependency::Qdrant, &error));
    }

    /// Issue #176: UUID hex fragments such as `401de82e` must not match a
    /// standalone HTTP 401/403. The failing production message carried both
    /// a UUID and a trailing `dns` signal and was misrouted to
    /// `configuration:` instead of transient.
    #[test]
    fn uuid_containing_401_is_not_an_authentication_error() {
        for message in [
            "failed to poll docling task 401de82e-e717-4f6a-9c2a-9b1a2c3d4e5f: dns error",
            "docling task 401de82e failed: temporary failure in name resolution",
            "request 0193f6c5-1234-7890-abcd-1234567890ab failed: dns",
        ] {
            let error = anyhow!(message);
            assert!(
                !is_configuration_error(&error),
                "UUID message must not be configuration: {message}"
            );
        }
        // Docling DNS errors with a UUID must stay transient with backoff.
        for message in [
            "failed to poll docling task 401de82e-e717-4f6a-9c2a-9b1a2c3d4e5f: dns error",
            "failed to poll docling task 0193f6c5: temporary failure in name resolution",
            "failed to resolve host for docling base url: no such host",
            "dns error while polling docling task abc123",
            "getaddrinfo failed for docling host: name or service not known",
        ] {
            assert!(
                dependency_is_transient(LibraryDependency::Docling, &anyhow!(message)),
                "DNS message must be transient: {message}"
            );
        }
    }

    /// Issue #176: a Docling base URL carrying port `:5001` must not match a
    /// standalone 5xx. The port token is 4 digits and is excluded by the
    /// length check; only standalone 3-digit 5xx counts.
    #[test]
    fn url_with_port_5001_is_not_a_server_error() {
        for message in [
            "failed to poll docling task abc: http://192.168.67.31:5001/v1/status/poll",
            "docling base url http://192.168.67.31:5001 is unreachable via dns",
        ] {
            // Port alone must not flip the server-error classifier; the
            // second message is transient only because of the dns/unreachable
            // signals, not because of the port number.
            let error = anyhow!(message);
            if message.contains("dns") || message.contains("unreachable") {
                assert!(dependency_is_transient(LibraryDependency::Docling, &error));
            }
            assert!(
                !is_configuration_error(&error),
                "port 5001 must not be configuration: {message}"
            );
        }
        // Standalone 5xx/429 still count as transient.
        for message in [
            "docling poll failed: status 503 service unavailable",
            "docling poll failed: HTTP 500 internal error",
            "docling poll failed: status 429 too many requests",
        ] {
            assert!(
                dependency_is_transient(LibraryDependency::Docling, &anyhow!(message)),
                "expected transient for {message}"
            );
        }
    }

    /// Issue #176: true HTTP 401/403 must stay on the configuration latch and
    /// never be treated as transient, even for Docling.
    #[test]
    fn true_401_and_403_stay_on_configuration_latch() {
        for message in [
            "docling poll failed: HTTP 401 unauthorized",
            "docling poll failed: status 401 unauthorized",
            "docling poll failed: status=403 forbidden",
            "docling poll failed: HTTP 403: forbidden",
            "embedding request failed: status=401",
        ] {
            let error = anyhow!(message);
            assert!(
                is_configuration_error(&error),
                "expected configuration for {message}"
            );
            assert!(
                !dependency_is_transient(LibraryDependency::Docling, &error),
                "auth must never be transient: {message}"
            );
        }
    }
}
