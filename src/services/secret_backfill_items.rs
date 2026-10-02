//! The reversible-secret backfill's worklist: what a run migrates, from where.
//!
//! Everything here *builds* the items a run walks and resolves which purpose owns a
//! stored key; [`super::secret_backfill_migrate`] is what moves them. The split is by
//! job: this module knows the shape of the worklist — the legacy columns the
//! application already reads, the connections whose identity owns a sealed DSN, the
//! store rows still in the legacy representation, and the catalogue mapping between a
//! key name and its purpose — and nothing about what a run does with an item.
//!
//! Legacy values are read raw, through the same projections their own consumers read,
//! so a run never sources a migration from a stored value: the store is what a legacy
//! value is migrated *to*, never where it comes from.

use anyhow::{Context, Result};

use crate::{
    db::{Database, LegacySecretColumn, StoredSourceConnection},
    services::{
        secret_store::{SecretPurpose, key_names},
        sync::source_connection_database_url_key,
    },
    support::normalize::normalize_optional_string,
};

/// The singleton credential categories, in migration order. Every value is read raw,
/// from the projection its own consumer reads it through and from the shared `llm`
/// row's own read; a blank value is not a credential, because the normalization the
/// consumers apply decides that here too.
pub(super) async fn singleton_items(db: &Database) -> Result<Vec<LegacyItem>> {
    let (embedding, s3) = match db.get_runtime_settings().await? {
        Some(settings) => (
            settings.embedding.api_key,
            settings.file_library.s3.map(|s3| s3.secret_key),
        ),
        None => (None, None),
    };
    Ok([
        legacy(
            SecretPurpose::EmbeddingApiKey,
            key_names::EMBEDDING_API_KEY,
            embedding,
            LegacySecretColumn::RuntimeEmbedding,
        ),
        legacy(
            SecretPurpose::SearchApiKey,
            key_names::SEARCH_API_KEY,
            db.get_search_settings()
                .await?
                .and_then(|settings| settings.api_key),
            LegacySecretColumn::Search,
        ),
        legacy(
            SecretPurpose::DoclingVlmApiKey,
            key_names::DOCLING_VLM_API_KEY,
            db.get_docling_settings()
                .await?
                .and_then(|settings| settings.api_key),
            LegacySecretColumn::Docling,
        ),
        legacy(
            SecretPurpose::TranslationProviderApiKey,
            key_names::TRANSLATION_PROVIDER_API_KEY,
            db.llm_provider_api_key().await?.flatten(),
            LegacySecretColumn::LlmProvider,
        ),
        // Sealed like the rest, cleared like none of them: the runtime S3 secret key
        // belongs to a projection whose readers need every field to be present.
        retained(
            SecretPurpose::RuntimeS3SecretKey,
            key_names::RUNTIME_S3_SECRET_KEY,
            s3,
        ),
    ]
    .into())
}

/// One source connection's database URL, sealed under its stable identity. Its legacy
/// DSN is retained: the column is `NOT NULL` with a non-blank check, so this phase
/// repairs the reference and leaves the value for the column removal.
pub(super) fn source_item(connection: &StoredSourceConnection) -> Result<LegacyItem> {
    let key_name = source_connection_database_url_key(connection.connection_key)?;
    Ok(LegacyItem {
        purpose: SecretPurpose::SourceConnectionDatabaseUrl,
        key_name: key_name.as_str().to_string(),
        value: legacy_bytes(Some(connection.database_url.clone())),
        from_legacy_column: true,
        disposition: Disposition::SourceReference {
            name: connection.name.clone(),
            stored: connection.database_url_secret_key.clone(),
        },
    })
}

/// One store row still in the legacy representation, mapped through the catalogue.
///
/// `None` when no purpose owns the key. The row is then left exactly as it is: the
/// catalogue is exhaustive, so an unmatched key is something this build does not know,
/// never a licence to invent an owner for it.
pub(super) async fn unversioned_item(
    store: &crate::services::secret_store::SecretStore,
    key_name: &str,
) -> Result<Option<LegacyItem>> {
    let Some(purpose) = catalogue_purpose(key_name) else {
        return Ok(None);
    };
    let value = store
        .get(purpose, key_name)
        .await
        .with_context(|| format!("{purpose} legacy store row could not be read"))?
        .map(|value| value.into_inner());
    Ok(Some(LegacyItem {
        purpose,
        key_name: key_name.to_string(),
        value,
        from_legacy_column: false,
        // Its own legacy representation is this row: rewriting it sealed is the whole
        // migration, so nothing is cleared afterwards.
        disposition: Disposition::Retain,
    }))
}

