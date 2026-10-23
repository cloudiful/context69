-- Extensions installed in the database, which every reset must preserve because
-- they live outside the owned schema.
SELECT count(*)::bigint AS "count!" FROM pg_extension
