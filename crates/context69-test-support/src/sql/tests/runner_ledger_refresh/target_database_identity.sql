-- The connected database and the owner comment the parent put on it, so a case
-- can prove it may recreate this fixture before anything is dropped.
SELECT current_database()::text AS "database!",
       coalesce((SELECT shobj_description(oid, 'pg_database')
                   FROM pg_database
                  WHERE datname = current_database()), '')::text AS "comment!"
