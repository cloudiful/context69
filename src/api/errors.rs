pub(crate) use super::error_mapping::{
    admin_user_error_response, internal_error_response, library_management_error_response,
    source_management_error_response, task_error,
};

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::domain_errors::DomainError;

    #[test]
    fn maps_embedding_upstream_failures() {
        assert_eq!(
            context69_http_support::typed_status_for_error(
                &DomainError::upstream_timeout(
                    "embedding upstream transport error: operation=read response body kind=timeout"
                )
                .into()
            ),
            Some(StatusCode::GATEWAY_TIMEOUT)
        );
        assert_eq!(
            context69_http_support::typed_status_for_error(
                &DomainError::upstream_error(
                    "embedding upstream transport error: operation=send request kind=connect"
                )
                .into()
            ),
            Some(StatusCode::BAD_GATEWAY)
        );
        assert_eq!(
            context69_http_support::typed_status_for_error(
                &DomainError::rate_limited(
                    "embedding request failed: status=429 Too Many Requests"
                )
                .into()
            ),
            Some(StatusCode::TOO_MANY_REQUESTS)
        );
        assert_eq!(
            context69_http_support::typed_status_for_error(
                &DomainError::upstream_error("embedding request failed: status=401 Unauthorized")
                    .into()
            ),
            Some(StatusCode::BAD_GATEWAY)
        );
        let plain = anyhow::anyhow!("plain internal boom");
        assert_eq!(context69_http_support::typed_status_for_error(&plain), None);
    }

    #[test]
    fn maps_non_failed_retry_to_conflict() {
        assert_eq!(
            super::library_management_error_response(
                DomainError::conflict("file id is not failed and cannot be retried").into()
            )
            .status(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn typed_error_code_matches_status() {
        use context69_contracts::ApiErrorCode;
        for (status, code) in [
            (StatusCode::BAD_REQUEST, ApiErrorCode::InvalidArgument),
            (StatusCode::NOT_FOUND, ApiErrorCode::NotFound),
            (StatusCode::CONFLICT, ApiErrorCode::Conflict),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                ApiErrorCode::UnprocessableEntity,
            ),
            (StatusCode::PAYLOAD_TOO_LARGE, ApiErrorCode::PayloadTooLarge),
            (StatusCode::SERVICE_UNAVAILABLE, ApiErrorCode::Unavailable),
            (StatusCode::BAD_GATEWAY, ApiErrorCode::UpstreamError),
            (StatusCode::GATEWAY_TIMEOUT, ApiErrorCode::UpstreamTimeout),
        ] {
            let body = super::super::error_mapping::error_response(status, "boom".to_string());
            assert_eq!(body.code, code.as_str());
            assert_eq!(body.code.parse::<ApiErrorCode>().expect("parse"), code);
        }
    }

    #[test]
    fn unknown_errors_map_to_internal_without_substring() {
        assert_eq!(
            super::internal_error_response(anyhow::anyhow!("plain internal boom")).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            super::library_management_error_response(anyhow::anyhow!("plain internal boom"))
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
