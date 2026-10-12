-- Relations in `public` whose names also exist inside the owned schema. With
-- both present, an unqualified name resolves through `search_path` and the
-- runner cannot prove which table a reset would affect.
SELECT c.relname
FROM pg_class c
JOIN pg_namespace pub ON pub.oid = c.relnamespace
WHERE pub.nspname = 'public'
  AND c.relkind IN ('r', 'v', 'm', 'S', 'f')
  AND EXISTS (
      SELECT 1
      FROM pg_class owned
      JOIN pg_namespace own_ns ON own_ns.oid = owned.relnamespace
      WHERE own_ns.nspname = $1
        AND owned.relname = c.relname
        AND owned.relkind = c.relkind
  )
ORDER BY c.relname;