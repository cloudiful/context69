use super::*;
use crate::{
    db::NewSourceConnection,
    domain_errors::DomainError,
    services::{
        secret_store::{SecretKeyName, SecretPurpose, SecretStore, key_names},
        settings::secrets::{resolve_stored_or_legacy, secret_error, secret_is_present},
    },
    support::normalize::normalize_optional_string,
};
use uuid::Uuid;

impl SyncService {
    /// The configured connections, with presence reported without opening any
    /// sealed value.
    pub async fn list_source_connections(&self) -> Result<Vec<SourceConnectionResponse>> {
        let statuses = self.source_connection_statuses.read().await.clone();
        let secrets = self.source_secrets();
        let mut responses = Vec::new();
        for connection in self.db.list_source_connections().await? {
            // Metadata-only presence: a connection whose sealed DSN this
            // deployment cannot open is still configured, so the projection
            // stays truthful without a master key.
            let has_database_url = secrets.is_present(&connection).await?;
            let name = connection.name;
            responses.push(SourceConnectionResponse {
                name: name.clone(),
                has_database_url,
                origin_status: statuses
                    .get(&name)
                    .map(|status| status.status.clone())
                    .unwrap_or(SourceOriginStatusKind::Unknown),
                origin_message: statuses
                    .get(&name)
                    .and_then(|status| status.message.clone()),
            });
        }
        Ok(responses)
    }

    pub async fn upsert_source_connection(
        &self,
        input: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse> {
        let pending = self
            .resolve_source_connection(&input.name, input.database_url.clone())
            .await?;
        let saved = self.persist_source_connection(&pending).await?;
        self.reload_sources().await?;
        Ok(SourceConnectionResponse {
            name: saved.name,
            has_database_url: !saved.database_url.trim().is_empty(),
            origin_status: SourceOriginStatusKind::Unknown,
            origin_message: None,
        })
    }

    pub async fn delete_source_connection(&self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(
                DomainError::invalid_argument("source connection name must not be empty").into(),
            );
        }

        for source in self.source_store.list_source_configs().await? {
            if source.connection == name {
                return Err(DomainError::conflict(format!(
                    "source connection {name} is referenced by source {}",
                    source.key
                ))
                .into());
            }
        }

        let Some(existing) = self.db.get_source_connection(name).await? else {
            return Err(DomainError::not_found(format!("unknown source connection {name}")).into());
        };
        // The sealed value is cleared first. Failing here leaves the connection
        // working and reported, whereas clearing after the delete would leave a
        // secret row nothing points at, with no way to find it again.
        if let Some(secret_key) = existing.database_url_secret_key.as_deref() {
            self.source_secrets().clear(secret_key).await?;
        }
        let deleted = self.db.delete_source_connection(name).await?;
        if !deleted {
            return Err(DomainError::not_found(format!("unknown source connection {name}")).into());
        }
        self.reload_sources().await?;
        Ok(())
    }
}

/// The source-connection view of the shared store: one database URL per
/// connection, sealed under the connection's own stable UUID.
///
/// The connection name never forms a key. It is user-chosen, renameable, and
/// reusable after a delete, so a value written under one could later be served
/// to a different connection.
pub(super) struct SourceConnectionSecrets {
    store: SecretStore,
}

impl SourceConnectionSecrets {
    pub(super) fn new(store: SecretStore) -> Self {
        Self { store }
    }

    /// The store key one connection's database URL is sealed under.
    fn key_name(connection_key: Uuid) -> Result<SecretKeyName> {
        SecretKeyName::with_prefix(
            key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX,
            &connection_key.to_string(),
        )
        .map_err(|error| secret_error(SecretPurpose::SourceConnectionDatabaseUrl, error))
    }

    /// The store row a stored connection points at, if it has one yet.
    ///
    /// A NULL reference is the transition state: the connection has not been
    /// written through the store, so its database URL is still the legacy
    /// column's and nothing is stored to be opened.
    fn stored_key(connection: &StoredSourceConnection) -> Option<&str> {
        connection.database_url_secret_key.as_deref()
    }

    /// The database URL in effect: the sealed value when the store holds one,
    /// otherwise the legacy column.
    ///
    /// Fails closed — a store row that exists and cannot be opened is an error,
    /// never a silent return to the legacy column.
    pub(super) async fn resolve(
        &self,
        connection: &StoredSourceConnection,
    ) -> Result<Option<String>> {
        let legacy = connection.database_url.clone();
        match Self::stored_key(connection) {
            Some(secret_key) => {
                resolve_stored_or_legacy(
                    &self.store,
                    SecretPurpose::SourceConnectionDatabaseUrl,
                    secret_key,
                    Some(legacy),
                )
                .await
            }
            None => Ok(normalize_optional_string(Some(legacy))),
        }
    }

