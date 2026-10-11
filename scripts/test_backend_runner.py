from __future__ import annotations

import contextlib
import io
import json
import os
import signal
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from backend_test_db import DatabaseAdmin, DatabaseManager
from backend_test_process import (
    ProcessResult,
    RunInterrupted,
    discover_doctest_packages,
    discover_test_executables,
    run_logged,
    sanitize_output,
)

ROOT = Path(__file__).resolve().parents[1]
CACHE_TMP = Path.home() / ".cache/opencode/context69-52-postgres/backend-test-unit-tmp"


class FakeRunner:
    def __init__(self, *results: ProcessResult | BaseException) -> None:
        self.results = list(results)
        self.calls: list[tuple[list[str], dict[str, object]]] = []

    def __call__(self, command: list[str], **kwargs: object) -> ProcessResult:
        self.calls.append((list(map(str, command)), kwargs))
        result = self.results.pop(0)
        if isinstance(result, BaseException):
            raise result
        return result


def ok(code: int = 0) -> ProcessResult:
    return ProcessResult(code, "", "", 0.0)


class BackendTestRunnerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        CACHE_TMP.mkdir(parents=True, exist_ok=True)

    def admin(self) -> DatabaseAdmin:
        return DatabaseAdmin.parse("postgres://runner:secret@127.0.0.1:55499/postgres")

    def manager(self, runner: FakeRunner) -> DatabaseManager:
        return DatabaseManager(
            self.admin(), db_init=ROOT / "target/debug/db_init", runner=runner,
            cwd=ROOT, base_env={"SQLX_OFFLINE": "true"}, temp_dir=CACHE_TMP,
            log_dir=ROOT / "target/backend-test-results/unit-test",
        )

    def test_admin_url_requires_loopback_and_avoids_default_database_port(self) -> None:
        with self.assertRaises(ValueError):
            DatabaseAdmin.parse("postgres://runner@db.example/postgres")
        with self.assertRaises(ValueError):
            DatabaseAdmin.parse("postgres://runner@127.0.0.1/postgres")
        child_url = self.admin().url_for("context69_test_backend_abc_001")
        self.assertIn("runner:secret@127.0.0.1:55499/", child_url)

    def test_database_name_collision_is_never_owned_or_dropped(self) -> None:
        runner = FakeRunner(ok(1))
        manager = self.manager(runner)
        with self.assertRaises(RuntimeError):
            manager.create_and_initialize("context69_test_backend_collision_001", "case")
        self.assertEqual(manager.created_names, [])
        self.assertEqual(manager.cleanup(), [])
        self.assertEqual([call[0][0] for call in runner.calls], ["createdb"])

    def test_init_failure_tracks_and_drops_only_the_created_database(self) -> None:
        runner = FakeRunner(ok(), ok(1), ok())
        manager = self.manager(runner)
        name = "context69_test_backend_owned_001"
        with self.assertRaisesRegex(RuntimeError, "initialization failed"):
            manager.create_and_initialize(name, "case")
        self.assertEqual(manager.created_names, [name])
        self.assertEqual(manager.cleanup(), [])
        self.assertEqual(runner.calls[-1][0][0], "dropdb")
        self.assertEqual(runner.calls[-1][0][-1], name)

    def test_init_interruption_keeps_database_owned_for_cleanup(self) -> None:
        runner = FakeRunner(ok(), RunInterrupted("cancelled"), ok())
        manager = self.manager(runner)
        name = "context69_test_backend_interrupted_001"
        with self.assertRaises(RunInterrupted):
            manager.create_and_initialize(name, "case")
        self.assertEqual(manager.created_names, [name])
        self.assertEqual(manager.cleanup(), [])
        self.assertEqual(runner.calls[-1][0][0], "dropdb")
        self.assertEqual(runner.calls[-1][0][-1], name)

    def test_drop_failure_is_reported_and_the_owned_name_is_retained(self) -> None:
        runner = FakeRunner(ok(), ok(), ok(1))
        manager = self.manager(runner)
        name = "context69_test_backend_owned_002"
        manager.create_and_initialize(name, "case")
        self.assertIn(name, manager.created_names)
        self.assertTrue(manager.cleanup())
        self.assertEqual(manager.created_names, [name])

    def test_artifact_discovery_is_workspace_scoped_complete_and_deduplicated(self) -> None:
        with tempfile.TemporaryDirectory(dir=CACHE_TMP) as directory:
            executable = Path(directory) / "test-bin"
            executable.write_text("binary", encoding="utf-8")
            package_id = "path+file:///repo#demo@0.1.0"
            package = {
                "id": package_id,
                "name": "demo",
                "targets": [{"name": "demo", "kind": ["lib"], "test": True, "doctest": True}],
            }
            artifact = {
                "reason": "compiler-artifact", "package_id": package_id,
                "target": {"name": "demo", "kind": ["lib"], "test": True},
                "profile": {"test": True}, "executable": str(executable),
            }
            output = "\n".join(json.dumps(artifact) for _ in range(2))
            discovered = discover_test_executables(output, [package])
            self.assertEqual(len(discovered), 1)
            self.assertEqual(discovered[0].executable, executable.resolve())
            self.assertEqual(discover_doctest_packages([package]), ["demo"])

    def test_log_redaction_and_timeout_cleanup(self) -> None:
        self.assertNotIn("secret", sanitize_output("postgres://runner:secret@localhost/db"))
        with tempfile.TemporaryDirectory(dir=CACHE_TMP) as directory:
            log = Path(directory) / "process.log"
            result = run_logged(
                [sys.executable, "-c", "import time; time.sleep(30)"],
                cwd=ROOT, env=os.environ.copy(), timeout_seconds=0.05, log_path=log,
            )
            self.assertTrue(result.timed_out)
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(log.is_file())

    def test_successful_parent_cannot_leave_a_devnull_descendant_alive(self) -> None:
        group_id: int | None = None
        child_pid: int | None = None
        with tempfile.TemporaryDirectory(dir=CACHE_TMP) as directory:
            log = Path(directory) / "process.log"
            source = (
                "import os,subprocess,sys; print(os.getpgrp(),flush=True); "
                "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'], "
                "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); "
                "print(child.pid,flush=True)"
            )
            try:
                result = run_logged(
                    [sys.executable, "-c", source], cwd=ROOT, env=os.environ.copy(),
                    timeout_seconds=5, log_path=log,
                )
                output = result.stdout.split()
                if output:
                    group_id = int(output[0])
                if len(output) > 1:
                    child_pid = int(output[1])
                self.assertEqual(result.returncode, 0)
                self.assertIsNotNone(child_pid)
                deadline = time.monotonic() + 2
                while process_state(child_pid) not in {None, "Z", "X"} and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertIn(process_state(child_pid), {None, "Z", "X"})
            finally:
                if group_id is not None:
                    try:
                        os.killpg(group_id, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_target_setup_interruption_stops_admission_and_cleans_owned_databases(self) -> None:
        import test_backend as runner_module

        with tempfile.TemporaryDirectory(dir=CACHE_TMP) as directory:
            root = Path(directory)
            target_dir = root / "target"
            (target_dir / "debug").mkdir(parents=True)
            (target_dir / "debug/db_init").write_text("test initializer", encoding="utf-8")
            package_id = "path+file:///fake#runner-fixture@0.1.0"
            names = ["first_target", "second_target"]
            targets = [{"name": name, "kind": ["test"], "test": True} for name in names]
            package = {"id": package_id, "name": "runner-fixture", "targets": targets}
            artifacts = []
            for name in names:
                executable = target_dir / name
                executable.write_text("test executable", encoding="utf-8")
                artifacts.append(json.dumps({
                    "reason": "compiler-artifact", "package_id": package_id,
                    "target": {"name": name, "kind": ["test"], "test": True},
                    "profile": {"test": True}, "executable": str(executable),
                }))
            metadata = {
                "workspace_members": [package_id], "packages": [package],
                "target_directory": str(target_dir),
            }

            class InterruptingManager:
                def __init__(self) -> None:
                    self.db_init = target_dir / "debug/db_init"
                    self.created_names: list[str] = []
                    self.setup_calls = 0
                    self.cleanup_names: list[str] = []

                def create_and_initialize(self, name: str, _label: str) -> str:
                    self.setup_calls += 1
                    self.created_names.append(name)
                    raise RunInterrupted("received SIGTERM")

                def cleanup(self) -> list[str]:
                    self.cleanup_names = list(self.created_names)
                    self.created_names.clear()
                    return []

            manager = InterruptingManager()

            def fake_run(command: list[str], **_kwargs: object) -> ProcessResult:
                args = list(map(str, command))
                if args[:2] == ["cargo", "metadata"]:
                    return ProcessResult(0, json.dumps(metadata), "", 0.0)
                if args[:2] == ["cargo", "test"]:
                    return ProcessResult(0, "\n".join(artifacts), "", 0.0)
                if args[:2] == ["cargo", "build"]:
                    return ProcessResult(0, "", "", 0.0)
                raise AssertionError(f"unexpected command after interruption: {args}")

            output = io.StringIO()
            with patch.object(runner_module, "ROOT", root), \
                 patch.object(runner_module, "TEMP_ROOT", root / "runner-temp"), \
                 patch.object(runner_module, "DatabaseManager", return_value=manager), \
                 patch.object(runner_module, "run_logged", side_effect=fake_run), \
                 patch.object(runner_module, "install_interrupt_handlers", return_value=None), \
                 contextlib.redirect_stdout(output):
                exit_code = runner_module.main(["--database-url", "postgres://runner@127.0.0.1:55499/postgres"])

            summaries = list((root / "target/backend-test-results").glob("*/summary.json"))
            self.assertEqual(exit_code, 130)
            self.assertEqual(manager.setup_calls, 1)
            self.assertEqual(len(manager.cleanup_names), 1)
            self.assertEqual(json.loads(summaries[0].read_text())["status"], "interrupted")
            self.assertIn("interrupted", output.getvalue())


def process_state(pid: int) -> str | None:
    try:
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    return stat.split(")", 1)[1].split()[0]


if __name__ == "__main__":
    unittest.main()
