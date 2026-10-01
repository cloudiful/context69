//! Why a secret-store operation failed.
//!
//! No variant carries a secret value, key material, or a rejected configuration
//! input, so the whole enum is safe to log, attach to a health response, or name
//! in an audit note.

use std::fmt;

use context69_secret_crypto::{FrameError, SecretCipherError};

/// Why a secret-store operation failed.
///
/// The variants split along the line a caller has to act on: a missing or
/// unusable deployment key is a configuration problem, a purpose or key-name
/// mismatch is a programming problem, and everything else is either a
/// cryptographic failure or a storage failure.
#[derive(Debug)]
pub enum SecretStoreError {
    /// No master key is configured, so a sealed secret cannot be opened.
    ///
    /// Reported instead of falling back to anything else. A metadata-only
    /// presence check still succeeds in this state, so `has_*` projections stay
    /// truthful on a deployment that cannot decrypt.
    MasterKeyNotConfigured,
    /// A configured master key is unusable.
    MasterKeyRejected(SecretCipherError),
    /// The key name is not a bounded identifier a frame header could carry.
    InvalidKeyName,
    /// The purpose is not a bounded identifier, or is not one this build knows.
    InvalidPurpose,
    /// The stored row is owned by a different purpose than the one requested.
    PurposeMismatch,
    /// A rotation named a key that is not stored.
    SecretNotFound,
    /// The stored bytes could not be opened.
    Cipher(SecretCipherError),
    /// The database read or write failed.
    Database(anyhow::Error),
}

const KEY_NAME_BOUNDS: &str =
    "secret key name must be 1..=128 characters of ASCII alphanumerics, '.', '_', or '-'";
const PURPOSE_BOUNDS: &str =
    "secret purpose must be 1..=64 characters of ASCII alphanumerics, '.', '_', or '-'";

impl fmt::Display for SecretStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MasterKeyNotConfigured => formatter.write_str(
                "secret store is not configured: set secret_store.master_key to open sealed secrets",
            ),
            Self::MasterKeyRejected(error) | Self::Cipher(error) => write!(formatter, "{error}"),
            Self::InvalidKeyName => formatter.write_str(KEY_NAME_BOUNDS),
            Self::InvalidPurpose => formatter.write_str(PURPOSE_BOUNDS),
            Self::PurposeMismatch => {
                formatter.write_str("secret is owned by a different purpose")
            }
            Self::SecretNotFound => formatter.write_str("secret is not stored"),
            Self::Database(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for SecretStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MasterKeyRejected(error) | Self::Cipher(error) => Some(error),
            Self::Database(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl From<SecretCipherError> for SecretStoreError {
    fn from(error: SecretCipherError) -> Self {
        match error {
            // A malformed purpose is a caller mistake, not a cryptographic
            // failure, so it keeps its own actionable variant.
            SecretCipherError::InvalidPurpose => Self::InvalidPurpose,
            // Likewise a key name a frame header could never carry. The other
            // frame failures describe the stored bytes, not the caller's input.
            SecretCipherError::Frame(
                FrameError::InvalidNameLength
                | FrameError::InvalidNameCharacters
                | FrameError::InvalidNameEncoding,
            ) => Self::InvalidKeyName,
            other => Self::Cipher(other),
        }
    }
}

impl From<anyhow::Error> for SecretStoreError {
    fn from(error: anyhow::Error) -> Self {
        Self::Database(error)
    }
}

impl From<sqlx::Error> for SecretStoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.into())
    }
}
