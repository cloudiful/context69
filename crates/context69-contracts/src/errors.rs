use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    InvalidArgument,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    PayloadTooLarge,
    UnprocessableEntity,
    RateLimited,
    UpstreamError,
    Unavailable,
    UpstreamTimeout,
    #[default]
    Internal,
}

impl ApiErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::PayloadTooLarge => "payload_too_large",
            Self::UnprocessableEntity => "unprocessable_entity",
            Self::RateLimited => "rate_limited",
            Self::UpstreamError => "upstream_error",
            Self::Unavailable => "unavailable",
            Self::UpstreamTimeout => "upstream_timeout",
            Self::Internal => "internal",
        }
    }

    pub fn code_for_status(status: u16) -> Self {
        match status {
            400 => Self::InvalidArgument,
            401 => Self::Unauthorized,
            403 => Self::Forbidden,
            404 => Self::NotFound,
            409 => Self::Conflict,
            413 => Self::PayloadTooLarge,
            422 => Self::UnprocessableEntity,
            429 => Self::RateLimited,
            502 => Self::UpstreamError,
            503 => Self::Unavailable,
            504 => Self::UpstreamTimeout,
            _ => Self::Internal,
        }
    }

    pub fn status_code(self) -> u16 {
        match self {
            Self::InvalidArgument => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::PayloadTooLarge => 413,
            Self::UnprocessableEntity => 422,
            Self::RateLimited => 429,
            Self::UpstreamError => 502,
            Self::Unavailable => 503,
            Self::UpstreamTimeout => 504,
            Self::Internal => 500,
        }
    }
}

impl std::fmt::Display for ApiErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ApiErrorCode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "invalid_argument" => Ok(Self::InvalidArgument),
            "unauthorized" => Ok(Self::Unauthorized),
            "forbidden" => Ok(Self::Forbidden),
            "not_found" => Ok(Self::NotFound),
            "conflict" => Ok(Self::Conflict),
            "payload_too_large" => Ok(Self::PayloadTooLarge),
            "unprocessable_entity" => Ok(Self::UnprocessableEntity),
            "rate_limited" => Ok(Self::RateLimited),
            "upstream_error" => Ok(Self::UpstreamError),
            "unavailable" => Ok(Self::Unavailable),
            "upstream_timeout" => Ok(Self::UpstreamTimeout),
            "internal" => Ok(Self::Internal),
            other => Err(anyhow::anyhow!("unsupported error code: {other}")),
        }
    }
}

