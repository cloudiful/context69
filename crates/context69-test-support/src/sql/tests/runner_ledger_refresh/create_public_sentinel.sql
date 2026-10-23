-- One relation in the ordinary schema that no reset may touch, so a case can
-- prove the reset stays inside its ownership.
CREATE TABLE IF NOT EXISTS public.context69_runner_ledger_refresh_sentinel (
    id integer PRIMARY KEY
)