    /// Whether a database URL is configured, without opening one.
    async fn is_present(&self, connection: &StoredSourceConnection) -> Result<bool> {
        match Self::stored_key(connection) {
            Some(secret_key) => {
                secret_is_present(
                    &self.store,
                    SecretPurpose::SourceConnectionDatabaseUrl,
                    secret_key,
                    Some(&connection.database_url),
                )
                .await
            }
            None => Ok(normalize_optional_string(Some(connection.database_url.clone())).is_some()),
        }
    }

    /// Seals one database URL, creating the store row or replacing it.
    async fn write(&self, secret_key: &SecretKeyName, database_url: &str) -> Result<()> {
        self.store
            .write(
                SecretPurpose::SourceConnectionDatabaseUrl,
                secret_key.as_str(),
                database_url.as_bytes(),
            )
            .await
            .map(|_| ())
            .map_err(|error| secret_error(SecretPurpose::SourceConnectionDatabaseUrl, error))
    }

    /// Clears one connection's store row. A row that is already gone is not an
    /// error: the value is gone either way.
    async fn clear(&self, secret_key: &str) -> Result<()> {
        self.store
            .delete(SecretPurpose::SourceConnectionDatabaseUrl, secret_key)
            .await
            .map(|_| ())
            .map_err(|error| secret_error(SecretPurpose::SourceConnectionDatabaseUrl, error))
    }
}

