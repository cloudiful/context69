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

    match dependency {
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
        }
        LibraryDependency::EmbeddingVector => {
            if message.contains("qdrant") {
                return message.contains("connect")
                    || message.contains("connection")
                    || message.contains("timeout")
                    || message.contains("timed out")
                    || message.contains("transport")
                    || status_is_too_many_requests(&message)
                    || status_is_server_error(&message);
            }
            message.contains("embedding upstream transport error")
                || message.contains("runtime is unavailable")
                || message.contains("runtime unavailable")
                || (message.contains("embedding request failed")
                    && (status_is_too_many_requests(&message) || status_is_server_error(&message)))
        }
    }
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

fn status_is_too_many_requests(message: &str) -> bool {
    message
        .split(|character: char| !character.is_ascii_digit())
        .any(|part| part == "429")
}

fn status_is_authentication_error(message: &str) -> bool {
    message
        .split(|character: char| !character.is_ascii_digit())
        .any(|part| part == "401" || part == "403")
}

fn status_is_client_error(message: &str) -> bool {
    message
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| part.len() == 3 && *part != "429")
        .any(|part| part.starts_with('4'))
}

fn status_is_server_error(message: &str) -> bool {
    message
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| part.len() == 3)
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

        assert!(dependency_is_transient(
            LibraryDependency::EmbeddingVector,
            &error
        ));
    }

    #[test]
    fn keeps_vector_dimension_errors_permanent_through_context() {
        let error = anyhow!("embedding dimension mismatch: expected 1536, got 768")
            .context("qdrant points upsert request failed");

        assert!(!dependency_is_transient(
            LibraryDependency::EmbeddingVector,
            &error
        ));
    }

    #[test]
    fn treats_unavailable_configured_vector_runtime_as_transient() {
        assert!(dependency_is_transient(
            LibraryDependency::EmbeddingVector,
            &anyhow!("embedding/vector runtime is unavailable")
        ));
        assert!(!dependency_is_transient(
            LibraryDependency::EmbeddingVector,
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
    /// the gRPC call errors. These tests pin the current classification for
    /// representative transport, server-status, and timeout errors so the
    /// phase 0 reproduction cannot drift, and so the eventual split between
    /// embedding and qdrant gates can change this behavior with confidence.
    #[test]
    fn classifies_qdrant_cleanup_transport_error_as_embedding_transient() {
        for message in [
            "transport error: connection refused",
            "transport error: connection reset",
            "connection refused",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                dependency_is_transient(LibraryDependency::EmbeddingVector, &error),
                "expected transient classification for {message}"
            );
        }
    }

    /// Pin the current behavior that "network is unreachable" alone (without
    /// "transport"/"connection"/"timeout" signals) does NOT trigger the
    /// qdrant transient classifier. The phase 0 reproduction deliberately
    /// avoids guessing at the gRPC error format we have not observed, so this
    /// test documents the boundary so the eventual split can change it.
    #[test]
    fn classifies_qdrant_cleanup_unreachable_alone_as_embedding_non_transient() {
        let error =
            anyhow!("network is unreachable").context("qdrant library file cleanup request failed");
        assert!(
            !dependency_is_transient(LibraryDependency::EmbeddingVector, &error),
            "network-only signal should not be transient under current classifier"
        );
    }

    #[test]
    fn classifies_qdrant_cleanup_timeout_as_embedding_transient() {
        for message in [
            "request timed out after 30s",
            "qdrant library file cleanup request timed out after 30s",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                dependency_is_transient(LibraryDependency::EmbeddingVector, &error),
                "expected transient classification for {message}"
            );
        }
    }

    #[test]
    fn classifies_qdrant_cleanup_server_status_as_embedding_transient() {
        for message in [
            "status 503 service unavailable",
            "status 502 bad gateway",
            "status 429 too many requests",
            "kind=Unexpected: status=503",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                dependency_is_transient(LibraryDependency::EmbeddingVector, &error),
                "expected transient classification for {message}"
            );
        }
    }

    /// Permanent (client-side) qdrant errors are not retried by the
    /// transient gate. The test uses only the publicly known contract of
    /// `delete_points_for_library_file`; it does not invent raw server error
    /// details we have not observed.
    #[test]
    fn classifies_qdrant_cleanup_permanent_error_as_embedding_non_transient() {
        for message in [
            "validation error: filter format is invalid",
            "unsupported point id variant",
        ] {
            let error = anyhow!(message).context("qdrant library file cleanup request failed");
            assert!(
                !dependency_is_transient(LibraryDependency::EmbeddingVector, &error),
                "expected permanent classification for {message}"
            );
        }
    }
}
