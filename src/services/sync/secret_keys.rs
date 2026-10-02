//! The store key one source connection's database URL is sealed under.
//!
//! This is the one place that mapping exists. Every writer *and* the
//! reversible-secret backfill resolve a connection's key through it, so a backfilled
//! connection can only ever land on the key the application would have written.

use anyhow::Result;
use uuid::Uuid;

use crate::services::{
    secret_store::{SecretKeyName, SecretPurpose, key_names},
    settings::secrets::secret_error,
};

/// The store key one source connection's database URL is sealed under.
///
/// The connection name never forms a key: it is user-chosen, renameable, and reusable
/// after a delete, so a value written under one could later be served to a different
/// connection. The stable `connection_key` is the identity the store row belongs to,
/// so the same connection always resolves the same key.
///
/// # Errors
///
/// [`SecretStoreError::InvalidKeyName`], reported with its owning purpose, when the
/// identity cannot form a key name. A caller-supplied record that could not be
/// embedded is refused here, before any storage is touched.
pub(crate) fn source_connection_database_url_key(connection_key: Uuid) -> Result<SecretKeyName> {
    SecretKeyName::with_prefix(
        key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX,
        &connection_key.to_string(),
    )
    .map_err(|error| secret_error(SecretPurpose::SourceConnectionDatabaseUrl, error))
}

#[cfg(test)]
mod tests {
    use super::source_connection_database_url_key;
    use crate::services::secret_store::{SecretPurpose, key_names};
    use uuid::Uuid;

    /// A store key is the stable identity and nothing else: never the user-chosen
    /// name, never the DSN, and always the same key for the same identity.
    #[test]
    fn a_store_key_is_the_stable_uuid_and_never_the_name_or_the_dsn() {
        let connection_key =
            Uuid::parse_str("2f6d1f0e-2b7c-4a1f-9a3d-5c8e7b0a1d22").expect("a uuid");
        let key =
            source_connection_database_url_key(connection_key).expect("a connection key is valid");

        assert_eq!(
            key.as_str(),
            format!(
                "{}{connection_key}",
                key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX
            )
        );
        assert!(
            key.as_str().starts_with("source_connection.database_url."),
            "the key is namespaced by the category that owns it"
        );
        assert!(
            !key.as_str().contains("primary") && !key.as_str().contains("postgres"),
            "neither a user-chosen name nor the DSN may appear in a store key"
        );
        assert_eq!(
            key.as_str(),
            source_connection_database_url_key(connection_key)
                .expect("the same identity always yields the same key")
                .as_str(),
            "the key is derived from the identity, so it is stable across saves"
        );
    }

    /// The mapping resolves to the purpose that owns a connection's DSN, and that
    /// purpose owns no other key namespace.
    #[test]
    fn a_source_connection_dsn_is_a_record_scoped_purpose() {
        assert_eq!(
            SecretPurpose::SourceConnectionDatabaseUrl.key_name_prefix(),
            Some(key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX)
        );
        assert_eq!(
            SecretPurpose::SourceConnectionDatabaseUrl.singleton_key_name(),
            None
        );
        for purpose in SecretPurpose::ALL
            .into_iter()
            .filter(|purpose| *purpose != SecretPurpose::SourceConnectionDatabaseUrl)
        {
            assert_ne!(
                purpose.key_name_prefix(),
                Some(key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX),
                "{purpose} must not write into the source-connection key namespace"
            );
        }
    }
}
