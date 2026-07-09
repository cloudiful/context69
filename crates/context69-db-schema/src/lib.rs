use anyhow::Result;
use sqlx::{PgPool, migrate::Migrator};

pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn relation_exists(pool: &PgPool, schema: &str, relation: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, Option<bool>>(
        r#"
            select exists (
                select 1
                from information_schema.tables
                where table_schema = $1 and table_name = $2
            )
            "#,
    )
    .bind(schema)
    .bind(relation)
    .fetch_one(pool)
    .await?
    .unwrap_or(false))
}

pub async fn assert_required_relations(
    pool: &PgPool,
    relations: &[(&str, &str)],
    owner: &str,
) -> Result<()> {
    for (schema, relation) in relations {
        if !relation_exists(pool, schema, relation).await? {
            anyhow::bail!(
                "database schema is not initialized for {}: missing relation {}.{}; run db_init first",
                owner,
                schema,
                relation
            );
        }
    }
    Ok(())
}
