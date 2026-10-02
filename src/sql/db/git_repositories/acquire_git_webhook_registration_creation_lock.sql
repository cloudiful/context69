-- Serializes webhook registration creation for one (group_id, repository_key).
--
-- Transaction-scoped, so the lock is released by PostgreSQL when the creating
-- transaction ends whether it committed or rolled back, and keyed by the owning
-- group plus the repository the registration hangs from: two groups registering
-- hooks on their own repositories never queue behind each other, while two
-- concurrent creates for the same repository do. The caller builds the key.
SELECT pg_advisory_xact_lock(hashtextextended($1, 0))
