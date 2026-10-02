//! Persistence for the reversible-secret backfill.
//!
//! The backfill reads legacy credentials through the projections its consumers
//! already use and needs six statements of its own: one read of the shared
//! `provider_key = 'llm'` row, four clears, and one narrow source-connection
//! reference update. Each is here, one method per statement, and each is
//! deliberately narrow — a clear nulls exactly one nullable column and nothing
//! else, and the reference update moves `database_url_secret_key` and
//! `updated_at` only.
//!
//! Nothing in this layer resolves a stored secret, so no value can reach a log
//! line or an error from here: a failure carries the statement's own reason,
//! and the methods return counts or booleans rather than rows.
//!
//! The legacy columns that are *not* cleared in this phase are deliberately
//! absent. A source connection's `database_url` is `NOT NULL` with a non-blank
//! check, and the runtime S3 `secret_key` belongs to an all-fields-required
//! settings projection, so both keep their legacy value until the
//! column-removal migration.

use anyhow::Result;

use super::Database;

/// One legacy credential column a backfill may null, named by its owning table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacySecretColumn {
    /// `context69.runtime_embedding_settings.api_key`.
    RuntimeEmbedding,
    /// `context69.search_settings.api_key`.
    Search,
    /// `context69.docling_settings.api_key`.
    Docling,
    /// `context69.translation_provider_settings.api_key`, `llm` row only.
    LlmProvider,
}

impl LegacySecretColumn {
    /// Every clearable legacy column, in a stable order.
    ///
    /// The catalogue of what a backfill may null. A column that is not here has
    /// a constraint or a projection that still depends on its value.
    pub const ALL: [Self; 4] = [
        Self::RuntimeEmbedding,
        Self::Search,
        Self::Docling,
        Self::LlmProvider,
    ];

    /// The column this variant names.
    ///
    /// Reported by a refusal or a count so an operator can see *where* a
    /// credential was or was not cleared. It is a column name, never a value.
    #[must_use]
    pub const fn column(self) -> &'static str {
        match self {
            Self::RuntimeEmbedding => "runtime_embedding_settings.api_key",
            Self::Search => "search_settings.api_key",
            Self::Docling => "docling_settings.api_key",
            Self::LlmProvider => "translation_provider_settings.api_key",
        }
    }

    /// Nulls this category's legacy column, reporting how many rows it touched.
    ///
    /// The caller decides the rest: this statement runs only after that
    /// category's sealed write and its round trip have committed, and it refuses
    /// nothing — a column that is already NULL matches no row, which is what
    /// makes a repeated run a no-op rather than an error.
    pub async fn clear(self, db: &Database) -> Result<u64> {
        let rows = match self {
            Self::RuntimeEmbedding => {
                sqlx::query_file!("src/sql/db/secret_backfill/clear_runtime_embedding_api_key.sql")
                    .execute(db.pool())
                    .await?
            }
            Self::Search => {
                sqlx::query_file!("src/sql/db/secret_backfill/clear_search_api_key.sql")
                    .execute(db.pool())
                    .await?
            }
            Self::Docling => {
                sqlx::query_file!("src/sql/db/secret_backfill/clear_docling_api_key.sql")
                    .execute(db.pool())
                    .await?
            }
            Self::LlmProvider => {
                sqlx::query_file!("src/sql/db/secret_backfill/clear_llm_provider_api_key.sql")
                    .execute(db.pool())
                    .await?
            }
        };
        Ok(rows.rows_affected())
    }
}

