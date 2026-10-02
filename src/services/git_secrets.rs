//! The internal Git secret writer seam (issue #681 work unit 4A3b-5).
//!
//! Three distinct reversible secrets back tracked Git access, and each has one
//! owner in the shared store's purpose catalogue and one reference column:
//! `Credential` → `GitProviderToken` / `credential_secret_key`, `AppPrivateKey`
//! → `GitHubAppPrivateKey` / `app_private_key_secret_key`, and `Webhook` →
//! `GitWebhookSigningSecret` / `signing_secret_key`. The mapping is closed at
//! both ends, so a credential cannot reach a webhook row, an App key cannot reach
//! the token column, and a value sealed under one purpose cannot be read through
//! another. [`GitSecretTarget::bind`] is the one place the pairing is resolved
//! and refuses the cross pairings before any storage is touched.
//!
//! A key name is built from a group-qualified record id —
//! `<group_id>.<connection_key>` or `<group_id>.<repository_key>` — hashed into
//! `g<group_id>.<sha256(record_id)>` under the purpose's prefix. Hashing keeps
//! the name bounded: a connection key is caller-supplied and may hold characters
//! a key name could never carry, while the longest result is 112 of the 128 bytes
//! a name may use. The group id stays readable because it is this application's
//! own integer; the record key stays out of the name because it is not.
//!
//! **Ordering and bounded failure.** The value is sealed by
//! [`SecretStore::write`] *before* the narrow reference update, because a
//! reference has to resolve to a row that exists. The consequence is bounded: if
//! that update then matches no row, the sealed row is left unreferenced, never
//! the reverse, and the previously referenced row is never cleared or
//! overwritten — the record keeps the key it already had, and a retried write
//! reseals the same key name and repoints the same reference. An orphan holds no
//! reference, discloses nothing without a master key, and is reclaimed by the
//! store's own cleanup. No value, key name, or record identifier from a failed
//! write appears in an error. `None` and empty bytes are Keep, so a caller with
//! no new value cannot clear one by omission.
//!
//! [`GitSecretWriter::seal`] is the seal-only half: it writes the value and
//! returns the reference it was stored under without moving any column, so a
//! path that is *creating* the record — where there is no row to attach to yet —
//! can obtain the deterministic reference first and insert the metadata and the
//! reference together in one statement. The create path never inserts an
//! uncredentialed row and then repoints it, and a failed seal leaves no row at
//! all. [`GitSecretWriter::write`] remains the attach-and-repoint operation for
//! a record that already exists.
//!
//! An internal seam: no provider call, no network, and no token transport. The
//! writer is reached only by internal setup; the reader half is reached by the
//! provider webhook ingress, which opens one registration's signing secret to
//! verify a signature and never logs or returns the value.
use anyhow::{Result, anyhow};
use context69_secret_crypto::SecretValue;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::Database,
    services::{
        secret_store::{SecretKeyName, SecretPurpose, SecretStore},
        settings::secrets::secret_error,
    },
};

/// Which purpose-bound secret a group-owned Git record owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitSecretSlot {
    /// A connection's personal access token, the read credential.
    Credential,
    /// A connection's GitHub App private key.
    AppPrivateKey,
    /// A repository's webhook signing secret.
    Webhook,
}

impl GitSecretSlot {
    /// The store purpose that owns this slot's value.
    const fn purpose(self) -> SecretPurpose {
        match self {
            Self::Credential => SecretPurpose::GitProviderToken,
            Self::AppPrivateKey => SecretPurpose::GitHubAppPrivateKey,
            Self::Webhook => SecretPurpose::GitWebhookSigningSecret,
        }
    }

    /// The single column this slot moves. Naming it lets a refusal say where a
    /// value was not written, without naming the value.
    const fn reference(self) -> &'static str {
        match self {
            Self::Credential => "git_provider_connections.credential_secret_key",
            Self::AppPrivateKey => "git_provider_connections.app_private_key_secret_key",
            Self::Webhook => "git_webhook_registrations.signing_secret_key",
        }
    }
}

/// Which group-owned record a Git secret belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitSecretTarget {
    /// A provider connection of `group_id`, named by its connection key.
    Connection {
        group_id: i64,
        connection_key: String,
    },
    /// A webhook registration of a repository source of `group_id`.
    Repository { group_id: i64, repository_key: Uuid },
}

