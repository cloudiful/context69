//! The application-owned encrypted secret store.
//!
//! Every persisted reversible runtime secret has one owner, one stored
//! representation, and one accessor: this store. It is a typed seam over
//! `context69.internal_secrets` plus the [`context69_secret_crypto`] primitive.
//!
//! * **Purpose binding.** A caller names a [`SecretPurpose`], the purpose is
//!   authenticated into the ciphertext and stored on the row, and both are
//!   checked before the AEAD runs, so a secret sealed for one owner cannot be
//!   opened by another even though both live in the same table.
//! * **Explicit lifecycle.** `get`, `put`, `rotate`, `delete`, and `has` are
//!   separate operations with separate outcomes. A create that loses a race
//!   re-reads the winner instead of overwriting it, and `rotate` never creates.
//! * **No plaintext fallback for a sealed secret.** A stored frame is opened with
//!   the configured master key or the call fails with
//!   [`SecretStoreError::MasterKeyNotConfigured`]. Nothing returns ciphertext, an
//!   empty value, or a caller-supplied default when the key is missing.
//!
//! The transition state is deliberate and narrow: a deployment with no master key
//! can still read and create a legacy plaintext row, so an existing installation
//! keeps booting with the values it already has. That is the only plaintext path
//! and the counterpart of [`SecretStore::is_encrypted`].

use std::fmt;

use context69_secret_crypto::{
    MasterKey, SecretCipher, SecretCipherError, SecretPurpose as CryptoPurpose, SecretValue,
    frame::is_valid_key_name,
};
use tracing::warn;

use crate::{
    config::SecretStoreConfig,
    db::{Database, StoredInternalSecret},
};

/// `ciphertext_version` of a row that still holds legacy plaintext bytes.
pub const LEGACY_PLAINTEXT_VERSION: i32 = 0;
/// `ciphertext_version` of a row whose bytes are a versioned sealed frame.
pub const SEALED_CIPHERTEXT_VERSION: i32 = 1;

/// The owner of one persisted secret. Only the code that owns a secret can name
/// its purpose, so no other feature can quietly read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretPurpose {
    /// Key material that signs browser session cookies.
    BrowserSessionSigningKey,
}

impl SecretPurpose {
    /// The purpose string sealed into the frame and stored on the row.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BrowserSessionSigningKey => "browser_session.signing_key",
        }
    }
}

impl fmt::Display for SecretPurpose {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a secret-store operation failed.
///
/// No variant carries a value, a master key, or a rejected configuration input,
/// so the whole enum is safe to log, attach to a health response, or name in an
/// audit note.
#[derive(Debug)]
pub enum SecretStoreError {
    /// No master key is configured, so a sealed secret cannot be opened.
    MasterKeyNotConfigured,
    /// A configured master key is unusable.
    MasterKeyRejected(SecretCipherError),
    /// The key name is not a bounded identifier a frame header could carry.
    InvalidKeyName,
    /// The stored row is owned by a different purpose than the one requested.
    PurposeMismatch,
    /// A rotation named a key that is not stored.
    SecretNotFound,
    /// The stored bytes could not be opened.
    Cipher(SecretCipherError),
    /// The database read or write failed.
    Database(anyhow::Error),
}

impl fmt::Display for SecretStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MasterKeyNotConfigured => formatter.write_str(
                "secret store is not configured: set secret_store.master_key to open sealed secrets",
            ),
            Self::MasterKeyRejected(error) | Self::Cipher(error) => write!(formatter, "{error}"),
            Self::InvalidKeyName => formatter.write_str(
                "secret key name must be 1..=128 characters of ASCII alphanumerics, '.', '_', or '-'",
            ),
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

impl From<anyhow::Error> for SecretStoreError {
    fn from(error: anyhow::Error) -> Self {
        Self::Database(error)
    }
}

/// Whether a create wrote the row or found the key already taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    Created,
    AlreadyPresent,
}

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

