//! Typed failures of the local Docling Serve client.
//!
//! Every failure carries exactly what the retry policy needs and nothing
//! else: no endpoint, no query, no credential, no document bytes, and no raw
//! response body. The remote cause is classified once, here, so no caller has
//! to read an error message to decide what to do.

use std::fmt;

/// The bound one answer exceeded.
///
/// The categories stay separate budgets: a converted-document answer, a
/// small control answer and a cancellation envelope are different sizes and
/// never share one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoclingLimitKind {
    /// The converted-document answer of a finished task.
    Response,
    /// The small answer of a submit, status, list or cancel call.
    Control,
    /// The conflict answer that may name a cancelled task.
    Cancellation,
}

impl DoclingLimitKind {
    /// The readable name of this budget.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Response => "response body",
            Self::Control => "control body",
            Self::Cancellation => "cancellation body",
        }
    }
}

/// One failed Docling Serve operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DoclingError {
    /// The saved endpoint or the VLM configuration cannot produce a valid
    /// request.
    Configuration { message: String },
    /// A request parameter was rejected before the request left the process.
    Validation { parameter: String, reason: String },
    /// The upstream answered with a body larger than the allowed bound.
    ///
    /// `declared` is the length the answer announced, and it is never a
    /// measured total. `observed_at_least` is the number of bytes the stream
    /// had delivered when the bound was crossed, so it is a lower bound of
    /// the real size and never a complete total.
    ResultTooLarge {
        limit: usize,
        kind: DoclingLimitKind,
        declared: Option<u64>,
        observed_at_least: Option<u64>,
    },
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
            Self::ResultTooLarge {
                limit,
                kind,
                declared,
                observed_at_least,
            } => {
                write!(
                    formatter,
                    "docling {} exceeds the maximum of {}",
                    kind.as_str(),
                    crate::docling::describe_limit(*limit)
                )?;
                if let Some(declared) = declared {
                    write!(formatter, ": the answer declared {declared} bytes")?;
                } else if let Some(observed) = observed_at_least {
                    write!(
                        formatter,
                        ": at least {observed} bytes were observed before the bound was crossed"
                    )?;
                }
                Ok(())
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
