-- One row per applied migration in the `public` ledger, read only.
--
-- The ledger table itself is a shared object, so ownership is decided from its
-- contents rather than its location: the runner compares each row's version and
-- checksum against the migrations this build actually ships. A row that matches
-- none of them is another application's, and the reset is refused rather than
-- guessed at.
--
-- Only reached when the preflight has already found a `public` ledger, so the
-- table is known to exist.
SELECT version  AS version,
       checksum AS checksum,
       success  AS success
  FROM public._sqlx_migrations
 ORDER BY version;
