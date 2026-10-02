//! The store: one typed, purpose-scoped accessor over the encrypted secret table.
//!
//! Every persisted reversible runtime secret has one owner, one stored
//! representation, and one accessor. The operations are separate on purpose —
//! `get`, `put`, `rotate`, `delete`, `has` — with separate outcomes, so a caller
//! cannot read "update" as "insert" or "is it configured?" as "give me the
//! value". The properties the store guarantees:
//!
//! * **Purpose binding.** A [`SecretPurpose`] is authenticated into the
//!   ciphertext *and* stored on the row, and both are checked before a value is
//!   returned, so a secret sealed for one owner cannot be read through another.
//! * **No plaintext fallback for a sealed secret.** A sealed row is opened with
//!   the configured master key or the call fails with
//!   [`SecretStoreError::MasterKeyNotConfigured`]. Nothing returns ciphertext, an
//!   empty value, or a caller-supplied default when the key is missing.
//! * **Metadata-only presence.** [`SecretStore::has`] reads only the row's
//!   version marker and purpose, so `has_*` projections stay truthful on a
//!   deployment that has no master key, and answering them decrypts nothing.
//! * **Bounded names.** A key name arrives as a `&str` and is validated into a
//!   [`SecretKeyName`] before anything is read or written, so a malformed
//!   dynamic identifier is refused at the call site instead of becoming a row.
//!   The boundary stays a string because a caller's identifier is a string; the
//!   bound is enforced by construction, not by convention.
//!
//! The transition state is deliberate and narrow: a deployment with no master key
//! can still read and create a legacy plaintext row, so an existing
//! installation keeps booting with the values it already has. That is the only
//! plaintext path and the counterpart of [`SecretStore::is_encrypted`]; the
//! reversible-secret backfill and the legacy-column removal are separate units.
//!
//! Every operation here is single-deployment — one database, one cipher. The one
//! exception, [`rewrap`], needs a second deployment because it re-seals this
//! store's rows under an incoming master key, and it lives in its own module for
//! that reason.

mod rewrap;

use std::fmt;

use context69_secret_crypto::{MasterKey, SecretCipher, SecretValue};
use tracing::debug;

use crate::{
    database::SecretDatabase,
    error::SecretStoreError,
    purpose::{SecretKeyName, SecretPurpose},
    rows::{open_from_storage, seal_for_storage},
};

pub use rewrap::{RewrapError, RewrapReport};

/// The outcome of a create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    /// This call created the row.
    Created,
    /// The key was already taken; nothing was written.
    AlreadyPresent,
}

/// Builds the cipher a deployment configured, if it configured one.
///
/// An absent or blank master key is the transition state, not an error. A
/// *present but unusable* key is a configuration failure and never degrades to
/// that state.
///
/// # Errors
///
/// [`SecretStoreError::MasterKeyRejected`] when the configured value is not a
/// usable 32-byte key. The rejected value is never carried in the error.
pub fn master_key_from_config(
    master_key: Option<&str>,
) -> Result<Option<MasterKey>, SecretStoreError> {
    let Some(encoded) = master_key.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    MasterKey::from_base64(encoded)
        .map(Some)
        .map_err(SecretStoreError::MasterKeyRejected)
}

/// The store: a database boundary plus the cipher deployment supplied, if any.
#[derive(Clone)]
pub struct SecretStore {
    db: SecretDatabase,
    cipher: Option<SecretCipher>,
    key_version: u32,
}

impl fmt::Debug for SecretStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretStore")
            .field("is_encrypted", &self.is_encrypted())
            .field("key_version", &self.key_version)
            .finish_non_exhaustive()
    }
}

impl SecretStore {
    /// Builds a store over an existing pool from validated deployment inputs.
    ///
    /// The only failure mode is an unusable configured master key; the store is
    /// otherwise usable either way, and the only difference is whether it can
    /// open a sealed secret.
    ///
    /// Several handles onto one configuration are expected — the application, a
    /// dependent service crate, a background task — so this logs the unconfigured
    /// state at debug rather than warning once per handle. Reporting it once per
    /// process is the application's job.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::MasterKeyRejected`] when a configured master key
    /// cannot be used.
    pub fn new(
        db: SecretDatabase,
        master_key: Option<&str>,
        key_version: u32,
    ) -> Result<Self, SecretStoreError> {
        let cipher = match master_key_from_config(master_key)? {
            Some(master_key) => Some(SecretCipher::new(master_key, key_version)),
            None => {
                debug!(
                    "secret store built without a master key; sealed secrets cannot be opened \
                     and new secrets use the legacy plaintext representation"
                );
                None
            }
        };
        Ok(Self {
            db,
            cipher,
            key_version,
        })
    }

    /// Whether this deployment can open sealed secrets.
    pub fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    /// The master-key version this store writes new ciphertext under.
    pub fn key_version(&self) -> u32 {
        self.key_version
    }

    /// The SQL boundary, for a caller that needs to compose a transaction.
    pub fn database(&self) -> &SecretDatabase {
        &self.db
    }

