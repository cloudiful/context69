use context69_test_support::config::resolve_default;
use context69_test_support::runner::preflight;
use sqlx::{Connection, postgres::PgConnection};

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("audit runtime");
    if runtime.block_on(inspect()).is_err() {
        eprintln!("read-only catalog audit failed; database diagnostic withheld");
        std::process::exit(1);
    }
}

async fn inspect() -> Result<(), Box<dyn std::error::Error>> {
    let target = resolve_default()?;
    let options = target.endpoint().expose().parse::<sqlx::postgres::PgConnectOptions>()?
        .application_name(preflight::RUNNER_APPLICATION_NAME)
        .options([("default_transaction_read_only", "on")]);
    let mut connection = PgConnection::connect_with(&options).await?;
    let (identity_matches, read_only): (bool, bool) = sqlx::query_as(
        "SELECT current_database() = $1, current_setting('default_transaction_read_only') = 'on'",
    )
    .bind(target.endpoint().database())
    .fetch_one(&mut connection)
    .await?;
    if !identity_matches || !read_only {
        return Err("read-only target validation failed".into());
    }
    println!("selected_database_matches=true default_transaction_read_only=true");
    let sql = include_str!("../../src/sql/foreign_schema_dependencies.sql");
    let refusals: Vec<(String,)> = sqlx::query_as(sql)
        .bind("context69")
        .fetch_all(&mut connection)
        .await?;
    println!("guard_refusal_count={}", refusals.len());
    for (namespace_or_category,) in refusals {
        println!("guard_dependency_namespace_or_category={namespace_or_category}");
    }
    let prefix = sql.split(", unclassified_owned AS (").next().ok_or("catalog prefix missing")?;
    let counts = format!(
        "{prefix} SELECT (SELECT count(*)::bigint FROM owned_toast_relations), \
         (SELECT count(*)::bigint FROM owned_toast_indexes), \
         (SELECT count(*)::bigint FROM allowed_toast_dependencies)"
    );
    let (relations, indexes, edges): (i64, i64, i64) =
        sqlx::query_as(sqlx::AssertSqlSafe(&*counts))
            .bind("context69")
            .fetch_one(&mut connection)
            .await?;
    println!("verified_toast_relations={relations} attached_indexes={indexes} allowed_dependency_edges={edges}");
    let diagnostic = format!(
        "{prefix} SELECT dc.relname::text, COALESCE(dn.nspname::text, '<unclassified>'), \
         COALESCE(dr.relkind::text, '-'), rc.relname::text, \
         COALESCE(rr.relkind::text, '-'), d.deptype::text, count(*)::bigint \
         FROM pg_catalog.pg_depend d \
         JOIN objects r ON r.classid = d.refclassid AND r.objid = d.refobjid \
         JOIN owned_schema own ON own.schema_oid = r.namespace_oid \
         LEFT JOIN objects dep ON dep.classid = d.classid AND dep.objid = d.objid \
         LEFT JOIN pg_catalog.pg_namespace dn ON dn.oid = dep.namespace_oid \
         JOIN pg_catalog.pg_class dc ON dc.oid = d.classid \
         JOIN pg_catalog.pg_class rc ON rc.oid = d.refclassid \
         LEFT JOIN pg_catalog.pg_class dr ON d.classid = 'pg_catalog.pg_class'::regclass AND dr.oid = d.objid \
         LEFT JOIN pg_catalog.pg_class rr ON d.refclassid = 'pg_catalog.pg_class'::regclass AND rr.oid = d.refobjid \
         WHERE dep.namespace_oid IS NULL OR dep.namespace_oid <> own.schema_oid \
         GROUP BY 1,2,3,4,5,6 ORDER BY 1,2,3,4,5,6"
    );
    let rows: Vec<(String, String, String, String, String, String, i64)> =
        sqlx::query_as(sqlx::AssertSqlSafe(&*diagnostic)).bind("context69").fetch_all(&mut connection).await?;
    for (class, namespace, kind, ref_class, ref_kind, dependency, count) in rows {
        println!("class={class} namespace={namespace} kind={kind} referenced_class={ref_class} referenced_kind={ref_kind} dependency={dependency} count={count}");
    }
    connection.close().await?;
    Ok(())
}
