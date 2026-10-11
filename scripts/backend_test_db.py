from __future__ import annotations

import ipaddress
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Mapping, Sequence
from urllib.parse import parse_qsl, quote, unquote, urlsplit, urlunsplit

from backend_test_process import ProcessResult


_SAFE_DATABASE = re.compile(r"[A-Za-z_][A-Za-z0-9_-]{0,62}\Z")
CommandRunner = Callable[..., ProcessResult]


@dataclass(frozen=True)
class DatabaseAdmin:
    scheme: str
    host: str
    port: int
    username: str
    password: str | None
    database: str
    query: str

    @classmethod
    def parse(cls, value: str) -> DatabaseAdmin:
        try:
            parts = urlsplit(value)
            host = parts.hostname or ""
            port = parts.port or 5432
        except ValueError as error:
            raise ValueError("invalid PostgreSQL database URL") from error
        if parts.scheme not in {"postgres", "postgresql"} or parts.fragment:
            raise ValueError("database URL must be a PostgreSQL URL without a fragment")
        try:
            is_loopback = ipaddress.ip_address(host).is_loopback
        except ValueError:
            is_loopback = host.lower().rstrip(".") == "localhost"
        if not is_loopback:
            raise ValueError("database URL host must be loopback")
        if port == 5432:
            raise ValueError("port 5432 is not an authorized scratch endpoint")
        username = unquote(parts.username or "")
        database = unquote(parts.path.removeprefix("/"))
        if not username or not _SAFE_DATABASE.fullmatch(database):
            raise ValueError("database URL must name a user and one safe maintenance database")
        return cls(
            parts.scheme,
            host,
            port,
            username,
            unquote(parts.password) if parts.password is not None else None,
            database,
            parts.query,
        )

    def url_for(self, database: str) -> str:
        if not _SAFE_DATABASE.fullmatch(database):
            raise ValueError("unsafe database name")
        host = f"[{self.host}]" if ":" in self.host else self.host
        auth = quote(self.username, safe="")
        if self.password is not None:
            auth += ":" + quote(self.password, safe="")
        return urlunsplit(
            (self.scheme, f"{auth}@{host}:{self.port}", f"/{quote(database, safe='_-')}", self.query, "")
        )

    def command_env(self, base: Mapping[str, str]) -> dict[str, str]:
        env = dict(base)
        for key in tuple(env):
            if key.startswith("PG"):
                env.pop(key, None)
        env.update({"PGHOST": self.host, "PGPORT": str(self.port), "PGUSER": self.username})
        if self.password is not None:
            env["PGPASSWORD"] = self.password
        for key, value in parse_qsl(self.query):
            if key == "sslmode":
                env["PGSSLMODE"] = value
        return env

    def summary_endpoint(self) -> str:
        return f"{self.host}:{self.port}/{self.database}"


class DatabaseManager:
    def __init__(
        self,
        admin: DatabaseAdmin,
        *,
        db_init: Path,
        runner: CommandRunner,
        cwd: Path,
        base_env: Mapping[str, str],
        temp_dir: Path,
        log_dir: Path,
        redact: Sequence[str] = (),
    ) -> None:
        self.admin = admin
        self.db_init = db_init
        self.runner = runner
        self.cwd = cwd
        self.base_env = dict(base_env)
        self.temp_dir = temp_dir
        self.log_dir = log_dir
        self.redact = tuple(redact)
        self.created_names: list[str] = []

    def create_and_initialize(self, name: str, label: str) -> str:
        if not _SAFE_DATABASE.fullmatch(name) or name in self.created_names:
            raise ValueError("unsafe or duplicate runner database name")
        env = self.admin.command_env(self.base_env)
        createdb = self.runner(
            ["createdb", "--no-password", "--host", self.admin.host, "--port", str(self.admin.port),
             "--username", self.admin.username, "--maintenance-db", self.admin.database, name],
            cwd=self.cwd, env=env, timeout_seconds=120,
            log_path=self.log_dir / f"{label}-createdb.log", redact=self.redact,
        )
        if createdb.returncode != 0:
            raise RuntimeError(f"createdb failed for {name}; see {label}-createdb.log")
        self.created_names.append(name)

        database_url = self.admin.url_for(name)
        init_env = self.admin.command_env(self.base_env)
        init_env.pop("DATABASE_URL", None)
        init_env.pop("CONTEXT69_TEST_DATABASE_URL", None)
        init_env.update({"SQLX_OFFLINE": "true", "TMPDIR": str(self.temp_dir)})
        initialized = self.runner(
            [self.db_init, "--database-url", database_url],
            cwd=self.cwd, env=init_env, timeout_seconds=600,
            log_path=self.log_dir / f"{label}-db-init.log",
            redact=(*self.redact, database_url),
        )
        if initialized.returncode != 0:
            raise RuntimeError(f"database initialization failed for {name}; see {label}-db-init.log")
        return database_url

    def cleanup(self) -> list[str]:
        failures: list[str] = []
        remaining: list[str] = []
        env = self.admin.command_env(self.base_env)
        for index, name in enumerate(reversed(self.created_names), start=1):
            result = self.runner(
                ["dropdb", "--no-password", "--host", self.admin.host, "--port", str(self.admin.port),
                 "--username", self.admin.username, name],
                cwd=self.cwd, env=env, timeout_seconds=120,
                log_path=self.log_dir / f"cleanup-{index:03d}-{name}.log", redact=self.redact,
            )
            if result.returncode:
                remaining.append(name)
                failures.append(f"dropdb failed for owned database {name}; see cleanup log")
        self.created_names = list(reversed(remaining))
        return failures
