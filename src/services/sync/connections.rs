use super::secret_keys::source_connection_database_url_key;
use super::*;
use crate::{
    db::NewSourceConnection,
    domain_errors::DomainError,
    services::{
        secret_store::{SecretKeyName, SecretPurpose, SecretStore},
        settings::secrets::{resolve_stored, secret_error, secret_is_present},
    },
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
        // The store was written before the row, so the saved reference resolves to
        // a credential this deployment is configured with.
        let has_database_url = self.source_secrets().is_present(&saved).await?;
        self.reload_sources().await?;
        Ok(SourceConnectionResponse {
            name: saved.name,
            has_database_url,
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
pub(super) struct SourceConnectionSecrets {
    store: SecretStore,
}

impl SourceConnectionSecrets {
    pub(super) fn new(store: SecretStore) -> Self {
        Self { store }
    }

    /// The store key one connection's database URL is sealed under.
    fn key_name(connection_key: Uuid) -> Result<SecretKeyName> {
        source_connection_database_url_key(connection_key)
    }

    /// The store row a stored connection points at.
    ///
    /// The row that carries it is the only representation of a connection's
    /// database URL, so a NULL reference means there is nothing to open and the
    /// connection is not configured.
    fn stored_key(connection: &StoredSourceConnection) -> Option<&str> {
        connection.database_url_secret_key.as_deref()
    }

    /// The database URL in effect: the sealed value the reference points at.
    ///
    /// Fails closed — a store row that exists and cannot be opened is an error,
    /// never a report that the connection has no credential.
    pub(super) async fn resolve(
        &self,
        connection: &StoredSourceConnection,
    ) -> Result<Option<String>> {
        match Self::stored_key(connection) {
            Some(secret_key) => {
                resolve_stored(
                    &self.store,
                    SecretPurpose::SourceConnectionDatabaseUrl,
                    secret_key,
                )
                .await
            }
            None => Ok(None),
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
                )
                .await
            }
            None => Ok(false),
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
/// reference has to resolve to a row that exists — and because a failed seal must
/// not leave a connection that points at a credential nobody stored.
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
        database_url_secret_key: secret_key.as_str().to_string(),
    })
    .await
}
