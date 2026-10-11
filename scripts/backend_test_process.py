from __future__ import annotations

import json
import os
import re
import shlex
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence


class RunInterrupted(Exception):
    pass


@dataclass(frozen=True)
class ProcessResult:
    returncode: int
    stdout: str
    stderr: str
    elapsed_seconds: float
    timed_out: bool = False


def install_interrupt_handlers() -> dict[int, object]:
    def interrupt(signum: int, _frame: object) -> None:
        raise RunInterrupted(f"received {signal.Signals(signum).name}")

    watched = (signal.SIGINT, signal.SIGTERM)
    previous = {signum: signal.getsignal(signum) for signum in watched}
    for signum in watched:
        signal.signal(signum, interrupt)
    return previous


def sanitize_output(text: str, secrets: Sequence[str] = ()) -> str:
    for secret in secrets:
        if secret:
            text = text.replace(secret, "<database-url-redacted>")
    return re.sub(
        r"(?i)(postgres(?:ql)?://[^:/@\s]+:)[^/@\s]+@",
        r"\1<redacted>@",
        text,
    )


def _signal_group(
    group_id: int | None,
    process: subprocess.Popen[str] | None,
    signum: int,
) -> None:
    try:
        if group_id is not None:
            os.killpg(group_id, signum)
        elif process is not None and process.poll() is None:
            process.send_signal(signum)
    except ProcessLookupError:
        pass


def _stop_process_group(process: subprocess.Popen[str], group_id: int | None) -> None:
    _signal_group(group_id, process, signal.SIGTERM)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        _signal_group(group_id, process, signal.SIGKILL)
        process.wait()
    _signal_group(group_id, process, signal.SIGKILL)


def _kill_orphaned_group(group_id: int | None) -> None:
    if group_id is not None:
        _signal_group(group_id, None, signal.SIGKILL)


def run_logged(
    command: Sequence[str | Path],
    *,
    cwd: Path,
    env: Mapping[str, str],
    timeout_seconds: float,
    log_path: Path,
    redact: Sequence[str] = (),
) -> ProcessResult:
    args = [str(part) for part in command]
    started = time.monotonic()
    timed_out = False
    stdout = ""
    stderr = ""
    try:
        with subprocess.Popen(
            args,
            cwd=cwd,
            env=dict(env),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            start_new_session=os.name == "posix",
        ) as process:
            # start_new_session makes the leader pid the group id.
            group_id = process.pid if os.name == "posix" else None
            try:
                stdout, stderr = process.communicate(timeout=timeout_seconds)
            except subprocess.TimeoutExpired:
                timed_out = True
                _stop_process_group(process, group_id)
                stdout, stderr = process.communicate()
            except BaseException:
                _stop_process_group(process, group_id)
                process.communicate()
                raise
            returncode = 124 if timed_out else int(process.returncode or 0)
            if not timed_out:
                _kill_orphaned_group(group_id)
    except OSError as error:
        returncode = 127
        stderr = str(error)

    elapsed = time.monotonic() - started
    safe_stdout = sanitize_output(stdout, redact)
    safe_stderr = sanitize_output(stderr, redact)
    safe_command = sanitize_output(shlex.join(args), redact)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_path.write_text(
        f"$ {safe_command}\nexit: {returncode}\nelapsed: {elapsed:.3f}s\n"
        f"--- stdout ---\n{safe_stdout}\n--- stderr ---\n{safe_stderr}",
        encoding="utf-8",
    )
    return ProcessResult(returncode, safe_stdout, safe_stderr, elapsed, timed_out)


@dataclass(frozen=True)
class TestExecutable:
    package_id: str
    package_name: str
    target_name: str
    kinds: tuple[str, ...]
    executable: Path

    @property
    def label(self) -> str:
        return f"{self.package_name}-{self.target_name}"


def discover_test_executables(output: str, packages: Sequence[dict[str, Any]]) -> list[TestExecutable]:
    workspace = {package["id"]: package for package in packages}
    expected = {
        (package["id"], target["name"])
        for package in packages
        for target in package.get("targets", [])
        if target.get("test")
    }
    found: dict[str, TestExecutable] = {}
    for line in output.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact":
            continue
        package_id = message.get("package_id")
        target = message.get("target", {})
        profile = message.get("profile", {})
        executable = message.get("executable")
        if package_id not in workspace or not target.get("test") or not profile.get("test"):
            continue
        if not executable:
            raise RuntimeError(f"test target has no executable: {target.get('name')}")
        path = Path(executable).resolve()
        if not path.is_file():
            raise RuntimeError(f"Cargo test executable is missing: {path}")
        package = workspace[package_id]
        found[str(path)] = TestExecutable(
            package_id, package["name"], target["name"], tuple(target.get("kind", [])), path
        )
    missing = expected - {(item.package_id, item.target_name) for item in found.values()}
    if missing:
        names = ", ".join(f"{workspace[package_id]['name']}:{name}" for package_id, name in sorted(missing))
        raise RuntimeError(f"Cargo test artifact discovery was incomplete: {names}")
    return sorted(found.values(), key=lambda item: (item.package_name, item.target_name, item.kinds))


def discover_doctest_packages(packages: Sequence[dict[str, Any]]) -> list[str]:
    return sorted(
        package["name"]
        for package in packages
        if any(
            target.get("doctest") and "lib" in target.get("kind", [])
            for target in package.get("targets", [])
        )
    )


def safe_slug(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "-", value).strip("-")[:64] or "target"
