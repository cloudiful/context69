use anyhow::Result;

use super::{Database, ProviderAccountRow, StoredProviderAccount};

impl Database {
    pub async fn list_provider_accounts(&self) -> Result<Vec<StoredProviderAccount>> {
        let rows = sqlx::query_file_as!(
            ProviderAccountRow,
            "src/sql/db/provider_accounts/list_provider_accounts.sql"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| StoredProviderAccount {
                account_key: row.account_key,
                provider_kind: row.provider_kind,
                display_name: row.display_name,
                base_url: row.base_url,
                api_key: row.api_key,
                disabled_at: row.disabled_at,
            })
            .collect())
    }

    pub async fn get_provider_account(
        &self,
        account_key: &str,
    ) -> Result<Option<StoredProviderAccount>> {
        let row = sqlx::query_file_as!(
            ProviderAccountRow,
            "src/sql/db/provider_accounts/get_provider_account.sql",
            account_key
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| StoredProviderAccount {
            account_key: row.account_key,
            provider_kind: row.provider_kind,
            display_name: row.display_name,
            base_url: row.base_url,
            api_key: row.api_key,
            disabled_at: row.disabled_at,
        }))
    }

    pub async fn save_provider_account(
        &self,
        account: &StoredProviderAccount,
    ) -> Result<StoredProviderAccount> {
        let row = sqlx::query_file_as!(
            ProviderAccountRow,
            "src/sql/db/provider_accounts/save_provider_account.sql",
            account.account_key,
            account.provider_kind,
            account.display_name,
            account.base_url,
            account.api_key,
            account.disabled_at
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(StoredProviderAccount {
            account_key: row.account_key,
            provider_kind: row.provider_kind,
            display_name: row.display_name,
            base_url: row.base_url,
            api_key: row.api_key,
            disabled_at: row.disabled_at,
        })
    }

    pub async fn delete_provider_account(&self, account_key: &str) -> Result<bool> {
        let result = sqlx::query_file!("src/sql/db/provider_accounts/delete_provider_account.sql", account_key)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