/// Builds the cipher a deployment configured, if it configured one. An absent or
/// blank `master_key` is the transition state, not an error; a *present but
/// unusable* key is a configuration failure and never degrades to that state.
pub fn master_key_from_config(
    config: &SecretStoreConfig,
) -> Result<Option<MasterKey>, SecretStoreError> {
    let Some(encoded) = config
        .master_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    MasterKey::from_base64(encoded)
        .map(Some)
        .map_err(SecretStoreError::MasterKeyRejected)
}

/// Produces the stored representation of one secret value: with a cipher, a
/// versioned frame carrying the purpose and key name; without one, the value as
/// it was, in the legacy representation the transition still has to read.
pub fn seal_for_storage(
    cipher: Option<&SecretCipher>,
    purpose: SecretPurpose,
    key_name: &str,
    value: &[u8],
) -> Result<StoredSecretBytes, SecretStoreError> {
    if !is_valid_key_name(key_name) {
        return Err(SecretStoreError::InvalidKeyName);
    }
    let Some(cipher) = cipher else {
        return Ok(StoredSecretBytes {
            value: value.to_vec(),
            purpose: None,
            key_version: LEGACY_PLAINTEXT_VERSION,
            ciphertext_version: LEGACY_PLAINTEXT_VERSION,
        });
    };
    let sealed = cipher
        .seal(&crypto_purpose(purpose)?, key_name, value)
        .map_err(SecretStoreError::Cipher)?;
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
pub fn open_from_storage(
    cipher: Option<&SecretCipher>,
    purpose: SecretPurpose,
    key_name: &str,
    stored: &StoredInternalSecret,
) -> Result<SecretValue, SecretStoreError> {
    if !is_valid_key_name(key_name) {
        return Err(SecretStoreError::InvalidKeyName);
    }
    if stored.is_legacy_plaintext() {
        return Ok(SecretValue::from_plaintext(stored.value.clone()));
    }
    if stored.purpose.as_deref() != Some(purpose.as_str()) {
        return Err(SecretStoreError::PurposeMismatch);
    }
    cipher
        .ok_or(SecretStoreError::MasterKeyNotConfigured)?
        .open(&crypto_purpose(purpose)?, key_name, &stored.value)
        .map_err(SecretStoreError::Cipher)
}

fn crypto_purpose(purpose: SecretPurpose) -> Result<CryptoPurpose, SecretStoreError> {
    CryptoPurpose::new(purpose.as_str()).map_err(|_| SecretStoreError::InvalidKeyName)
}

/// The store itself: a database plus the cipher deployment supplied, if any.
#[derive(Clone)]
pub struct SecretStore {
    db: Database,
    cipher: Option<SecretCipher>,
}

impl fmt::Debug for SecretStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretStore")
            .field("is_encrypted", &self.is_encrypted())
            .finish_non_exhaustive()
    }
}

impl SecretStore {
    /// Builds a store from validated deployment configuration. The only failure
    /// mode is an unusable configured master key; the store is otherwise usable
    /// either way, and the only difference is whether it can open a sealed secret.
    pub fn new(db: Database, config: &SecretStoreConfig) -> Result<Self, SecretStoreError> {
        let cipher = match master_key_from_config(config)? {
            Some(master_key) => Some(SecretCipher::new(master_key, config.key_version)),
            None => {
                warn!(
                    "secret_store.master_key is not configured; sealed secrets cannot be opened \
                     and new secrets are stored in the legacy plaintext representation"
                );
                None
            }
        };
        Ok(Self { db, cipher })
    }

    /// Whether this deployment can open sealed secrets.
    pub fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    /// Reads one secret, or `None` when the key is not stored. Fails as
    /// [`open_from_storage`] does, plus on a database error.
    pub async fn get(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
    ) -> Result<Option<SecretValue>, SecretStoreError> {
        let Some(stored) = self.db.get_internal_secret(key_name).await? else {
            return Ok(None);
        };
        open_from_storage(self.cipher.as_ref(), purpose, key_name, &stored).map(Some)
    }

