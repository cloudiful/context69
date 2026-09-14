use anyhow::Error;
use axum::{
    Json,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use context69_contracts::{ApiErrorCode, ApiErrorResponse};

pub use context69_contracts::DomainError;

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

/// Canonical typed domain error taxonomy shared by HTTP mappers and service
/// producers. The enum itself lives in `context69-contracts` so the
/// independent `context69-search` and `context69-translation` crates can
/// construct it without depending on HTTP types; this crate re-exports it
/// and owns the HTTP status mapping. Mappers must classify via
/// [`find_domain_error`] downcast through `anyhow` chains and never via
/// message substrings. Failures without a typed category map to `internal`.
///
/// HTTP status for one typed [`DomainError`].
pub fn domain_status(error: &DomainError) -> StatusCode {
    status_for_code(error.code())
}

/// Find the first typed [`DomainError`] in an `anyhow` error, including when
/// the typed variant is used as an outer `context()` layer.
///
/// `anyhow`'s `chain()` does not surface a `DomainError` supplied as outer
/// context (e.g. `Err(plain).context(DomainError)` or
/// `None::<()>.context(DomainError)`), while `Error::downcast_ref` does.
/// Check the outer value first, then fall back to the chain so custom source
/// wrappers (e.g. retry-budget errors carrying a typed source) stay visible.
pub fn find_domain_error(error: &Error) -> Option<&DomainError> {
    if let Some(typed) = error.downcast_ref::<DomainError>() {
        return Some(typed);
    }
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<DomainError>())
}

