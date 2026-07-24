use std::{error::Error as StdError, fmt, time::Duration};

use reqwest::StatusCode;

#[derive(Debug)]
pub enum Error {
    InvalidBaseUrl(String),
    InvalidHeader(String),
    InvalidPersonalAccessToken(String),
    Http(reqwest::Error),
    Serialization(serde_json::Error),
    HttpStatus {
        status: StatusCode,
        api_error: Option<String>,
        body: String,
    },
    AuthenticationRequired,
    UrlJoin {
        path: String,
        source: url::ParseError,
    },
    InvalidTimeout(Duration),
    TaskWaitTimeout {
        task_id: uuid::Uuid,
        timeout: Duration,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBaseUrl(url) => write!(f, "invalid base url: {url}"),
            Self::InvalidHeader(value) => write!(f, "invalid header value: {value}"),
            Self::InvalidPersonalAccessToken(message) => write!(f, "{message}"),
            Self::Http(error) => write!(f, "{error}"),
            Self::Serialization(error) => write!(f, "{error}"),
            Self::HttpStatus {
                status,
                api_error,
                body,
            } => {
                if let Some(error) = api_error {
                    write!(f, "http {status}: {error}")
                } else {
                    write!(f, "http {status}: {body}")
                }
            }
            Self::AuthenticationRequired => {
                write!(
                    f,
                    "personal access token is required before calling this API"
                )
            }
            Self::UrlJoin { path, source } => {
                write!(f, "failed to resolve path '{path}': {source}")
            }
            Self::InvalidTimeout(timeout) => {
                write!(f, "invalid timeout: {:?}", timeout)
            }
            Self::TaskWaitTimeout { task_id, timeout } => {
                write!(f, "timed out waiting for task {task_id} after {timeout:?}")
            }
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Http(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::UrlJoin { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value)
    }
}

impl Error {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(_) => true,
            Self::HttpStatus { status, .. } => {
                matches!(status.as_u16(), 408 | 425 | 429 | 500..=599)
            }
            _ => false,
        }
    }
}
