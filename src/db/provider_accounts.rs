use anyhow::Result;

use super::{Database, ProviderAccountRow, StoredProviderAccount};

impl Database {
    pub async fn list_provider_accounts(&self) -> Result<Vec<StoredProviderAccount>> {
        let rows = sqlx::query_as::<_, ProviderAccountRow>(
            r#"
            SELECT account_key, provider_kind, display_name, base_url, api_key, disabled_at
            FROM context69.runtime_provider_accounts
            ORDER BY account_key
            "#,
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
        let row = sqlx::query_as::<_, ProviderAccountRow>(
            r#"
            SELECT account_key, provider_kind, display_name, base_url, api_key, disabled_at
            FROM context69.runtime_provider_accounts
            WHERE account_key = $1
            "#,
        )
        .bind(account_key)
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
        let row = sqlx::query_as::<_, ProviderAccountRow>(
            r#"
            INSERT INTO context69.runtime_provider_accounts (
                account_key,
                provider_kind,
                display_name,
                base_url,
                api_key,
                disabled_at,
                updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, now())
            ON CONFLICT (account_key) DO UPDATE
            SET provider_kind = EXCLUDED.provider_kind,
                display_name = EXCLUDED.display_name,
                base_url = EXCLUDED.base_url,
                api_key = EXCLUDED.api_key,
                disabled_at = EXCLUDED.disabled_at,
                updated_at = now()
            RETURNING account_key, provider_kind, display_name, base_url, api_key, disabled_at
            "#,
        )
        .bind(&account.account_key)
        .bind(&account.provider_kind)
        .bind(&account.display_name)
        .bind(&account.base_url)
        .bind(&account.api_key)
        .bind(account.disabled_at)
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
        let result = sqlx::query(
            r#"
            DELETE FROM context69.runtime_provider_accounts
            WHERE account_key = $1
            "#,
        )
        .bind(account_key)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