    /// Reads one secret, or `None` when the key is not stored.
    ///
    /// # Errors
    ///
    /// See [`open_from_storage`], plus [`SecretStoreError::Database`].
    pub async fn get(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
    ) -> Result<Option<SecretValue>, SecretStoreError> {
        let key_name = SecretKeyName::new(key_name)?;
        let Some(stored) = self.db.get_secret(key_name.as_str()).await? else {
            return Ok(None);
        };
        open_from_storage(self.cipher.as_ref(), purpose, &key_name, &stored).map(Some)
    }

    /// Whether one secret is stored for this purpose, without opening it.
    ///
    /// This is the metadata-only path: it reads the row's version marker and
    /// owning purpose and nothing else, so it answers correctly — and
    /// decrypts nothing — on a deployment that has no master key. That is what
    /// keeps a `has_*` projection truthful while a secret is still unreadable.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::Database`].
    pub async fn has(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
    ) -> Result<bool, SecretStoreError> {
        let key_name = SecretKeyName::new(key_name)?;
        let Some(metadata) = self.db.get_secret_metadata(key_name.as_str()).await? else {
            return Ok(false);
        };
        Ok(metadata.belongs_to(purpose.as_str()))
    }

    /// Creates one secret and reports whether this call created it.
    ///
    /// # Errors
    ///
    /// See [`seal_for_storage`], plus [`SecretStoreError::Database`].
    pub async fn put(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
        value: &[u8],
    ) -> Result<PutOutcome, SecretStoreError> {
        let key_name = SecretKeyName::new(key_name)?;
        let stored = seal_for_storage(self.cipher.as_ref(), purpose, key_name.as_str(), value)?;
        let created = self
            .db
            .put_secret(key_name.as_str(), &(&stored).into())
            .await?;
        Ok(if created {
            PutOutcome::Created
        } else {
            PutOutcome::AlreadyPresent
        })
    }

    /// Stores one secret, creating or replacing it as needed.
    ///
    /// This is the operation a settings form wants: "make the stored secret be
    /// this value" is one intent, not a create-or-update decision the caller has
    /// to get right. It is race-safe — a create that loses to a concurrent
    /// writer falls through to the replace rather than reporting a conflict the
    /// caller cannot act on — and it never leaves the key absent.
    ///
    /// Use [`Self::put`] or [`Self::rotate`] directly when the caller *does* need
    /// to distinguish creating from replacing.
    ///
    /// # Errors
    ///
    /// See [`seal_for_storage`], plus [`SecretStoreError::Database`].
    pub async fn write(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
        value: &[u8],
    ) -> Result<PutOutcome, SecretStoreError> {
        match self.put(purpose, key_name, value).await? {
            PutOutcome::Created => Ok(PutOutcome::Created),
            PutOutcome::AlreadyPresent => {
                self.rotate(purpose, key_name, value).await?;
                Ok(PutOutcome::AlreadyPresent)
            }
        }
    }

    /// Replaces the stored representation of an existing secret.
    ///
    /// A rotation never creates: a key that is not stored is
    /// [`SecretStoreError::SecretNotFound`].
    ///
    /// # Errors
    ///
    /// See [`seal_for_storage`], [`SecretStoreError::SecretNotFound`], and
    /// [`SecretStoreError::Database`].
    pub async fn rotate(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
        value: &[u8],
    ) -> Result<(), SecretStoreError> {
        let key_name = SecretKeyName::new(key_name)?;
        let stored = seal_for_storage(self.cipher.as_ref(), purpose, key_name.as_str(), value)?;
        let rotated = self
            .db
            .rotate_secret(key_name.as_str(), &(&stored).into())
            .await?;
        if rotated {
            Ok(())
        } else {
            Err(SecretStoreError::SecretNotFound)
        }
    }

    /// Clears one secret and reports whether this call cleared it.
    ///
    /// The purpose is an ownership check, not decoration: a sealed row owned by
    /// another feature is not clearable from here.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::PurposeMismatch`] and
    /// [`SecretStoreError::Database`].
    pub async fn delete(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
    ) -> Result<bool, SecretStoreError> {
        let key_name = SecretKeyName::new(key_name)?;
        let Some(stored) = self.db.get_secret(key_name.as_str()).await? else {
            return Ok(false);
        };
        if !stored.is_legacy_plaintext() && stored.purpose.as_deref() != Some(purpose.as_str()) {
            return Err(SecretStoreError::PurposeMismatch);
        }
        Ok(self.db.delete_secret(key_name.as_str()).await?)
    }

    /// Reads one secret, creating it from `candidate` when it is not stored.
    ///
    /// The only create-on-read path, and it is race-safe: if a concurrent writer
    /// created the key first, the create reports `AlreadyPresent` and the stored
    /// value is read instead of the candidate being written over it. The returned
    /// bytes are always the bytes the store holds, so two processes cannot end up
    /// signing with different values.
    ///
    /// # Errors
    ///
    /// See [`Self::get`] and [`Self::put`].
    pub async fn get_or_create(
        &self,
        purpose: SecretPurpose,
        key_name: &str,
        candidate: &[u8],
    ) -> Result<SecretValue, SecretStoreError> {
        SecretKeyName::new(key_name)?;
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
}
