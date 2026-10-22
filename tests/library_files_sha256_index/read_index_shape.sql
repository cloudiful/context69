-- The catalog shape of the applied library-file dedup lookup index.
--
-- Read-only. Every property the migration's compatibility guard decides on is
-- read from the catalog, so the applied index and the guard are judged by the
-- same facts.
SELECT
    c.relkind::TEXT AS relation_kind,
    am.amname::TEXT AS access_method,
    (SELECT n.nspname
       FROM pg_catalog.pg_namespace n
      WHERE n.oid = (SELECT owner.relnamespace
                       FROM pg_catalog.pg_class owner
                      WHERE owner.oid = i.indrelid)) AS indexed_schema,
    (SELECT owner.relname
       FROM pg_catalog.pg_class owner
      WHERE owner.oid = i.indrelid) AS indexed_table,
    i.indisunique AS is_unique,
    i.indisvalid AS is_valid,
    i.indisready AS is_ready,
    i.indislive AS is_live,
    i.indpred IS NOT NULL AS has_predicate,
    i.indexprs IS NOT NULL AS has_expressions,
    i.indnkeyatts::INT AS key_column_count,
    i.indnatts::INT AS total_column_count,
    (SELECT array_agg(a.attname::TEXT ORDER BY k.ord)
       FROM unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord)
       JOIN pg_catalog.pg_attribute a
         ON a.attrelid = i.indrelid AND a.attnum = k.attnum) AS key_columns
FROM pg_catalog.pg_index i
JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
JOIN pg_catalog.pg_am am ON am.oid = c.relam
WHERE n.nspname = 'context69'
  AND c.relname = 'idx_library_files_group_sha256'
