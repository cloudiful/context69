//! Sanitized failure categories for target resolution.
//!
//! Every variant is a unit value or a small integer, so `Debug` and `Display`
//! can be derived: no variant can carry a URL, a password, or a raw `.env`
//! line back to a log.

use std::fmt;

/// Why a selected test database target was rejected before any connection
/// attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedTarget {
    /// The value is not a `postgres` or `postgresql` URL.
    UnsupportedScheme,
    /// A fragment is present; PostgreSQL connection URLs never carry one.
    FragmentNotAllowed,
    /// The authority section has no usable host.
    MissingHost,
    /// The URL does not name exactly one database.
    MissingDatabase,
    /// The URL does not name a role.
    MissingUsername,
    /// The URL could not be parsed into connection options.
    Unparseable,
}

/// Why resolution produced no usable target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// Neither the explicit override nor the repository `.env` file supplied a
    /// value. This is a hard failure; the runner never continues as if the
    /// database-backed suite had passed.
    TargetNotConfigured,
    /// The explicit override was present but empty.
    OverrideEmpty,
    /// The explicit override was present but unusable.
    OverrideMalformed(MalformedTarget),
    /// The repository `.env` file could not be read.
    EnvFileUnreadable,
    /// The repository `.env` file contains a line this build cannot parse.
    EnvFileMalformed,
    /// The repository `.env` file supplied a value but it was unusable.
    EnvFileValueMalformed(MalformedTarget),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetNotConfigured => f.write_str(
                "no test database is configured; set CONTEXT69_TEST_DATABASE_URL or add \
                 DATABASE_URL to the repository .env file",
            ),
            Self::OverrideEmpty => f.write_str("CONTEXT69_TEST_DATABASE_URL is set but empty"),
            Self::OverrideMalformed(reason) => {
                write!(f, "CONTEXT69_TEST_DATABASE_URL is unusable: {reason}")
            }
            Self::EnvFileUnreadable => f.write_str("the repository .env file could not be read"),
            Self::EnvFileMalformed => {
                f.write_str("the repository .env file contains an unparsable line")
            }
            Self::EnvFileValueMalformed(reason) => {
                write!(
                    f,
                    "DATABASE_URL in the repository .env file is unusable: {reason}"
                )
            }
        }
    }
}

impl fmt::Display for MalformedTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::UnsupportedScheme => "expected a postgres or postgresql URL",
            Self::FragmentNotAllowed => "a URL fragment is not allowed",
            Self::MissingHost => "no host",
            Self::MissingDatabase => "no database name",
            Self::MissingUsername => "no role name",
            Self::Unparseable => "could not be parsed",
        };
        f.write_str(text)
    }
}

impl std::error::Error for ConfigError {}

/// The selected URL passed structural validation but sqlx would not turn it into
/// connection options. The underlying value is deliberately absent so it cannot
/// reach a log.
#[derive(Debug, Clone, Copy)]
pub struct ConnectionOptionsUnavailable;

impl fmt::Display for ConnectionOptionsUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the selected test database URL could not be turned into connection options")
    }
}

impl std::error::Error for ConnectionOptionsUnavailable {}
