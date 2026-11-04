-- One observation of everything the rollback-only upgrade test compares.
--
-- It is executed only while `context69.task_items.execution_failure_count`
-- exists: before the fixture transaction, after the immutable migration, and
-- after the rollback. `$1` is the fixture task id, `$2` the fixture owner id
-- (`0` for the pre-seed baseline) and `$3` the migration version under test.
--
-- The application ledger is read through the connection's admitted search path
-- with an unqualified table name, exactly as the application's own migrator
-- resolves it. The catalog count over `public`/`context69` lets the test refuse
-- an ambiguous ledger instead of guessing which one is the application's. The
-- library's separate ledger schema is deliberately not listed.
WITH ledger_tables AS (
    SELECT n.nspname AS schema_name
    FROM pg_class AS c
    JOIN pg_namespace AS n ON n.oid = c.relnamespace
    WHERE c.relname = '_sqlx_migrations'
      AND c.relkind = 'r'
      AND n.nspname IN ('public', 'context69')
)
SELECT
    EXISTS (SELECT 1 FROM context69.users AS u WHERE u.id = $2) AS "user_exists!",
    EXISTS (SELECT 1 FROM context69.tasks AS t WHERE t.id = $1) AS "task_exists!",
    (SELECT count(*)::bigint FROM ledger_tables) AS "application_ledger_count!",
    (
        SELECT COALESCE(jsonb_agg(schema_name ORDER BY schema_name), '[]'::jsonb)
        FROM ledger_tables
    ) AS "application_ledger_schemas!",
    (SELECT current_schema()) AS "ledger_schema",
    (SELECT m.version FROM _sqlx_migrations AS m WHERE m.version = $3) AS "ledger_version",
    (SELECT m.checksum FROM _sqlx_migrations AS m WHERE m.version = $3) AS "ledger_checksum",
    (SELECT m.success FROM _sqlx_migrations AS m WHERE m.version = $3) AS "ledger_success",
    (
        SELECT COALESCE(jsonb_agg(
            jsonb_build_object(
                'version', m.version,
                'description', m.description,
                'checksum', encode(m.checksum, 'hex'),
                'success', m.success,
                'installed_on', m.installed_on,
                'execution_time', m.execution_time
            )
            ORDER BY m.version
        ), '[]'::jsonb)
        FROM _sqlx_migrations AS m
    ) AS "ledger_rows!",
    (
        SELECT COALESCE(jsonb_agg(to_jsonb(item) ORDER BY item.ordinal), '[]'::jsonb)
        FROM context69.task_items AS item
        WHERE item.task_id = $1
    ) AS "fixture_items!",
    (
        SELECT COALESCE(jsonb_agg(to_jsonb(history_row) ORDER BY history_row.id), '[]'::jsonb)
        FROM context69.task_attempts AS history_row
        WHERE history_row.task_id = $1
    ) AS "fixture_history!",
    (
        SELECT md5(COALESCE(string_agg(to_jsonb(item)::text, '|' ORDER BY item.id), ''))
        FROM context69.task_items AS item
    ) AS "all_items_digest!",
    (
        SELECT count(*) > 0
        FROM pg_attribute AS a
        WHERE a.attrelid = 'context69.task_items'::regclass
          AND a.attname = 'execution_failure_count'
          AND a.attnum > 0
          AND NOT a.attisdropped
    ) AS "column_exists!",
    (
        SELECT a.attnotnull
        FROM pg_attribute AS a
        WHERE a.attrelid = 'context69.task_items'::regclass
          AND a.attname = 'execution_failure_count'
          AND a.attnum > 0
          AND NOT a.attisdropped
    ) AS "column_not_null",
    (
        SELECT format_type(a.atttypid, a.atttypmod)
        FROM pg_attribute AS a
        WHERE a.attrelid = 'context69.task_items'::regclass
          AND a.attname = 'execution_failure_count'
          AND a.attnum > 0
          AND NOT a.attisdropped
    ) AS "column_type",
    (
        SELECT pg_get_expr(d.adbin, d.adrelid)
        FROM pg_attrdef AS d
        JOIN pg_attribute AS a
          ON a.attrelid = d.adrelid
         AND a.attnum = d.adnum
        WHERE d.adrelid = 'context69.task_items'::regclass
          AND a.attname = 'execution_failure_count'
          AND NOT a.attisdropped
    ) AS "column_default",
    (
        SELECT pg_get_constraintdef(c.oid)
        FROM pg_constraint AS c
        WHERE c.conrelid = 'context69.task_items'::regclass
          AND c.contype = 'c'
          AND EXISTS (
              SELECT 1
              FROM pg_attribute AS a
              WHERE a.attrelid = c.conrelid
                AND a.attnum = ANY (c.conkey)
                AND a.attname = 'execution_failure_count'
                AND NOT a.attisdropped
          )
    ) AS "column_check",
    pg_get_functiondef('context69.notify_task_event()'::regprocedure) AS "notify_function!"
