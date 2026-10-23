-- Every row of the shared ledger with its checksum as hex, so a case can compare
-- the applied set with the migrations this build ships.
SELECT version AS "version!",
       encode(checksum, 'hex') AS "checksum!",
       success AS "success!"
  FROM public._sqlx_migrations
 ORDER BY version
