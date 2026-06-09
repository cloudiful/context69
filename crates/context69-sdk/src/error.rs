use std::{error::Error as StdError, fmt, time::Duration};

use reqwest::StatusCode;

#[derive(Debug)]
pub enum Error {
    InvalidBaseUrl(String),
    InvalidHeader(String),
    Http(reqwest::Error),
    Serialization(serde_json::Error),
    HttpStatus {
        status: StatusCode,
        api_error: Option<String>,
        body: String,
    },
    AuthenticationRequired,
    RefreshFailed {
        status: Option<StatusCode>,
        message: String,
    },
    UrlJoin {
        path: String,
        source: url::ParseError,
    },
    InvalidTimeout(Duration),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBaseUrl(url) => write!(f, "invalid base url: {url}"),
            Self::InvalidHeader(value) => write!(f, "invalid header value: {value}"),
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
                write!(f, "authentication required before calling this API")
            }
            Self::RefreshFailed { status, message } => match status {
                Some(status) => write!(f, "token refresh failed with {status}: {message}"),
                None => write!(f, "token refresh failed: {message}"),
            },
            Self::UrlJoin { path, source } => {
                write!(f, "failed to resolve path '{path}': {source}")
            }
            Self::InvalidTimeout(timeout) => {
                write!(f, "invalid timeout: {:?}", timeout)
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