impl Database {
    /// The shared `provider_key = 'llm'` row's legacy API key.
    ///
    /// `Ok(None)` when that row does not exist and `Ok(Some(None))` when it
    /// exists without a key; either way there is nothing to migrate. The
    /// sibling `deepl` and `libretranslate` rows are never read, because no
    /// purpose owns their credentials.
    pub async fn llm_provider_api_key(&self) -> Result<Option<Option<String>>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/secret_backfill/get_llm_provider_api_key.sql")
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Points one source connection at the sealed database URL that owns its
    /// value, reporting whether exactly one row matched.
    ///
    /// `false` means the name matched no row, or matched more than one, so the
    /// caller treats the reference as not repointed. The statement leaves the
    /// legacy DSN and the connection's stable `connection_key` untouched.
    pub async fn set_source_connection_database_url_secret_key(
        &self,
        name: &str,
        secret_key: &str,
    ) -> Result<bool> {
        let rows = sqlx::query_file!(
            "src/sql/db/source_connections/set_source_connection_database_url_secret_key.sql",
            name,
            secret_key
        )
        .execute(&self.pool)
        .await?;
        Ok(rows.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::LegacySecretColumn;

    const LLM_READ: &str = include_str!("../sql/db/secret_backfill/get_llm_provider_api_key.sql");
    const CLEAR_EMBEDDING: &str =
        include_str!("../sql/db/secret_backfill/clear_runtime_embedding_api_key.sql");
    const CLEAR_SEARCH: &str = include_str!("../sql/db/secret_backfill/clear_search_api_key.sql");
    const CLEAR_DOCLING: &str = include_str!("../sql/db/secret_backfill/clear_docling_api_key.sql");
    const CLEAR_LLM: &str =
        include_str!("../sql/db/secret_backfill/clear_llm_provider_api_key.sql");
    const SET_REFERENCE: &str = include_str!(
        "../sql/db/source_connections/set_source_connection_database_url_secret_key.sql"
    );

    /// Every clear is one assignment to one nullable column, guarded so a
    /// repeated run is a no-op and nothing else in the row is rewritten.
    #[test]
    fn a_clear_nulls_one_nullable_column_and_nothing_else() {
        for (column, statement) in [
            (LegacySecretColumn::RuntimeEmbedding, CLEAR_EMBEDDING),
            (LegacySecretColumn::Search, CLEAR_SEARCH),
            (LegacySecretColumn::Docling, CLEAR_DOCLING),
            (LegacySecretColumn::LlmProvider, CLEAR_LLM),
        ] {
            let named = column.column();
            assert!(
                statement.contains("SET api_key = NULL") && statement.contains("IS NOT NULL"),
                "{named} must null only a column that still holds a key: {statement}"
            );
            let body = statement
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with("--"))
                .collect::<Vec<_>>()
                .join(" ");
            let (assignment, condition) = body
                .split_once(" WHERE ")
                .expect("a guarded single-row update");
            assert!(
                assignment
                    .to_ascii_lowercase()
                    .ends_with("set api_key = null"),
                "{named} must assign the credential column and nothing else: {body}"
            );
            assert_eq!(
                condition.to_ascii_lowercase(),
                format!(
                    "{} and api_key is not null",
                    if matches!(column, LegacySecretColumn::LlmProvider) {
                        "provider_key = 'llm'"
                    } else {
                        "singleton = true"
                    }
                ),
                "{named} must be guarded so a repeated run is a no-op: {body}"
            );
            assert!(
                named.ends_with("api_key"),
                "a clearable legacy column is an api_key column: {named}"
            );
        }
        // Only the shared row is cleared and only the shared row is read; the
        // sibling providers in the same column keep their credentials.
        assert!(
            CLEAR_LLM.contains("WHERE provider_key = 'llm'") && LLM_READ.contains("'llm'"),
            "the shared category is exactly the llm row"
        );
        for statement in [CLEAR_LLM, LLM_READ] {
            let lower = statement.to_ascii_lowercase();
            assert!(
                !lower.contains("'deepl'") && !lower.contains("'libretranslate'"),
                "a sibling provider's credential is not this category's: {statement}"
            );
        }
    }

    /// The source-connection reference update is the one statement that may
    /// touch a source connection row, and it moves the reference alone.
    #[test]
    fn the_source_reference_update_is_narrow_and_name_scoped() {
        assert!(
            SET_REFERENCE.contains("SET database_url_secret_key = $2")
                && SET_REFERENCE.contains("updated_at = now()"),
            "only the reference and its timestamp move: {SET_REFERENCE}"
        );
        let lower = SET_REFERENCE.to_ascii_lowercase();
        assert!(
            !lower.contains("database_url =") && !lower.contains("connection_key ="),
            "the legacy DSN and the stable identity a sealed value is keyed by must survive: \
             {SET_REFERENCE}"
        );
        assert!(
            lower.contains("update context69.runtime_source_connections")
                && lower.contains("where name = $1"),
            "the update is scoped to one named connection, not to every row: {SET_REFERENCE}"
        );
    }

    /// The four cleared columns are the complete set of clearable legacy
    /// credentials, and the two deferred columns are named by no statement in
    /// this module, so nothing here can reach them.
    #[test]
    fn the_deferred_legacy_columns_are_out_of_reach() {
        let statements = [
            CLEAR_EMBEDDING,
            CLEAR_SEARCH,
            CLEAR_DOCLING,
            CLEAR_LLM,
            SET_REFERENCE,
        ]
        .join("\n")
        .to_ascii_lowercase();
        for deferred in ["s3_secret_key", "s3_access_key"] {
            assert!(
                !statements.contains(deferred),
                "the runtime S3 settings stay intact until the column removal: {deferred}"
            );
        }
        // The S3 secret key is still sealed under its own singleton, so a
        // deployment that must read it keeps working while the column remains.
        assert!(
            statements.contains("database_url_secret_key")
                && !LegacySecretColumn::ALL
                    .iter()
                    .any(|column| column.column().contains("database_url")),
            "only the reference to a sealed DSN moves; the DSN column itself is never assigned"
        );
        assert!(
            statements.contains("where provider_key = 'llm'"),
            "the shared provider row is cleared through its own filter, not through the table"
        );
    }
}