impl GitSecretTarget {
    /// Resolves this record against one slot, refusing everything decidable
    /// without I/O so a rejected write reaches neither the store nor the database.
    fn bind(&self, slot: GitSecretSlot) -> Result<(SecretPurpose, SecretKeyName)> {
        let purpose = slot.purpose();
        let prefix = purpose
            .key_name_prefix()
            .ok_or_else(|| anyhow!("{purpose} is not a record-scoped purpose"))?;
        // The group id is part of the record id, so two groups never derive one
        // name for the same connection key. A blank key names no record.
        let (group_id, record_id) = match self {
            Self::Connection {
                group_id,
                connection_key,
            } => {
                if connection_key.trim().is_empty() {
                    return Err(anyhow!("git secret connection key must not be blank"));
                }
                (*group_id, format!("{group_id}.{connection_key}"))
            }
            Self::Repository {
                group_id,
                repository_key,
            } => (*group_id, format!("{group_id}.{repository_key}")),
        };
        if group_id <= 0 {
            return Err(anyhow!(
                "git secret group id must be a positive identity, found {group_id}"
            ));
        }
        if matches!(slot, GitSecretSlot::Webhook) != matches!(self, Self::Repository { .. }) {
            return Err(anyhow!(
                "{purpose} is not writable to a {} reference ({})",
                self.kind(),
                slot.reference()
            ));
        }
        let digest: String = Sha256::digest(record_id.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let key_name = SecretKeyName::with_prefix(prefix, &format!("g{group_id}.{digest}"))
            .map_err(|error| secret_error(purpose, error))?;
        Ok((purpose, key_name))
    }

    /// The record kind this target names, for a refusal that has to say where a
    /// value was not written.
    fn kind(&self) -> &'static str {
        match self {
            Self::Connection { .. } => "connection",
            Self::Repository { .. } => "repository",
        }
    }
}

/// What one write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitSecretWrite {
    /// No value was supplied, so neither the store nor the reference changed.
    Kept,
    /// The value was sealed under this key name. [`GitSecretWriter::seal`]
    /// stops there, leaving the name for a creating caller to insert with the
    /// row; [`GitSecretWriter::write`] also moves the existing record's
    /// reference to it.
    Stored(SecretKeyName),
}

/// The internal writer for Git provider credentials, App keys, and webhook
/// signing secrets.
#[derive(Clone)]
pub struct GitSecretWriter {
    db: Database,
    store: SecretStore,
}

impl GitSecretWriter {
    pub fn new(db: Database, store: SecretStore) -> Self {
        Self { db, store }
    }

    /// Seals one Git secret and returns the reference it was stored under,
    /// without moving any record's column.
    ///
    /// `None` and empty bytes are Keep: nothing is written and the caller gets
    /// [`GitSecretWrite::Kept`]. A non-empty value is sealed under the slot's
    /// purpose at the exact key name [`GitSecretTarget::bind`] derives, so a
    /// caller that is creating a record can obtain the reference *before* the
    /// row exists and insert the two together — the create path never writes an
    /// uncredentialed row and then repoints it. No reference column is touched.
    ///
    /// # Errors
    ///
    /// A refused group id, record key, or slot/record pairing, or a store
    /// failure. The error names the purpose, never the value or the key name.
    pub async fn seal(
        &self,
        target: &GitSecretTarget,
        slot: GitSecretSlot,
        value: Option<&[u8]>,
    ) -> Result<GitSecretWrite> {
        let (purpose, key_name) = target.bind(slot)?;
        let Some(value) = value.filter(|bytes| !bytes.is_empty()) else {
            return Ok(GitSecretWrite::Kept);
        };
        self.seal_value(purpose, key_name.as_str(), value).await?;
        Ok(GitSecretWrite::Stored(key_name))
    }

    /// Writes a bound value to the store, mapping a failure to the purpose
    /// both public methods report through.
    async fn seal_value(&self, purpose: SecretPurpose, key_name: &str, value: &[u8]) -> Result<()> {
        self.store
            .write(purpose, key_name, value)
            .await
            .map_err(|error| secret_error(purpose, error))?;
        Ok(())
    }

