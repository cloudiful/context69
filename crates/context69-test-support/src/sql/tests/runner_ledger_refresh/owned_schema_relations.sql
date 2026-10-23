-- How many relations the owned schema holds, so a case can tell a migrated
-- schema from the empty one a skipped migrate leaves behind.
SELECT count(*)::bigint AS "count!"
  FROM pg_class class
  JOIN pg_namespace namespace ON namespace.oid = class.relnamespace
 WHERE namespace.nspname = 'context69'
   AND class.relkind IN ('r', 'p', 'v', 'm', 'S')
