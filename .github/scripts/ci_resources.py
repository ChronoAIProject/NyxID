#!/usr/bin/env python3
"""Run a CI command with optional resource samples, preserving failure/cancellation."""

import argparse
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import tempfile
import threading
import time


def read_optional(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None


def snapshot():
    memory = read_optional("/proc/meminfo") or ""
    values = {
        line.split(":", 1)[0]: line.split(":", 1)[1].strip()
        for line in memory.splitlines()
        if line.split(":", 1)[0]
        in {"MemTotal", "MemAvailable", "SwapTotal", "SwapFree"}
    }
    counters = {}
    for name in ("memory.current", "memory.peak", "memory.max", "memory.events",
                 "memory.swap.current", "memory.swap.max", "cpu.max",
                 "memory/memory.limit_in_bytes", "memory/memory.memsw.limit_in_bytes",
                 "memory/memory.max_usage_in_bytes", "memory/memory.failcnt"):
        value = read_optional(Path("/sys/fs/cgroup") / name)
        if value is not None:
            counters[name] = value
    processes = {}
    for entry in Path("/proc").glob("[0-9]*/comm"):
        name = read_optional(entry)
        if name in {"cargo", "rustc", "rust-lld", "ld", "ld.lld", "mongod"}:
            processes[name] = processes.get(name, 0) + 1
    result = {"memory": values, "cgroup_root": counters, "process_counts": processes}
    # CPU counters and pressure distinguish slow execution from memory exhaustion.
    # These are host observations, not per-command resource accounting.
    result["load_average"] = read_optional("/proc/loadavg")
    cpu = (read_optional("/proc/stat") or "").splitlines()
    result["cpu_times"] = cpu[0] if cpu and cpu[0].startswith("cpu ") else None
    result["pressure"] = {
        name: read_optional(Path("/proc/pressure") / name)
        for name in ("cpu", "io", "memory")
    }
    try:
        disk = os.statvfs(".")
        result["disk_available_bytes"] = disk.f_bavail * disk.f_frsize
    except OSError:
        pass
    return result


def command_output(command):
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=5)
        return result.stdout.strip() if result.returncode == 0 else "unavailable"
    except (OSError, subprocess.TimeoutExpired):
        return "unavailable"


def metadata():
    # Whitelist metadata: never print the environment or process command lines.
    names = ("ImageOS", "ImageVersion", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT",
             "CARGO_PROFILE_TEST_DEBUG", "CARGO_BUILD_JOBS", "RUST_TEST_THREADS",
             "CARGO_INCREMENTAL", "RUSTFLAGS", "CARGO_TARGET_DIR")
    return {
        "environment": {name: os.environ[name] for name in names if name in os.environ},
        "cpus": os.cpu_count(),
        "checkout_commit_and_tree": command_output(["git", "rev-parse", "HEAD", "HEAD^{tree}"]),
        "rust": command_output(["rustc", "-Vv"]),
        "coverage_tool": command_output(["cargo", "llvm-cov", "--version"]),
        "test_runner": command_output(["cargo", "nextest", "--version"]),
    }


def run(label, command, interval=30, cancellation_grace=10):
    started = time.monotonic()
    log = None
    try:
        directory = Path(os.environ.get("RUNNER_TEMP") or tempfile.gettempdir()) / "nyxid-ci-resources"
        directory.mkdir(parents=True, exist_ok=True)
        log = (directory / f"{label}.jsonl").open("a", encoding="utf-8")
    except OSError:
        print("ci-resources: artifact unavailable; continuing with live samples", flush=True)

    lock = threading.Lock()

    def emit(event, **data):
        nonlocal log
        line = json.dumps({"phase": label, "event": event,
                           "elapsed_seconds": round(time.monotonic() - started, 2), **data})
        with lock:
            try:
                print(f"[ci-resources] {line}", flush=True)
            except OSError:
                pass
            if log is not None:
                try:
                    log.write(line + "\n")
                    log.flush()
                except OSError:
                    try:
                        log.close()
                    except OSError:
                        pass
                    log = None

    def sample():
        try:
            emit("resources", **snapshot())
        except (OSError, ValueError):
            emit("resources_unavailable")

    stop = threading.Event()

    def monitor():
        while not stop.wait(interval):
            sample()

    child = None
    received_signal = None
    signal_errors = []

    def signal_child_group(signum):
        try:
            os.killpg(child.pid, signum)
        except ProcessLookupError:
            pass
        except OSError as error:
            # A reparented descendant can be outside our signal permissions.
            # Record this after wait; cancellation still returns a failure.
            signal_errors.append({"signal": signum, "errno": error.errno})

    def forward(signum, _frame):
        nonlocal received_signal
        received_signal = signum
        if child is not None:
            signal_child_group(signum)

    previous_handlers = {sig: signal.signal(sig, forward) for sig in (signal.SIGINT, signal.SIGTERM)}
    sampler = threading.Thread(target=monitor, daemon=True)
    try:
        emit("start", **metadata())
        sample()
        if received_signal is not None:
            return 128 + received_signal
        try:
            # A separate group lets cancellation reach Cargo's compiler/test children.
            child = subprocess.Popen(command, start_new_session=True)
        except FileNotFoundError:
            emit("exit", exit_code=127)
            return 127
        except OSError:
            emit("exit", exit_code=126)
            return 126
        if received_signal is not None:
            forward(received_signal, None)
        sampler.start()
        while True:
            try:
                code = child.wait(timeout=1)
                break
            except subprocess.TimeoutExpired:
                if received_signal is not None:
                    try:
                        code = child.wait(timeout=cancellation_grace)
                    except subprocess.TimeoutExpired:
                        signal_child_group(signal.SIGKILL)
                        try:
                            code = child.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            code = -signal.SIGKILL
                    break
        # Clean up descendants even if Cargo exits before its subprocesses do.
        if received_signal is not None or code < 0:
            signal_child_group(signal.SIGKILL)
            code = 128 + received_signal if received_signal is not None else 128 - code
        if signal_errors:
            emit("signal_delivery_errors", errors=signal_errors)
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        emit("exit", exit_code=code, child_user_seconds=usage.ru_utime,
             child_system_seconds=usage.ru_stime,
             child_max_rss_bytes=int(usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024)))
        return code
    finally:
        stop.set()
        if sampler.is_alive():
            sampler.join(timeout=2)
        sample()
        for sig, handler in previous_handlers.items():
            signal.signal(sig, handler)
        if log is not None:
            try:
                log.close()
            except OSError:
                pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("label")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not args.command or not args.label.replace("-", "").replace("_", "").isalnum():
        parser.error("use an alphanumeric phase label and a command")
    return run(args.label, args.command)


if __name__ == "__main__":
    sys.exit(main())