/// Creates or updates one source connection with its database URL sealed.
///
/// The identity is read back first so a re-save keeps the key an existing sealed
/// value is stored under. The store is written before the row, because a
/// reference has to resolve to a row that exists, and because a failed seal must
/// not leave the legacy column holding a value the store would never serve.
pub(crate) async fn save_source_connection(
    db: &Database,
    store: &SecretStore,
    name: &str,
    database_url: &str,
) -> Result<StoredSourceConnection> {
    let secrets = SourceConnectionSecrets::new(store.clone());
    let connection_key = db
        .get_source_connection(name)
        .await?
        .map_or_else(Uuid::new_v4, |existing| existing.connection_key);
    let secret_key = SourceConnectionSecrets::key_name(connection_key)?;
    secrets.write(&secret_key, database_url).await?;

    db.save_source_connection(&NewSourceConnection {
        connection_key,
        name: name.to_string(),
        database_url: database_url.to_string(),
        database_url_secret_key: secret_key.as_str().to_string(),
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{SourceConnectionSecrets, save_source_connection};
    use crate::{
        db::StoredSourceConnection,
        services::secret_store::{SecretPurpose, SecretStore, key_names},
    };
    use uuid::Uuid;

    /// A store over a pool nothing listens on. The paths asserted here either
    /// decide before any statement runs or fail on the connection attempt, so no
    /// server is needed to tell them apart; the short acquire timeout keeps that
    /// attempt quick.
    fn unreached_store() -> SecretStore {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(250))
            .connect_lazy("postgres://127.0.0.1:1/unused")
            .expect("a lazy pool needs no server");
        SecretStore::new(context69_secret_store::SecretDatabase::new(pool), None, 1)
            .expect("an unkeyed store cannot fail to build")
    }

    fn stored(
        name: &str,
        database_url: &str,
        database_url_secret_key: Option<&str>,
    ) -> StoredSourceConnection {
        StoredSourceConnection {
            name: name.to_string(),
            connection_key: Uuid::nil(),
            database_url: database_url.to_string(),
            database_url_secret_key: database_url_secret_key.map(str::to_string),
        }
    }

    #[test]
    fn a_store_key_is_the_stable_uuid_and_never_the_name_or_the_dsn() {
        let connection_key =
            Uuid::parse_str("2f6d1f0e-2b7c-4a1f-9a3d-5c8e7b0a1d22").expect("a uuid");
        let key = SourceConnectionSecrets::key_name(connection_key)
            .expect("a connection key is a valid store key");

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
            SourceConnectionSecrets::key_name(connection_key)
                .expect("the same identity always yields the same key")
                .as_str(),
            "the key is derived from the identity, so it is stable across saves"
        );
    }

    #[tokio::test]
    async fn a_connection_without_a_reference_is_read_from_its_legacy_column() {
        // The store is never reached: there is no reference, so there is no row
        // to open, and the legacy column is the whole value.
        let secrets = SourceConnectionSecrets::new(unreached_store());
        let connection = stored("primary", "postgres://legacy/one", None);
        assert_eq!(SourceConnectionSecrets::stored_key(&connection), None);
        assert_eq!(
            secrets
                .resolve(&connection)
                .await
                .expect("an unmigrated connection needs no store access"),
            Some("postgres://legacy/one".to_string())
        );
        assert!(
            secrets
                .is_present(&connection)
                .await
                .expect("an unmigrated connection needs no store access")
        );
    }

    #[tokio::test]
    async fn a_connection_with_a_reference_uses_exactly_the_persisted_key() {
        let connection_key = Uuid::new_v4();
        let expected = SourceConnectionSecrets::key_name(connection_key)
            .expect("a connection key is a valid store key");
        let mut connection = stored("primary", "postgres://legacy/one", Some(expected.as_str()));
        connection.connection_key = connection_key;
        assert_eq!(
            SourceConnectionSecrets::stored_key(&connection),
            Some(expected.as_str()),
            "the reference the row carries is the key that is opened"
        );

        // A sealed row answers for itself, so presence is decided from the store
        // rather than from the legacy column. The store here is unreachable, so
        // this asserts the store is what is consulted — and that a failure to
        // reach it is reported instead of answered from the column.
        let error = SourceConnectionSecrets::new(unreached_store())
            .is_present(&connection)
            .await
            .expect_err("presence must consult the store for a referenced connection");
        assert!(
            error.to_string().contains("source_connection.database_url"),
            "the failure names the purpose and never a value: {error}"
        );
    }

    #[test]
    fn a_source_connection_dsn_is_a_record_scoped_purpose() {
        // The category owns one secret per record, so it has a prefix rather than
        // a fixed key name, and no two purposes may share a key namespace.
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

    /// The whole transition for one connection, against a migrated scratch
    /// database: the writer seals the DSN and mirrors it into the legacy column,
    /// the configured store opens it, a store without the master key fails
    /// closed instead of serving the plaintext column, presence stays answerable
    /// on that deployment, and clearing removes the sealed value.
    ///
    /// Skipped unless `CONTEXT69_TEST_DATABASE_URL` names one. Every row it
    /// writes is keyed by a fresh UUID and removed again.
    #[tokio::test]
    async fn a_saved_connection_is_sealed_mirrored_and_fail_closed() {
        let Ok(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL") else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the store round trip");
            return;
        };
        let db = crate::db::Database::connect(&url)
            .await
            .expect("connect test database");
        let name = format!("source-connection-secret-{}", Uuid::new_v4());
        // Synthetic, and only ever compared against: no real origin is reached.
        let database_url = "postgres://user:pass@source.invalid/db";

        // A per-run key, so nothing here is a credential of any deployment.
        let master_key = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            Uuid::new_v4().into_bytes().repeat(2),
        );
        let keyed = store(&db, Some(master_key.as_str()));
        let unkeyed = store(&db, None);

        let saved = save_source_connection(&db, &keyed, &name, database_url)
            .await
            .expect("save the connection");
        let secret_key = saved
            .database_url_secret_key
            .clone()
            .expect("a saved connection references its store row");
        assert_eq!(
            saved.database_url, database_url,
            "the legacy column keeps receiving the value for the whole transition"
        );
        assert_eq!(
            SourceConnectionSecrets::new(keyed.clone())
                .resolve(&saved)
                .await
                .expect("the configured store opens its own sealed value")
                .as_deref(),
            Some(database_url)
        );

        let failure = SourceConnectionSecrets::new(unkeyed.clone())
            .resolve(&saved)
            .await
            .expect_err("a sealed value must not fall back to the legacy column");
        assert!(
            failure.to_string().contains("secret_store.master_key"),
            "the failure is a configuration failure an operator can act on: {failure}"
        );
        assert!(
            !failure.to_string().contains(database_url),
            "the failure carries no value"
        );
        assert!(
            SourceConnectionSecrets::new(unkeyed)
                .is_present(&saved)
                .await
                .expect("presence is answered from metadata, without a master key"),
            "a connection is reported configured even where it cannot be opened"
        );

        SourceConnectionSecrets::new(keyed.clone())
            .clear(&secret_key)
            .await
            .expect("clear the sealed value");
        assert!(
            keyed
                .get(SecretPurpose::SourceConnectionDatabaseUrl, &secret_key)
                .await
                .expect("read the store")
                .is_none(),
            "clearing removes the sealed value itself"
        );
        assert_eq!(
            db.get_source_connection(&name)
                .await
                .expect("read the connection")
                .expect("the record is still there")
                .database_url_secret_key,
            None,
            "a connection whose value was cleared carries no store reference"
        );
        assert!(db.delete_source_connection(&name).await.expect("delete"));
    }

    /// A store over the scratch pool, with or without a deployment master key.
    fn store(db: &crate::db::Database, master_key: Option<&str>) -> SecretStore {
        SecretStore::new(
            context69_secret_store::SecretDatabase::new(db.pool().clone()),
            master_key,
            1,
        )
        .expect("a store builds from any master key or its absence")
    }
}
