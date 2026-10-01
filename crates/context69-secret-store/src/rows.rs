//! The rows the store reads and writes, and the pure transformations between a
//! secret's plaintext and its stored form.
//!
//! Two row shapes matter, and they are deliberately different. A *value* row
//! carries the stored bytes and is only read when a caller actually needs the
//! secret. A *metadata* row carries the version marker and owning purpose only,
//! so a presence check can answer without ever touching ciphertext.
//!
//! [`seal_for_storage`] and [`open_from_storage`] are the whole transition in two
//! functions. They are pure — no pool, no clock, no configuration — so the
//! representation rules can be asserted without a database, and a dependent crate
//! can reason about a sealed value without holding a store.

use std::fmt;

use context69_secret_crypto::{SecretCipher, SecretValue};

use crate::{
    error::SecretStoreError,
    purpose::{SecretKeyName, SecretPurpose},
};

/// `ciphertext_version` of a row that still holds legacy plaintext bytes.
pub const LEGACY_PLAINTEXT_VERSION: i32 = 0;
/// `ciphertext_version` of a row whose bytes are a versioned sealed frame.
pub const SEALED_CIPHERTEXT_VERSION: i32 = 1;

/// The bytes to write for one secret, with the metadata that describes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSecretBytes {
    /// A sealed frame, or legacy plaintext while the deployment has no key.
    pub value: Vec<u8>,
    /// The owning purpose, absent for a legacy plaintext write.
    pub purpose: Option<String>,
    /// Master-key version, zero for a legacy plaintext write.
    pub key_version: i32,
    /// Representation marker matching `value`.
    pub ciphertext_version: i32,
}

/// One secret row, with its stored bytes.
///
/// `Debug` reports the row's metadata and byte count, never its bytes, so a
/// stored row can be described in a log or an audit note.
#[derive(Clone)]
pub struct StoredSecret {
    /// The stored bytes, framed or legacy.
    pub value: Vec<u8>,
    /// Owning purpose, absent on a legacy row.
    pub purpose: Option<String>,
    /// Master-key version the value was sealed with.
    pub key_version: i32,
    /// Representation marker: zero is legacy plaintext.
    pub ciphertext_version: i32,
}

impl fmt::Debug for StoredSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredSecret")
            .field("purpose", &self.purpose)
            .field("key_version", &self.key_version)
            .field("ciphertext_version", &self.ciphertext_version)
            .field("value_len", &self.value.len())
            .finish()
    }
}

/// One secret row's metadata, without its value.
///
/// This is what a purpose-scoped presence check reads. It is derived from a
/// query that never selects `value`, so answering "is this secret configured?"
/// cannot and does not decrypt anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSecretMetadata {
    /// Owning purpose, absent on a legacy row.
    pub purpose: Option<String>,
    /// Representation marker: zero is legacy plaintext.
    pub ciphertext_version: i32,
}

impl StoredSecretMetadata {
    /// Whether the row still holds the legacy plaintext representation.
    #[must_use]
    pub fn is_legacy_plaintext(&self) -> bool {
        self.ciphertext_version == LEGACY_PLAINTEXT_VERSION
    }

    /// Whether this row is the requested purpose's secret.
    ///
    /// A legacy row is unclaimed rather than owned, and the transition hands it
    /// back to the purpose that created it, so a legacy row answers `true` for
    /// any purpose. A sealed row answers `true` only for its stored owner.
    #[must_use]
    pub fn belongs_to(&self, purpose: &str) -> bool {
        self.is_legacy_plaintext() || self.purpose.as_deref() == Some(purpose)
    }
}

impl StoredSecret {
    /// Whether these bytes are the legacy plaintext representation.
    #[must_use]
    pub fn is_legacy_plaintext(&self) -> bool {
        self.ciphertext_version == LEGACY_PLAINTEXT_VERSION
    }

    /// The row's metadata, without the value.
    #[must_use]
    pub fn metadata(&self) -> StoredSecretMetadata {
        StoredSecretMetadata {
            purpose: self.purpose.clone(),
            ciphertext_version: self.ciphertext_version,
        }
    }
}

/// Produces the stored representation of one secret value: with a cipher, a
/// versioned frame carrying the purpose and key name; without one, the value as
/// it was, in the legacy representation the transition still has to read.
///
/// # Errors
///
/// [`SecretStoreError::InvalidKeyName`] for a key name no frame header could
/// carry, and [`SecretStoreError::Cipher`] when sealing fails.
pub fn seal_for_storage(
    cipher: Option<&SecretCipher>,
    purpose: SecretPurpose,
    key_name: &str,
    value: &[u8],
) -> Result<StoredSecretBytes, SecretStoreError> {
    let key_name = SecretKeyName::new(key_name)?;
    let Some(cipher) = cipher else {
        return Ok(StoredSecretBytes {
            value: value.to_vec(),
            purpose: None,
            key_version: LEGACY_PLAINTEXT_VERSION,
            ciphertext_version: LEGACY_PLAINTEXT_VERSION,
        });
    };
    let sealed = cipher
        .seal(&purpose_name(purpose)?, key_name.as_str(), value)
        .map_err(SecretStoreError::from)?;
    Ok(StoredSecretBytes {
        value: sealed,
        purpose: Some(purpose.as_str().to_string()),
        key_version: i32::try_from(cipher.key_version())
            .map_err(|_| SecretStoreError::MasterKeyNotConfigured)?,
        ciphertext_version: SEALED_CIPHERTEXT_VERSION,
    })
}

/// Returns the plaintext of one stored row.
///
/// This is the whole transition in one place. A legacy row comes back exactly as
/// stored, which is what keeps an existing deployment resolving the same bytes it
/// always did. A sealed row is opened only with the configured cipher, and only
/// after its stored purpose matches the requested one, so a cross-purpose read
/// fails with a specific reason rather than a generic authentication failure.
///
/// # Errors
///
/// [`SecretStoreError::MasterKeyNotConfigured`] for a sealed row with no
/// configured key, [`SecretStoreError::PurposeMismatch`] when the row belongs to
/// another purpose, and [`SecretStoreError::Cipher`] when the bytes do not open.
pub fn open_from_storage(
    cipher: Option<&SecretCipher>,
    purpose: SecretPurpose,
    key_name: &SecretKeyName,
    stored: &StoredSecret,
) -> Result<SecretValue, SecretStoreError> {
    if stored.is_legacy_plaintext() {
        return Ok(SecretValue::from_plaintext(stored.value.clone()));
    }
    if stored.purpose.as_deref() != Some(purpose.as_str()) {
        return Err(SecretStoreError::PurposeMismatch);
    }
    cipher
        .ok_or(SecretStoreError::MasterKeyNotConfigured)?
        .open(&purpose_name(purpose)?, key_name.as_str(), &stored.value)
        .map_err(SecretStoreError::from)
}

fn purpose_name(
    purpose: SecretPurpose,
) -> Result<context69_secret_crypto::SecretPurpose, SecretStoreError> {
    context69_secret_crypto::SecretPurpose::new(purpose.as_str())
        .map_err(|_| SecretStoreError::InvalidPurpose)
}
