use anyhow::Error;
use axum::{
    Json,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use context69_contracts::ApiErrorResponse;

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct CurrentUser(pub AuthenticatedUser);

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .map(Self)
            .ok_or_else(|| json_error_response(StatusCode::UNAUTHORIZED, "missing bearer token"))
    }
}

pub fn json_error_response(status: StatusCode, error: impl Into<String>) -> Response {
    let code = ApiErrorResponse::code_for_status(status.as_u16());
    (status, Json(ApiErrorResponse::new(code, error.into()))).into_response()
}

pub fn internal_error_response(error: Error) -> Response {
    let message = error.to_string();
    let status = if message.contains("page must be")
        || message.contains("page_size must be")
        || message.contains("page offset is too large")
        || message.contains("limit must be between 1 and 100")
    {
        StatusCode::BAD_REQUEST
    } else {
        runtime_aware_status(&message).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    };
    json_error_response(status, message)
}

pub fn map_settings_error(error: Error) -> Response {
    let message = error.to_string();
    let status = if let Some(status) = runtime_aware_status(&message) {
        status
    } else if message.contains("already running") {
        StatusCode::CONFLICT
    } else if message.contains("must not be empty")
        || message.contains("must be greater than 0")
        || message.contains("must be one of")
        || message.contains("is required when")
        || message.contains("invalid runtime.scheduler.valkey_url")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    json_error_response(status, message)
}

pub fn runtime_aware_status(message: &str) -> Option<StatusCode> {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("s3 dependency unavailable")
        || normalized.contains("docling dependency unavailable")
        || normalized.contains("embedding/vector dependency unavailable")
        || normalized.contains("library dependency unavailable")
        || normalized.contains("runtime is not configured")
        || normalized.contains("docling is not configured")
        || message.contains("save runtime settings and restart the service")
    {
        Some(StatusCode::SERVICE_UNAVAILABLE)
    } else if normalized.contains("embedding upstream transport error") {
        if normalized.contains("kind=timeout") || normalized.contains("timed out") {
            Some(StatusCode::GATEWAY_TIMEOUT)
        } else {
            Some(StatusCode::BAD_GATEWAY)
        }
    } else if normalized.contains("embedding request failed: status=429") {
        Some(StatusCode::TOO_MANY_REQUESTS)
    } else if normalized.contains("qdrant")
        && (normalized.contains("timeout") || normalized.contains("timed out"))
    {
        Some(StatusCode::GATEWAY_TIMEOUT)
    } else if normalized.contains("qdrant") && normalized.contains("429") {
        Some(StatusCode::TOO_MANY_REQUESTS)
    } else if (normalized.contains("qdrant")
        && (normalized.contains("transport")
            || normalized.contains("connect")
            || normalized.contains("connection")))
        || normalized.contains("embedding request failed:")
        || normalized.contains("failed to parse embedding response:")
    {
        Some(StatusCode::BAD_GATEWAY)
    } else {
        None
    }
}

pub fn error_code_for_status(status: StatusCode) -> context69_contracts::ApiErrorCode {
    context69_contracts::ApiErrorCode::code_for_status(status.as_u16())
}

pub fn status_for_code(code: context69_contracts::ApiErrorCode) -> StatusCode {
    StatusCode::from_u16(code.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

pub fn json_error_for_code(
    code: context69_contracts::ApiErrorCode,
    message: impl Into<String>,
) -> Response {
    let status = status_for_code(code);
    let body = context69_contracts::CanonicalApiErrorResponse::new(code, message.into());
    let legacy: ApiErrorResponse = body.into();
    (status, Json(legacy)).into_response()
}

pub fn validate_canonical_offset(page: u32, page_size: u32) -> anyhow::Result<()> {
    context69_contracts::OffsetPageQuery { page, page_size }.validate()
}

pub fn validate_cursor_limit(limit: u32) -> anyhow::Result<()> {
    context69_contracts::CursorPageQuery {
        limit,
        cursor: None,
    }
    .validate()
}

pub fn is_search_validation_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("cursor")
        || lower.contains("page_size must be between 1 and 100")
        || lower.contains("page must be greater than 0")
        || lower.contains("page must be between 1 and 10000")
        || lower.contains("limit must be between 1 and 100")
        || lower.contains("page is too large")
        || lower.contains("search result limit is too large")
        || (lower.contains("page > 1") && lower.contains("sort=date"))
        || (lower.contains("query text is required") && lower.contains("sort=date"))
        || lower.contains("query must not be blank")
}

pub fn is_not_found_message(message: &str) -> bool {
    message.to_ascii_lowercase().contains("not found")
}

pub fn map_search_service_error(error: Error) -> Response {
    let message = error.to_string();
    if is_search_validation_message(&message) {
        json_error_for_code(context69_contracts::ApiErrorCode::InvalidArgument, message)
    } else {
        internal_error_response(error)
    }
}

pub fn map_document_lookup_error(error: Error) -> Response {
    let message = error.to_string();
    if is_not_found_message(&message) {
        json_error_for_code(context69_contracts::ApiErrorCode::NotFound, message)
    } else {
        map_search_service_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_offset_bounds_match_shared_kernel() {
        assert!(validate_canonical_offset(1, 50).is_ok());
        assert!(validate_canonical_offset(0, 50).is_err());
        assert!(validate_canonical_offset(1, 0).is_err());
        assert!(validate_canonical_offset(10_001, 50).is_err());
        assert!(validate_canonical_offset(1, 101).is_err());
    }

    #[test]
    fn cursor_limit_bounds_match_shared_kernel() {
        assert!(validate_cursor_limit(1).is_ok());
        assert!(validate_cursor_limit(50).is_ok());
        assert!(validate_cursor_limit(0).is_err());
        assert!(validate_cursor_limit(101).is_err());
    }

    #[test]
    fn error_code_status_round_trip() {
        use context69_contracts::ApiErrorCode;
        for code in [
            ApiErrorCode::InvalidArgument,
            ApiErrorCode::Unauthorized,
            ApiErrorCode::Forbidden,
            ApiErrorCode::NotFound,
            ApiErrorCode::Conflict,
            ApiErrorCode::PayloadTooLarge,
            ApiErrorCode::UnprocessableEntity,
            ApiErrorCode::RateLimited,
            ApiErrorCode::UpstreamError,
            ApiErrorCode::Unavailable,
            ApiErrorCode::UpstreamTimeout,
            ApiErrorCode::Internal,
        ] {
            let status = status_for_code(code);
            assert_eq!(status.as_u16(), code.status_code());
            assert_eq!(error_code_for_status(status), code);
        }
    }

    #[test]
    fn runtime_aware_maps_dependencies_and_upstreams() {
        assert_eq!(
            runtime_aware_status("s3 dependency unavailable").map(|s| s.as_u16()),
            Some(503)
        );
        assert_eq!(
            runtime_aware_status("embedding upstream transport error: kind=timeout")
                .map(|s| s.as_u16()),
            Some(504)
        );
        assert_eq!(
            runtime_aware_status("embedding request failed: status=429").map(|s| s.as_u16()),
            Some(429)
        );
        assert!(runtime_aware_status("plain internal boom").is_none());
    }

    #[test]
    fn search_validation_messages_map_to_invalid_argument() {
        assert!(is_search_validation_message(
            "page_size must be between 1 and 100"
        ));
        assert!(is_search_validation_message("malformed cursor payload"));
        assert!(is_search_validation_message("query must not be blank"));
        assert!(!is_search_validation_message("plain internal boom"));
        let response = map_search_service_error(anyhow::anyhow!("page must be greater than 0"));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
