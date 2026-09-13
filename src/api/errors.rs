pub(crate) use super::error_mapping::{
    admin_user_error_response, internal_error_response, library_management_error_response,
    source_management_error_response, task_error,
};

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    #[test]
    fn maps_embedding_upstream_failures() {
        assert_eq!(
            context69_http_support::runtime_aware_status(
                "embedding upstream transport error: operation=read response body kind=timeout"
            ),
            Some(StatusCode::GATEWAY_TIMEOUT)
        );
        assert_eq!(
            context69_http_support::runtime_aware_status(
                "embedding upstream transport error: operation=send request kind=connect"
            ),
            Some(StatusCode::BAD_GATEWAY)
        );
        assert_eq!(
            context69_http_support::runtime_aware_status(
                "embedding request failed: status=429 Too Many Requests"
            ),
            Some(StatusCode::TOO_MANY_REQUESTS)
        );
        assert_eq!(
            context69_http_support::runtime_aware_status(
                "embedding request failed: status=401 Unauthorized"
            ),
            Some(StatusCode::BAD_GATEWAY)
        );
    }

    #[test]
    fn maps_non_failed_retry_to_conflict() {
        assert_eq!(
            super::library_management_error_response(anyhow::anyhow!(
                "file id is not failed and cannot be retried"
            ))
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
}