    /// Creates one secret and reports whether this call created it. Fails as
    /// [`seal_for_storage`] does, plus on a database error.
    pub async fn put(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
        value: &[u8],
    ) -> Result<PutOutcome, SecretStoreError> {
        let stored = seal_for_storage(self.cipher.as_ref(), purpose, key_name, value)?;
        let created = self
            .db
            .put_internal_secret(
                key_name,
                &stored.value,
                stored.purpose.as_deref(),
                stored.key_version,
                stored.ciphertext_version,
            )
            .await?;
        Ok(if created {
            PutOutcome::Created
        } else {
            PutOutcome::AlreadyPresent
        })
    }

    /// Replaces the stored representation of an existing secret. A rotation never
    /// creates: a key that is not stored is [`SecretStoreError::SecretNotFound`].
    pub async fn rotate(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
        value: &[u8],
    ) -> Result<(), SecretStoreError> {
        let stored = seal_for_storage(self.cipher.as_ref(), purpose, key_name, value)?;
        let rotated = self
            .db
            .rotate_internal_secret(
                key_name,
                &stored.value,
                stored.purpose.as_deref(),
                stored.key_version,
                stored.ciphertext_version,
            )
            .await?;
        if rotated {
            Ok(())
        } else {
            Err(SecretStoreError::SecretNotFound)
        }
    }

    /// Clears one secret and reports whether this call cleared it. The purpose is
    /// an ownership check, not decoration: a sealed row owned by another feature
    /// is not clearable from here.
    pub async fn delete(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
    ) -> Result<bool, SecretStoreError> {
        let Some(stored) = self.read_row(key_name).await? else {
            return Ok(false);
        };
        if !stored.is_legacy_plaintext() && stored.purpose.as_deref() != Some(purpose.as_str()) {
            return Err(SecretStoreError::PurposeMismatch);
        }
        Ok(self.db.delete_internal_secret(key_name).await?)
    }

    /// Whether one secret is stored. Shares [`Self::get`]'s semantics, including
    /// refusing to report presence for a sealed secret the deployment cannot open:
    /// a store that cannot read a secret must not claim to have checked it.
    pub async fn has(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
    ) -> Result<bool, SecretStoreError> {
        Ok(self.get(purpose, key_name).await?.is_some())
    }

    /// Reads one secret, creating it from `candidate` when it is not stored. The
    /// only create-on-read path, and it is race-safe: if a concurrent writer
    /// created the key first, the create reports `AlreadyPresent` and the stored
    /// value is read instead of the candidate being written over it, so two
    /// processes cannot end up signing with different values.
    pub async fn get_or_create(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
        candidate: &[u8],
    ) -> Result<SecretValue, SecretStoreError> {
        if let Some(existing) = self.get(purpose, key_name).await? {
            return Ok(existing);
        }
        match self.put(purpose, key_name, candidate).await? {
            PutOutcome::Created => Ok(SecretValue::from_plaintext(candidate.to_vec())),
            PutOutcome::AlreadyPresent => self
                .get(purpose, key_name)
                .await?
                .ok_or(SecretStoreError::SecretNotFound),
        }
    }

    /// Lists the keys still stored in the legacy plaintext representation: the
    /// transition worklist for the reversible-secret migration. Only key names
    /// come back, so listing what is left to seal discloses nothing.
    pub async fn list_unversioned_keys(&self) -> Result<Vec<String>, SecretStoreError> {
        Ok(self.db.list_unversioned_internal_secrets().await?)
    }

    /// Reads a row without decoding it, so an ownership check needs no key.
    async fn read_row(
        &self,
        key_name: &str,
    ) -> Result<Option<StoredInternalSecret>, SecretStoreError> {
        if !is_valid_key_name(key_name) {
            return Err(SecretStoreError::InvalidKeyName);
        }
        Ok(self.db.get_internal_secret(key_name).await?)
    }
}