    /// Seals one Git secret and points the record at it.
    ///
    /// Delegates the seal to [`GitSecretWriter::seal`] first, so a value is
    /// always written before any reference moves — a reference has to resolve
    /// to a row that exists. `None` and empty bytes are Keep. A non-empty value
    /// is written to the store immediately, then one narrow reference update
    /// moves only that column, preserving `disabled_at` on a connection and
    /// `active` on a registration. An update that matches no row is an error
    /// naming the purpose and column.
    ///
    /// # Errors
    ///
    /// A refused group id, record key, or slot/record pairing; a store failure;
    /// or a reference update that matched no row.
    pub async fn write(
        &self,
        target: &GitSecretTarget,
        slot: GitSecretSlot,
        value: Option<&[u8]>,
    ) -> Result<GitSecretWrite> {
        let GitSecretWrite::Stored(key_name) = self.seal(target, slot, value).await? else {
            return Ok(GitSecretWrite::Kept);
        };
        if !self.repoint(slot, target, key_name.as_str()).await? {
            return Err(anyhow!(
                "{} was sealed but no {} of this group was updated",
                slot.purpose(),
                slot.reference()
            ));
        }
        Ok(GitSecretWrite::Stored(key_name))
    }

    /// Moves the one reference column this slot owns.
    ///
    /// Every arm is a narrow statement. The broad connection upsert is never
    /// reached, because its conflict clause clears `disabled_at` and a rotation
    /// must not re-enable a connection an operator disabled.
    async fn repoint(
        &self,
        slot: GitSecretSlot,
        target: &GitSecretTarget,
        secret_key: &str,
    ) -> Result<bool> {
        match (slot, target) {
            (
                GitSecretSlot::Credential,
                GitSecretTarget::Connection {
                    group_id,
                    connection_key,
                },
            ) => {
                self.db
                    .set_git_connection_credential_secret_key(*group_id, connection_key, secret_key)
                    .await
            }
            (
                GitSecretSlot::AppPrivateKey,
                GitSecretTarget::Connection {
                    group_id,
                    connection_key,
                },
            ) => {
                self.db
                    .set_git_connection_app_private_key_secret_key(
                        *group_id,
                        connection_key,
                        secret_key,
                    )
                    .await
            }
            (
                GitSecretSlot::Webhook,
                GitSecretTarget::Repository {
                    group_id,
                    repository_key,
                },
            ) => {
                self.db
                    .set_git_webhook_signing_secret_key(*group_id, *repository_key, secret_key)
                    .await
            }
            (slot, target) => Err(anyhow!(
                "{} is not writable to a {} reference ({})",
                slot.purpose(),
                target.kind(),
                slot.reference()
            )),
        }
    }
}

/// The internal reader for the Git provider secrets the writer seals.
///
/// It derives the record-scoped key name through the same
/// [`GitSecretTarget::bind`] the writer uses, so a reader cannot look up a
/// different name than a writer stored, and it opens the value under the slot's
/// own purpose. A missing row is `None`; a sealed row that cannot be opened is
/// the store's error and is never downgraded to a plaintext fallback.
#[derive(Clone)]
pub struct GitSecretReader {
    store: SecretStore,
}

impl GitSecretReader {
    pub fn new(store: SecretStore) -> Self {
        Self { store }
    }

    /// Opens the value `target` owns for `slot`, or `None` when nothing is
    /// stored under the derived name.
    ///
    /// # Errors
    ///
    /// A refused slot/record pairing, or a store failure — including a sealed
    /// row that this deployment cannot open. The error names the purpose, never
    /// the value.
    pub async fn read(
        &self,
        target: &GitSecretTarget,
        slot: GitSecretSlot,
    ) -> Result<Option<SecretValue>> {
        let (purpose, key_name) = target.bind(slot)?;
        self.store
            .get(purpose, key_name.as_str())
            .await
            .map_err(|error| secret_error(purpose, error))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GitSecretSlot::{AppPrivateKey, Credential, Webhook},
        GitSecretTarget, SecretPurpose,
    };
    use context69_secret_store::LONGEST_KEY_NAME;
    use std::collections::BTreeSet;
    use uuid::Uuid;

    const CONNECTION_KEY: &str = "github-app";