/// Typed status for an error: the inner [`DomainError`] status when present,
/// otherwise 500 internal. Never inspects message text.
pub fn status_for_error(error: &Error) -> StatusCode {
    find_domain_error(error)
        .map(domain_status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Typed code for an error: the inner [`DomainError`] code when present,
/// otherwise `internal`.
pub fn code_for_error(error: &Error) -> ApiErrorCode {
    find_domain_error(error)
        .map(DomainError::code)
        .unwrap_or(ApiErrorCode::Internal)
}

pub fn json_error_response(status: StatusCode, error: impl Into<String>) -> Response {
    let code = ApiErrorResponse::code_for_status(status.as_u16());
    (status, Json(ApiErrorResponse::new(code, error.into()))).into_response()
}

fn typed_response(error: Error) -> Response {
    let message = error.to_string();
    let status = status_for_error(&error);
    json_error_response(status, message)
}

pub fn internal_error_response(error: Error) -> Response {
    typed_response(error)
}

pub fn map_settings_error(error: Error) -> Response {
    typed_response(error)
}

pub fn map_search_service_error(error: Error) -> Response {
    typed_response(error)
}

pub fn map_document_lookup_error(error: Error) -> Response {
    typed_response(error)
}

/// Typed status for an error that already carries a [`DomainError`].
/// Returns the typed status when a [`DomainError`] is present, otherwise
/// `None` so callers fall back to internal. Takes the full error to support
/// nested `anyhow` contexts.
pub fn typed_status_for_error(error: &Error) -> Option<StatusCode> {
    find_domain_error(error).map(domain_status)
}

/// True when the error chain carries a typed not-found error.
pub fn is_not_found_error(error: &Error) -> bool {
    matches!(find_domain_error(error), Some(DomainError::NotFound(_)))
}

/// True when the error chain carries a typed invalid-argument error.
pub fn is_invalid_argument_error(error: &Error) -> bool {
    matches!(
        find_domain_error(error),
        Some(DomainError::InvalidArgument(_))
    )
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

/// Canonical offset validation that preserves the shared-kernel message while
/// returning a typed [`DomainError::InvalidArgument`] for mapper classification.
pub fn validate_canonical_offset(page: u32, page_size: u32) -> anyhow::Result<()> {
    context69_contracts::OffsetPageQuery { page, page_size }
        .validate()
        .map_err(|error| DomainError::invalid_argument(error.to_string()))?;
    Ok(())
}

/// Canonical cursor-limit validation returning typed invalid-argument errors.
pub fn validate_cursor_limit(limit: u32) -> anyhow::Result<()> {
    context69_contracts::CursorPageQuery {
        limit,
        cursor: None,
    }
    .validate()
    .map_err(|error| DomainError::invalid_argument(error.to_string()))?;
    Ok(())
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
    fn domain_error_code_and_status_cover_all_variants() {
        for (error, status) in [
            (
                DomainError::invalid_argument("bad"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::unauthorized("unauth"),
                StatusCode::UNAUTHORIZED,
            ),
            (DomainError::forbidden("denied"), StatusCode::FORBIDDEN),
            (DomainError::not_found("missing"), StatusCode::NOT_FOUND),
            (DomainError::conflict("clash"), StatusCode::CONFLICT),
            (
                DomainError::payload_too_large("big"),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                DomainError::unprocessable_entity("unprocessable"),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                DomainError::rate_limited("slow"),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                DomainError::unavailable("down"),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                DomainError::upstream_error("bad gateway"),
                StatusCode::BAD_GATEWAY,
            ),
            (
                DomainError::upstream_timeout("timeout"),
                StatusCode::GATEWAY_TIMEOUT,
            ),
            (
                DomainError::internal("boom"),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            assert_eq!(domain_status(&error), status);
            assert_eq!(error.code(), error_code_for_status(status));
            assert_eq!(status_for_code(error.code()), status);
        }
    }

    #[test]
    fn typed_status_falls_back_to_internal_for_unknown_errors() {
        let plain = anyhow::anyhow!("plain internal boom");
        assert_eq!(status_for_error(&plain), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(code_for_error(&plain), ApiErrorCode::Internal);
        assert!(typed_status_for_error(&plain).is_none());
        assert!(!is_not_found_error(&plain));
        assert!(!is_invalid_argument_error(&plain));
    }

    #[test]
    fn typed_dependency_and_upstream_errors_keep_status_through_context() {
        let unavailable: Error = DomainError::unavailable("s3 dependency unavailable").into();
        let wrapped = unavailable.context("upload failed");
        assert_eq!(
            typed_status_for_error(&wrapped),
            Some(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(status_for_error(&wrapped), StatusCode::SERVICE_UNAVAILABLE);

        let timeout: Error = DomainError::upstream_timeout("embedding upstream timed out").into();
        let wrapped = timeout.context("search failed");
        assert_eq!(
            typed_status_for_error(&wrapped),
            Some(StatusCode::GATEWAY_TIMEOUT)
        );

        let rate: Error = DomainError::rate_limited("embedding 429").into();
        assert_eq!(status_for_error(&rate), StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn search_validation_maps_via_typed_invalid_argument() {
        let validation: Error =
            DomainError::invalid_argument("page_size must be between 1 and 100").into();
        assert!(is_invalid_argument_error(&validation));
        let response = map_search_service_error(validation);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let plain = anyhow::anyhow!("plain internal boom");
        assert!(!is_invalid_argument_error(&plain));
        let response = map_search_service_error(anyhow::anyhow!("plain internal boom"));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn document_lookup_maps_via_typed_not_found() {
        let missing: Error = DomainError::not_found("document 42 not found").into();
        assert!(is_not_found_error(&missing));
        let response = map_document_lookup_error(missing);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let plain = anyhow::anyhow!("plain internal boom");
        let response = map_document_lookup_error(plain);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn outer_domain_error_context_maps_to_typed_status() {
        use anyhow::Context as _;
        for (typed, status) in [
            (
                DomainError::not_found("missing outer"),
                StatusCode::NOT_FOUND,
            ),
            (
                DomainError::invalid_argument("bad outer"),
                StatusCode::BAD_REQUEST,
            ),
            (
                DomainError::internal("boom outer"),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            let expected = typed.message().to_string();
            let via_result: Error = Err::<(), _>(anyhow::anyhow!("plain inner"))
                .context(typed.clone())
                .unwrap_err();
            assert_eq!(
                find_domain_error(&via_result).map(DomainError::message),
                Some(expected.as_str())
            );
            assert_eq!(status_for_error(&via_result), status);
            assert_eq!(typed_status_for_error(&via_result), Some(status));

            let via_option: Error = None::<()>.context(typed.clone()).unwrap_err();
            assert_eq!(
                find_domain_error(&via_option).map(DomainError::message),
                Some(expected.as_str())
            );
            assert_eq!(status_for_error(&via_option), status);
        }
    }

    #[test]
    fn middle_domain_error_context_stays_visible_under_plain_outer() {
        use anyhow::Context as _;
        let typed = DomainError::not_found("missing middle");
        let middle: Error = Err::<(), _>(anyhow::anyhow!("plain inner"))
            .context(typed.clone())
            .unwrap_err();
        let wrapped = middle.context("plain outer");
        assert_eq!(
            find_domain_error(&wrapped).map(DomainError::message),
            Some("missing middle")
        );
        assert_eq!(status_for_error(&wrapped), StatusCode::NOT_FOUND);
        assert!(is_not_found_error(&wrapped));
    }
}