/// The purpose that owns a stored key, from the catalogue alone: an exact singleton
/// name first, then a record-scoped prefix. Nothing else matches, so a key is only
/// ever migrated under a purpose this build declares.
fn catalogue_purpose(key_name: &str) -> Option<SecretPurpose> {
    if let Some(purpose) = SecretPurpose::ALL
        .into_iter()
        .find(|purpose| purpose.singleton_key_name() == Some(key_name))
    {
        return Some(purpose);
    }
    SecretPurpose::ALL.into_iter().find(|purpose| {
        purpose
            .key_name_prefix()
            .is_some_and(|prefix| key_name.starts_with(prefix) && key_name.len() > prefix.len())
    })
}

/// One legacy value awaiting its sealed representation.
pub(super) struct LegacyItem {
    /// The catalogue purpose that will own the sealed value.
    pub(super) purpose: SecretPurpose,
    /// The catalogue key name, resolved once when the item is built.
    pub(super) key_name: String,
    /// The raw legacy bytes, or `None` when there is nothing to migrate.
    pub(super) value: Option<Vec<u8>>,
    /// Whether the value came from a legacy column rather than from a store row that
    /// is itself still in the legacy representation.
    pub(super) from_legacy_column: bool,
    /// What the run may do with the legacy value once the store holds it.
    pub(super) disposition: Disposition,
}

impl LegacyItem {
    /// Whether this item's legacy value still has to change. A sealed value whose
    /// legacy column is retained, or whose reference already names the sealed key, is
    /// finished work: it is neither rewritten nor counted against the run's budget,
    /// which is what keeps a converged deployment from looking endless.
    pub(super) fn needs_cleanup(&self) -> bool {
        match &self.disposition {
            Disposition::Clear(_) => true,
            Disposition::Retain => false,
            Disposition::SourceReference { stored, .. } => {
                stored.as_deref() != Some(self.key_name.as_str())
            }
        }
    }
}

/// What one run does with a legacy value after its sealed write has verified.
pub(super) enum Disposition {
    /// A standalone nullable credential column, nulled after the verified write.
    Clear(LegacySecretColumn),
    /// Retained until the column-removal migration: a `NOT NULL` DSN column, a
    /// settings column an all-fields-required projection still reads, or a store row
    /// whose own legacy representation the write already replaced.
    Retain,
    /// A source connection, whose reference is pointed at the sealed value. `name` is
    /// the connection's user-facing identifier, which scopes the one statement that may
    /// move its reference.
    SourceReference {
        /// The connection's user-facing identifier.
        name: String,
        /// The reference the row carries today, so an already-repaired one is
        /// recognized instead of rewritten.
        stored: Option<String>,
    },
}

/// A singleton category whose legacy column this phase may null.
fn legacy(
    purpose: SecretPurpose,
    key_name: &'static str,
    value: Option<String>,
    column: LegacySecretColumn,
) -> LegacyItem {
    LegacyItem {
        purpose,
        key_name: key_name.to_string(),
        value: legacy_bytes(value),
        from_legacy_column: true,
        disposition: Disposition::Clear(column),
    }
}

/// A singleton category whose legacy column stays until the column removal.
fn retained(purpose: SecretPurpose, key_name: &'static str, value: Option<String>) -> LegacyItem {
    LegacyItem {
        purpose,
        key_name: key_name.to_string(),
        value: legacy_bytes(value),
        from_legacy_column: true,
        disposition: Disposition::Retain,
    }
}

/// A legacy column's bytes, under the normalization its consumers apply. A blank or
/// absent value is no credential at all, and a surrounding blank is not part of one:
/// sealing what a consumer would have served keeps the sealed value and the legacy
/// column in agreement right up to the moment the column goes.
fn legacy_bytes(legacy: Option<String>) -> Option<Vec<u8>> {
    normalize_optional_string(legacy).map(String::into_bytes)
}

#[cfg(test)]
mod tests {
    use super::catalogue_purpose;
    use crate::services::secret_store::{SecretPurpose, key_names};

    /// Every catalogue key resolves back to the purpose that owns it, through an exact
    /// singleton name or a record-scoped prefix, and nothing else resolves.
    #[test]
    fn a_stored_key_is_migrated_only_under_a_purpose_the_catalogue_declares() {
        for purpose in SecretPurpose::ALL {
            if let Some(singleton) = purpose.singleton_key_name() {
                assert_eq!(catalogue_purpose(singleton), Some(purpose));
            }
            if let Some(prefix) = purpose.key_name_prefix() {
                assert_eq!(catalogue_purpose(&format!("{prefix}record")), Some(purpose));
                assert_eq!(
                    catalogue_purpose(prefix),
                    None,
                    "a bare prefix names no record: {prefix}"
                );
            }
        }
        for unmapped in [
            key_names::EMBEDDING_API_KEY.to_uppercase().as_str(),
            "embedding_api_key_v2",
            "operator_note",
            "",
        ] {
            assert_eq!(
                catalogue_purpose(unmapped),
                None,
                "a key the catalogue does not declare is never guessed: {unmapped}"
            );
        }
    }
}