    fn connection(group_id: i64) -> GitSecretTarget {
        GitSecretTarget::Connection {
            group_id,
            connection_key: CONNECTION_KEY.to_string(),
        }
    }

    fn repository() -> GitSecretTarget {
        GitSecretTarget::Repository {
            group_id: 7,
            repository_key: Uuid::nil(),
        }
    }

    /// The three slots against the record that owns each and the purpose that
    /// owns the value, so one loop can assert the mappings stay separate.
    fn bindings() -> [(super::GitSecretSlot, GitSecretTarget, SecretPurpose); 3] {
        [
            (Credential, connection(7), SecretPurpose::GitProviderToken),
            (
                AppPrivateKey,
                connection(7),
                SecretPurpose::GitHubAppPrivateKey,
            ),
            (
                Webhook,
                repository(),
                SecretPurpose::GitWebhookSigningSecret,
            ),
        ]
    }

    #[test]
    fn each_slot_binds_its_own_bounded_group_qualified_key_name() {
        let (mut names, mut columns) = (BTreeSet::new(), BTreeSet::new());
        for (slot, target, purpose) in bindings() {
            let (bound, name) = target.bind(slot).expect("a bound pair resolves");
            // `<purpose prefix>g<group id>.<sha256(record id)>`
            let prefix = purpose.key_name_prefix().expect("record-scoped");
            let record = name
                .as_str()
                .strip_prefix(prefix)
                .expect("a key name sits under its purpose's prefix");
            let (group, digest) = record
                .split_once('.')
                .expect("the name ends in the hashed record segment");
            assert_eq!(bound, purpose, "a slot owns exactly one purpose");
            assert_eq!(
                (group, digest.len()),
                ("g7", 64),
                "a key name is its purpose prefix over g<group>.<sha256(record id)>: {name}"
            );
            assert!(
                digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && name.as_str().len() <= LONGEST_KEY_NAME
                    && !name.as_str().contains(CONNECTION_KEY),
                "a hex digest keeps the name in charset and in bound, and a \
                 caller-supplied record key never appears in one: {name}"
            );
            assert_eq!(
                target.bind(slot).expect("stable").1.as_str(),
                name.as_str(),
                "the same record always yields the same key name"
            );
            names.insert(name.as_str().to_string());
            columns.insert(slot.reference().to_string());
        }
        assert_eq!((names.len(), columns.len()), (3, 3), "no shared namespace");
        // The namespaces are the catalogue's, so only these three purposes may
        // write into them.
        let mine = [Credential, AppPrivateKey, Webhook].map(super::GitSecretSlot::purpose);
        assert_eq!(
            SecretPurpose::ALL
                .iter()
                .filter(|p| mine.contains(p))
                .count(),
            3,
            "no other purpose may share a Git secret key namespace"
        );
        assert_ne!(
            connection(8).bind(Credential).expect("bound").1.as_str(),
            connection(7).bind(Credential).expect("bound").1.as_str(),
            "the group id is part of the name, so two groups never share one"
        );
    }

    #[test]
    fn an_invalid_group_record_or_pairing_is_refused_before_any_storage() {
        for group_id in [0, -1, i64::MIN] {
            let error = connection(group_id)
                .bind(Credential)
                .expect_err("a non-positive group owns no record");
            assert!(error.to_string().contains("positive identity"), "{error}");
        }
        for key in ["", " ", "\t"] {
            assert!(
                GitSecretTarget::Connection {
                    group_id: 7,
                    connection_key: key.to_string(),
                }
                .bind(Credential)
                .is_err(),
                "a blank connection key names no record: {key:?}"
            );
        }
        // Every cross pairing is refused, and the refusal names the purpose and
        // the reference it was not written to — never a value or a record key.
        let repository = repository();
        for (slot, target) in [
            (Webhook, connection(7)),
            (Credential, repository.clone()),
            (AppPrivateKey, repository),
        ] {
            let message = target
                .bind(slot)
                .expect_err("a cross pairing is refused")
                .to_string();
            assert!(
                message.contains(slot.purpose().as_str())
                    && message.contains(slot.reference())
                    && !message.contains(CONNECTION_KEY),
                "{message}"
            );
        }
    }
}