impl From<ApiErrorCode> for String {
    fn from(code: ApiErrorCode) -> Self {
        code.as_str().to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CanonicalApiErrorResponse {
    pub code: ApiErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl CanonicalApiErrorResponse {
    pub fn new(code: ApiErrorCode, message: String) -> Self {
        Self {
            code,
            message,
            details: None,
            request_id: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = Some(request_id);
        self
    }

    pub fn status_code(&self) -> u16 {
        self.code.status_code()
    }
}

impl From<CanonicalApiErrorResponse> for crate::ApiErrorResponse {
    fn from(canonical: CanonicalApiErrorResponse) -> Self {
        Self {
            code: canonical.code.as_str().to_string(),
            message: canonical.message,
            details: canonical.details,
            request_id: canonical.request_id,
        }
    }
}

/// Typed domain error taxonomy shared by service producers and HTTP mappers.
///
/// Each variant preserves the human-readable message while carrying a stable
/// [`ApiErrorCode`] mapping. Producers construct these at the source;
/// HTTP mappers classify via downcast through `anyhow` chains and never via
/// message substrings. Failures without a typed category map to `internal`.
///
/// This enum is deliberately transport-free: no `serde`, schema, or HTTP
/// framework types. It is never serialized on the wire. HTTP status mapping
/// lives with the HTTP layer (`context69-http-support`), which converts
/// [`ApiErrorCode`] to status codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    InvalidArgument(String),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    PayloadTooLarge(String),
    UnprocessableEntity(String),
    RateLimited(String),
    Unavailable(String),
    UpstreamError(String),
    UpstreamTimeout(String),
    Internal(String),
}

impl DomainError {
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument(message.into())
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::PayloadTooLarge(message.into())
    }

    pub fn unprocessable_entity(message: impl Into<String>) -> Self {
        Self::UnprocessableEntity(message.into())
    }

    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::RateLimited(message.into())
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    pub fn upstream_error(message: impl Into<String>) -> Self {
        Self::UpstreamError(message.into())
    }

    pub fn upstream_timeout(message: impl Into<String>) -> Self {
        Self::UpstreamTimeout(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn message(&self) -> &str {
        match self {
            Self::InvalidArgument(message)
            | Self::Unauthorized(message)
            | Self::Forbidden(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::PayloadTooLarge(message)
            | Self::UnprocessableEntity(message)
            | Self::RateLimited(message)
            | Self::Unavailable(message)
            | Self::UpstreamError(message)
            | Self::UpstreamTimeout(message)
            | Self::Internal(message) => message,
        }
    }

    pub fn code(&self) -> ApiErrorCode {
        match self {
            Self::InvalidArgument(_) => ApiErrorCode::InvalidArgument,
            Self::Unauthorized(_) => ApiErrorCode::Unauthorized,
            Self::Forbidden(_) => ApiErrorCode::Forbidden,
            Self::NotFound(_) => ApiErrorCode::NotFound,
            Self::Conflict(_) => ApiErrorCode::Conflict,
            Self::PayloadTooLarge(_) => ApiErrorCode::PayloadTooLarge,
            Self::UnprocessableEntity(_) => ApiErrorCode::UnprocessableEntity,
            Self::RateLimited(_) => ApiErrorCode::RateLimited,
            Self::Unavailable(_) => ApiErrorCode::Unavailable,
            Self::UpstreamError(_) => ApiErrorCode::UpstreamError,
            Self::UpstreamTimeout(_) => ApiErrorCode::UpstreamTimeout,
            Self::Internal(_) => ApiErrorCode::Internal,
        }
    }

    /// Numeric HTTP status for this error's code. The HTTP layer converts
    /// this to its own status type.
    pub fn status_code_number(&self) -> u16 {
        self.code().status_code()
    }
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_error_preserves_message_and_maps_to_stable_codes() {
        for (error, code, status) in [
            (
                DomainError::invalid_argument("bad"),
                ApiErrorCode::InvalidArgument,
                400,
            ),
            (
                DomainError::unauthorized("unauth"),
                ApiErrorCode::Unauthorized,
                401,
            ),
            (
                DomainError::forbidden("denied"),
                ApiErrorCode::Forbidden,
                403,
            ),
            (
                DomainError::not_found("missing"),
                ApiErrorCode::NotFound,
                404,
            ),
            (DomainError::conflict("clash"), ApiErrorCode::Conflict, 409),
            (
                DomainError::payload_too_large("big"),
                ApiErrorCode::PayloadTooLarge,
                413,
            ),
            (
                DomainError::unprocessable_entity("unprocessable"),
                ApiErrorCode::UnprocessableEntity,
                422,
            ),
            (
                DomainError::rate_limited("slow"),
                ApiErrorCode::RateLimited,
                429,
            ),
            (
                DomainError::unavailable("down"),
                ApiErrorCode::Unavailable,
                503,
            ),
            (
                DomainError::upstream_error("bad gateway"),
                ApiErrorCode::UpstreamError,
                502,
            ),
            (
                DomainError::upstream_timeout("timeout"),
                ApiErrorCode::UpstreamTimeout,
                504,
            ),
            (DomainError::internal("boom"), ApiErrorCode::Internal, 500),
        ] {
            assert_eq!(error.message(), error.to_string().as_str());
            assert_eq!(error.code(), code);
            assert_eq!(error.status_code_number(), status);
            assert_eq!(ApiErrorCode::code_for_status(status), code);
        }
    }

    #[test]
    fn domain_error_travels_through_anyhow_contexts() {
        let error: anyhow::Error = DomainError::not_found("document 42 not found").into();
        let wrapped = error.context("outer context");
        let found = wrapped
            .chain()
            .find_map(|cause| cause.downcast_ref::<DomainError>());
        assert_eq!(
            found.map(DomainError::message),
            Some("document 42 not found")
        );
    }
}
