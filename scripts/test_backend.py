from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import tempfile
import uuid
from pathlib import Path
from typing import Any, Sequence

from backend_test_db import DatabaseAdmin, DatabaseManager
from backend_test_process import (
    RunInterrupted,
    TestExecutable,
    discover_doctest_packages,
    discover_test_executables,
    install_interrupt_handlers,
    run_logged,
    safe_slug,
    sanitize_output,
)

ROOT = Path(__file__).resolve().parents[1]
TEMP_ROOT = Path.home() / ".cache/opencode/context69-52-postgres/backend-test-tmp"
COMPILE_TIMEOUT = 2400
TARGET_TIMEOUT = 1800


def base_environment(temp_dir: Path) -> dict[str, str]:
    env = os.environ.copy()
    for key in tuple(env):
        if key in {"DATABASE_URL", "CONTEXT69_TEST_DATABASE_URL"} or key.startswith("PG"):
            env.pop(key, None)
    env.update({"SQLX_OFFLINE": "true", "TMPDIR": str(temp_dir), "TMP": str(temp_dir), "TEMP": str(temp_dir)})
    return env


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run each workspace backend test target with an isolated scratch database.")
    parser.add_argument("--database-url", required=True, help="explicit local loopback scratch PostgreSQL maintenance URL")
    args = parser.parse_args(argv)
    try:
        admin = DatabaseAdmin.parse(args.database_url)
    except ValueError as error:
        parser.error(str(error))

    run_id = uuid.uuid4().hex
    run_dir = ROOT / "target/backend-test-results" / run_id
    run_dir.mkdir(parents=True, exist_ok=False)
    temp_dir: Path | None = None
    manager: DatabaseManager | None = None
    failures: list[str] = []
    outcomes: list[dict[str, Any]] = []
    interrupted = False
    previous_handlers: dict[int, object] | None = None
    summary: dict[str, Any] = {
        "run_id": run_id,
        "database_endpoint": admin.summary_endpoint(),
        "status": "running",
        "test_executables": 0,
        "doctest_packages": 0,
        "outcomes": outcomes,
        "failures": failures,
    }
    previous_handlers = install_interrupt_handlers()
    try:
        TEMP_ROOT.mkdir(parents=True, exist_ok=True)
        temp_dir = Path(tempfile.mkdtemp(prefix=f"context69-{run_id}-", dir=TEMP_ROOT))
        env = base_environment(temp_dir)
        metadata_run = run_logged(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=ROOT, env=env, timeout_seconds=300, log_path=run_dir / "metadata.log",
        )
        if metadata_run.returncode:
            raise RuntimeError("cargo metadata failed; see metadata.log")
        metadata = json.loads(metadata_run.stdout)
        workspace = set(metadata["workspace_members"])
        packages = [package for package in metadata["packages"] if package["id"] in workspace]
        target_dir = Path(metadata["target_directory"])
        db_init = target_dir / "debug" / ("db_init.exe" if os.name == "nt" else "db_init")
        manager = DatabaseManager(
            admin, db_init=db_init, runner=run_logged, cwd=ROOT, base_env=env,
            temp_dir=temp_dir, log_dir=run_dir, redact=(args.database_url,),
        )

        build_run = run_logged(
            ["cargo", "test", "--workspace", "--all-targets", "--features", "integration-test-helpers",
             "--no-run", "--message-format=json"],
            cwd=ROOT, env=env, timeout_seconds=COMPILE_TIMEOUT,
            log_path=run_dir / "compile-tests.log",
        )
        if build_run.returncode:
            raise RuntimeError("workspace test compilation failed; see compile-tests.log")
        executables = discover_test_executables(build_run.stdout, packages)
        doctest_packages = discover_doctest_packages(packages)
        if not executables and not doctest_packages:
            raise RuntimeError("Cargo metadata and compiler artifacts discovered no test targets")
        summary["test_executables"] = len(executables)
        summary["doctest_packages"] = len(doctest_packages)

        db_init_build = run_logged(
            ["cargo", "build", "-p", "context69", "--bin", "db_init"],
            cwd=ROOT, env=env, timeout_seconds=COMPILE_TIMEOUT,
            log_path=run_dir / "compile-db-init.log",
        )
        if db_init_build.returncode or not manager.db_init.is_file():
            raise RuntimeError("could not build the database initializer; see compile-db-init.log")

        targets: list[tuple[str, TestExecutable | str]] = [
            (item.label, item) for item in executables
        ] + [(f"{name}-doctest", name) for name in doctest_packages]
        for index, (label, target) in enumerate(targets, start=1):
            slug = safe_slug(label)
            database_name = f"context69_test_backend_{run_id}_{index:03d}"
            try:
                database_url = manager.create_and_initialize(database_name, f"db-{index:03d}-{slug}")
            except RunInterrupted:
                raise
            except Exception as error:
                failures.append(f"{label}: database setup failed: {error}")
                outcomes.append({"target": label, "database": database_name, "status": "setup-failed"})
                continue
            target_env = dict(env)
            target_env["CONTEXT69_TEST_DATABASE_URL"] = database_url
            if isinstance(target, TestExecutable):
                command: list[str | Path] = [target.executable, "--test-threads=1"]
            else:
                command = ["cargo", "test", "-p", target, "--doc", "--", "--test-threads=1"]
            log_path = run_dir / f"test-{index:03d}-{slug}.log"
            result = run_logged(
                command, cwd=ROOT, env=target_env, timeout_seconds=TARGET_TIMEOUT,
                log_path=log_path, redact=(args.database_url, database_url),
            )
            outcomes.append({
                "target": label,
                "database": database_name,
                "exit_code": result.returncode,
                "timed_out": result.timed_out,
                "elapsed_seconds": round(result.elapsed_seconds, 3),
                "log": str(log_path.relative_to(ROOT)),
            })
            if result.returncode:
                failures.append(f"{label}: exited {result.returncode}; see {log_path.relative_to(ROOT)}")
    except (RunInterrupted, KeyboardInterrupt) as error:
        interrupted = True
        failures.append(str(error) or "interrupted")
    except Exception as error:
        failures.append(sanitize_output(f"runner failed: {type(error).__name__}: {error}", (args.database_url,)))
    finally:
        if previous_handlers is not None:
            signal.signal(signal.SIGINT, signal.SIG_IGN)
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
        try:
            if manager is not None:
                try:
                    failures.extend(manager.cleanup())
                except Exception as error:
                    failures.append(f"owned database cleanup raised {type(error).__name__}: {error}")
                summary["remaining_owned_databases"] = list(manager.created_names)
            if temp_dir is not None:
                try:
                    shutil.rmtree(temp_dir)
                except OSError as error:
                    failures.append(f"owned TMPDIR cleanup failed: {error}")
            summary["status"] = "interrupted" if interrupted else ("failed" if failures else "passed")
            summary["failures"] = failures
            (run_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        finally:
            if previous_handlers is not None:
                for signum, handler in previous_handlers.items():
                    signal.signal(signum, handler)

    print(f"Backend test run {run_id}: {summary['status']}")
    print(f"Targets: {len(outcomes)}; failures: {len(failures)}")
    print(f"Logs: {run_dir.relative_to(ROOT)}")
    return 130 if interrupted else (1 if failures else 0)


if __name__ == "__main__":
    raise SystemExit(main())
