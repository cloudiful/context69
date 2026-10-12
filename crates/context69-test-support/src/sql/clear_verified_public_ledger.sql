-- Remove the stale Context69 ledger rows after the owned schema was dropped and
-- recreated, so the migrator re-applies every migration into the fresh schema.
--
-- The ledger lives in `public`, which this runner does not own, so only the
-- exact (version, checksum) pairs this build ships are removed. A row belonging
-- to anything else stays untouched, and the preflight refuses the reset before
-- this runs when such a row exists. There is no whole-table delete: the ledger
-- table itself is preserved along with every other object in `public`.
DELETE FROM public._sqlx_migrations AS ledger
 WHERE EXISTS (
           SELECT 1
             FROM unnest($1::bigint[], $2::bytea[]) AS expected(version, checksum)
            WHERE expected.version = ledger.version
              AND expected.checksum = ledger.checksum
       );
