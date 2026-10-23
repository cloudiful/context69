-- One shared-ledger row this build does not ship. The fresh ledger guards must
-- refuse the next reset before it writes anything, and the row must survive the
-- refusal untouched.
INSERT INTO public._sqlx_migrations
    (version, description, installed_on, success, checksum, execution_time)
VALUES
    ($1, 'runner-ledger-refresh-foreign', now(), true, sha384('foreign'), 0)
