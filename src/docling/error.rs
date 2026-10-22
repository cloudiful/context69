//! Typed failures of the local Docling Serve client.
//!
//! Every failure carries exactly what the retry policy needs and nothing
//! else: no endpoint, no query, no credential, no document bytes, and no raw
//! response body. The remote cause is classified once, here, so no caller has
//! to read an error message to decide what to do.

use std::fmt;

/// One failed Docling Serve operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DoclingError {
    /// The saved endpoint or the VLM configuration cannot produce a valid
    /// request.
    Configuration { message: String },
    /// A request parameter was rejected before the request left the process.
    Validation { parameter: String, reason: String },
    /// The upstream answered with a body larger than the allowed bound.
    ResultTooLarge { limit: usize },
    /// The remote task reached a terminal unsuccessful state.
    TaskFailed { status: String, message: String },
    /// The upstream answered with a status that is not a success.
    Http { status: u16 },
    /// The request could not be completed.
    Transport { message: String },
    /// The request exceeded its per-operation budget.
    Timeout,
    /// The upstream answered with something that is not a valid response.
    Parse { message: String },
    /// The upstream answered about a different task than the one requested.
    IdentityMismatch,
}

impl fmt::Display for DoclingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration { message } => {
                write!(formatter, "docling configuration is invalid: {message}")
            }
            Self::Validation { parameter, reason } => {
                write!(
                    formatter,
                    "docling request rejected '{parameter}': {reason}"
                )
            }
            Self::ResultTooLarge { limit } => {
                write!(
                    formatter,
                    "docling response exceeds the maximum of {limit} bytes"
                )
            }
            Self::TaskFailed { status, message } => {
                write!(
                    formatter,
                    "docling task failed with status {status}: {message}"
                )
            }
            Self::Http { status } => {
                write!(formatter, "docling request failed with HTTP {status}")
            }
            Self::Transport { message } => {
                write!(formatter, "docling request failed: {message}")
            }
            Self::Timeout => write!(formatter, "docling request timed out"),
            Self::Parse { message } => {
                write!(formatter, "docling response is not valid: {message}")
            }
            Self::IdentityMismatch => {
                write!(formatter, "docling answered about a different task")
            }
        }
    }
}

impl std::error::Error for DoclingError {}
