SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate AND datname <> $1 ORDER BY datname
