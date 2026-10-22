-- Guarded lookup index for library file dedup (issue #1041 P1).
--
-- The dedup lookup reads `context69.library_files` by `(group_id, sha256)`
-- with a deterministic `ORDER BY created_at, id LIMIT 1`. No index covered that
-- pair, so every miss scanned the whole table. This migration adds one
-- non-unique btree index on `(group_id, sha256, created_at, id)`: the equality
-- pair leads the index and the trailing pair already carries the order the
-- lookup asks for, so one index serves both the filter and the sort. No
-- query, response, or uniqueness rule changes, and no data is touched.
--
-- Plain (transactional) CREATE INDEX on purpose: the sqlx migrator sends a
-- migration file as one simple-query message, so all statements of a file
-- share a single implicit transaction and CREATE INDEX CONCURRENTLY is
-- rejected there. To avoid the write lock on a large
-- `context69.library_files` table, the operator may pre-create the index
-- CONCURRENTLY out of band before applying this migration; the guard below
-- then accepts that equivalent index and the IF NOT EXISTS turns the
-- statement into a no-op.
--
-- The guard is fail-closed. `CREATE INDEX IF NOT EXISTS` only emits a notice
-- when a same-name relation exists, so it would silently skip a wrong or
-- invalid object and report a success that never created the index. The guard
-- therefore inspects the catalog and accepts a same-name object only when it
-- is a valid, ready, live, non-unique btree index over exactly
-- `(group_id, sha256, created_at, id)` in the default ascending order, with no
-- predicate, no expression columns, and no INCLUDE columns. Anything else
-- aborts the migration with actionable guidance. The migration never drops or
-- repairs a same-name object; only the operator may resolve one.
--
-- Rollback: DROP INDEX (CONCURRENTLY) context69.idx_library_files_group_sha256;
-- no data is touched.

DO $$
DECLARE
    target_name CONSTANT TEXT := 'idx_library_files_group_sha256';
    target_table CONSTANT regclass := 'context69.library_files'::regclass;
    expected_columns CONSTANT TEXT[] := ARRAY['group_id', 'sha256', 'created_at', 'id'];
    existing_oid OID;
    existing_kind "char";
    existing_kind_label TEXT;
    existing_method TEXT;
    existing_table regclass;
    existing_columns TEXT[];
    existing_default_order BOOLEAN;
    existing_key_count INT;
    existing_total_count INT;
    existing_unique BOOLEAN;
    existing_valid BOOLEAN;
    existing_ready BOOLEAN;
    existing_live BOOLEAN;
    existing_predicate BOOLEAN;
    existing_expressions BOOLEAN;
    problems TEXT[] := ARRAY[]::TEXT[];
    summary TEXT;
    inspection TEXT;
BEGIN
    SELECT c.oid, c.relkind
      INTO existing_oid, existing_kind
      FROM pg_catalog.pg_class c
      JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
     WHERE n.nspname = 'context69'
       AND c.relname = target_name;

    -- Nothing to reconcile: the index does not exist yet.
    IF existing_oid IS NULL THEN
        RETURN;
    END IF;

    IF existing_kind <> 'i' THEN
        existing_kind_label := CASE existing_kind
            WHEN 'r' THEN 'table'
            WHEN 'p' THEN 'partitioned table'
            WHEN 'v' THEN 'view'
            WHEN 'm' THEN 'materialized view'
            WHEN 'f' THEN 'foreign table'
            WHEN 'S' THEN 'sequence'
            ELSE existing_kind::TEXT
        END;
        problems := array_append(
            problems,
            format('context69.%s is a %s, not an index', target_name, existing_kind_label));
    ELSE
        SELECT am.amname,
               i.indrelid,
               i.indisunique,
               i.indisvalid,
               i.indisready,
               i.indislive,
               i.indpred IS NOT NULL,
               i.indexprs IS NOT NULL,
               i.indnkeyatts,
               i.indnatts,
               (SELECT coalesce(bool_and(sort_option = 0), false)
                  FROM unnest(i.indoption::int2[]) AS sort_option),
               (SELECT array_agg(a.attname ORDER BY k.ord)
                  FROM unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord)
                  JOIN pg_catalog.pg_attribute a
                    ON a.attrelid = i.indrelid AND a.attnum = k.attnum)
          INTO existing_method,
               existing_table,
               existing_unique,
               existing_valid,
               existing_ready,
               existing_live,
               existing_predicate,
               existing_expressions,
               existing_key_count,
               existing_total_count,
               existing_default_order,
               existing_columns
          FROM pg_catalog.pg_index i
          JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid
          JOIN pg_catalog.pg_am am ON am.oid = c.relam
         WHERE i.indexrelid = existing_oid;

        IF existing_table IS DISTINCT FROM target_table THEN
            problems := array_append(
                problems,
                format('it indexes %s, not context69.library_files', existing_table));
        END IF;
        IF existing_method IS DISTINCT FROM 'btree' THEN
            problems := array_append(
                problems,
                format('it uses the %s access method, not btree', existing_method));
        END IF;
        IF existing_unique THEN
            problems := array_append(problems, 'it is unique');
        END IF;
        IF existing_predicate THEN
            problems := array_append(problems, 'it has a partial predicate');
        END IF;
        IF existing_expressions THEN
            problems := array_append(problems, 'it has expression columns');
        END IF;
        IF existing_total_count IS DISTINCT FROM existing_key_count THEN
            problems := array_append(
                problems,
                format('it carries %s INCLUDE columns', existing_total_count - existing_key_count));
        END IF;
        IF existing_key_count IS DISTINCT FROM 4
           OR existing_columns IS DISTINCT FROM expected_columns THEN
            problems := array_append(
                problems,
                format('its key columns are (%s), not (%s)',
                       coalesce(array_to_string(existing_columns, ', '), 'none'),
                       array_to_string(expected_columns, ', ')));
        ELSIF NOT existing_default_order THEN
            problems := array_append(problems, 'its columns are not in the default ascending order');
        END IF;
        IF NOT existing_valid OR NOT existing_ready OR NOT existing_live THEN
            problems := array_append(problems, 'it is not a valid, ready, live index');
        END IF;
    END IF;

    IF cardinality(problems) > 0 THEN
        summary := array_to_string(problems, '; ');
        inspection := format(
            'Inspect it with SELECT pg_get_indexdef(%s::regclass) and the pg_index/pg_class catalogs.',
            format('''context69.%s''', target_name));
        RAISE EXCEPTION '%s',
            format(
                'context69 library file dedup index guard failed for context69.%s: %s. %s Keep an equivalent index, or correct a wrong or invalid one, as the operator. This migration never drops or repairs a same-name object.',
                target_name, summary, inspection);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_library_files_group_sha256
    ON context69.library_files (group_id, sha256, created_at, id);
