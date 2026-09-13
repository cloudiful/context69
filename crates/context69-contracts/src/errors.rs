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
