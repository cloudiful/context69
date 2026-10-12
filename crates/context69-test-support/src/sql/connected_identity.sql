-- Read-only identity of the connected session. The runner compares this with
-- the selection so a URL that silently lands somewhere else is refused.
SELECT current_database(),
       current_setting('search_path'),
       current_schema()::text,
       current_schemas(false)::text[];