-- Object counts inside the owned schema. Reported as counts only; no
-- definition, row, or password is read.
SELECT
    (SELECT count(*)::bigint
       FROM pg_class c
       JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = $1 AND c.relkind = 'r')::bigint AS tables,
    (SELECT count(*)::bigint
       FROM pg_class c
       JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = $1 AND c.relkind = 'v')::bigint AS views,
    (SELECT count(*)::bigint
       FROM pg_class c
       JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = $1 AND c.relkind = 'S')::bigint AS sequences,
    (SELECT count(*)::bigint
       FROM pg_proc p
       JOIN pg_namespace n ON n.oid = p.pronamespace
      WHERE n.nspname = $1)::bigint AS functions,
    (SELECT count(*)::bigint
       FROM pg_index i
       JOIN pg_class c ON c.oid = i.indexrelid
       JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = $1)::bigint AS indexes;