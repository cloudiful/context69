//! Non-mutating resolution of the single configured test database.
//!
//! Precedence is fixed: an explicit [`crate::TEST_DATABASE_URL_VAR`] override
//! wins, otherwise `DATABASE_URL` is read from the repository root `.env`
//! file through a non-mutating iterator. Nothing here reads application
//! configuration, creates a fallback target, or prints the values it sees.

mod endpoint;
mod error;
mod resolver;

pub use endpoint::{Endpoint, Redactor};
pub use error::{ConfigError, ConnectionOptionsUnavailable, MalformedTarget};
pub use resolver::{ResolvedTarget, TargetSource, resolve, resolve_default};
