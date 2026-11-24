"""Measure the native sync loop against a private, already-synced library."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import statistics
import subprocess
import tempfile
import time
from contextlib import contextmanager


@contextmanager
def backend(driver, root):
    with subprocess.Popen([driver, root], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          text=True, encoding="utf-8") as child:
        def send(message):
            child.stdin.write(json.dumps(message) + "\n")
            child.stdin.flush()

        def call(command, **fields):
            send({"command": command, **fields})
            line = child.stdout.readline()
            if not line:
                raise RuntimeError(f"native fixture exited during {command} (code {child.poll()})")
            reply = json.loads(line)
            if not reply.get("ok"):
                raise RuntimeError(reply.get("error", "missing native reply"))
            return reply["value"]

        try:
            if json.loads(child.stdout.readline()) != {"ready": True}:
                raise RuntimeError("native fixture driver did not initialize")
            yield call
        finally:
            if child.poll() is None:
                send({"command": "shutdown"})
                try:
                    child.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    child.kill()
                    raise
        if child.returncode:
            raise RuntimeError(f"native fixture failed at shutdown (code {child.returncode})")


def seed(root, args):
    (root / ".native-e2e-fixture").write_text("telegram-drive-synthetic-e2e\n")
    with backend(args.driver, root) as call:
        call("seed_account", owner=101)
        call("sync_seed", owner="101", scanner=args.scanner)
    remote_path = root / "sync-fixture-remote.json"
    remote = json.loads(remote_path.read_text())
    # Replace the driver's small seed with exactly the requested cardinality.
    (root / "mapping-1" / "keep.txt").unlink()
    remote["1"] = {}
    payload = bytes(range(256)) * 256
    digest = hashlib.sha256(payload).hexdigest()
    mtime = int(time.time()) - 3600
    with sqlite3.connect(root / "shares.db") as db:
        db.execute("UPDATE sync_pairs SET is_active = 0 WHERE id <> 1")
        db.execute("DELETE FROM sync_state WHERE pair_id = 1")
        for i in range(args.files):
            name = f"fixture-{i:05}.bin"
            path = root / "mapping-1" / name
            path.write_bytes(payload)
            os.utime(path, times=(mtime, mtime))
            message = 1000 + i
            db.execute(
                "INSERT INTO sync_state(pair_id, relative_path, local_hash, remote_hash, "
                "file_size, local_mtime, remote_date, message_id, sync_status) "
                "VALUES(1, ?, ?, ?, ?, ?, ?, ?, 'synced')",
                (name, digest, digest, len(payload), mtime, mtime, message))
            remote["1"][name] = dict(relativePath=name, hash=digest, fileSize=len(payload),
                                     modifiedAt=mtime, messageId=message)
    remote_path.write_text(json.dumps(remote))
    return len(payload) * args.files


def ready(call, minimum_scans=0):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        value = call("sync_status", waitFor="ready")
        status = value["status"]
        if status["lastError"] or status["conflicts"] or status["pendingOps"]:
            raise RuntimeError("sample did not represent an idle library")
        # The native loop updates status just before it emits the final event.
        # Wait for its matching completed cycle too. This fixture has one active pair.
        if value["scans"] >= minimum_scans and len(durations(value)) == value["scans"]:
            return value
        time.sleep(0.001)
    raise RuntimeError("timed out waiting for the next completed scan")


def durations(snapshot):
    started = None
    elapsed = []
    for event in snapshot["history"]:
        if "fixtureElapsedUs" not in event:
            raise RuntimeError("rebuild the native fixture driver to enable monotonic scan timing")
        stamp = event["fixtureElapsedUs"]
        if event["running"] and started is None:
            started = stamp
        elif not event["running"] and started is not None:
            elapsed.append(round((stamp - started) / 1000, 2))
            started = None
    return elapsed


def measure(args):
    with tempfile.TemporaryDirectory(prefix="td-idle-sync-") as tmp:
        root = Path(tmp)
        size = seed(root, args)
        with backend(args.driver, root) as call:
            start = time.perf_counter()
            selected = call("sync_start", readyOwner=101)
            if args.scanner == "incremental" and selected != {"scanner": "incremental"}:
                raise RuntimeError("supplied driver does not support the incremental scanner")
            first = ready(call, 1)
            first_ms = (time.perf_counter() - start) * 1000

            # No writes, wakes, polling or account transitions in this window.
            time.sleep(args.seconds)
            idle = ready(call)
            passive_ms = durations(idle)[len(durations(first)):]
            previous = idle
            samples = []
            for _ in range(5):
                start = time.perf_counter()
                call("sync_wake")
                previous = ready(call, previous["scans"] + 1)
                samples.append(round((time.perf_counter() - start) * 1000, 2))
            call("sync_stop")
        with sqlite3.connect(root / "shares.db") as db:
            synced = db.execute(
                "SELECT COUNT(*) FROM sync_state WHERE pair_id = 1 AND sync_status = 'synced'"
            ).fetchone()[0]
        if synced != args.files:
            raise RuntimeError("library changed during idle sample")
        return dict(scanner=args.scanner, files=args.files, library_bytes=size,
                    first_ready_ms=round(first_ms, 2), passive_seconds=args.seconds,
                    passive_tree_requests=idle["scans"] - first["scans"],
                    passive_readiness_probes=idle["probes"] - first["probes"],
                    passive_native_cycle_ms=passive_ms,
                    additional_wake_ms=samples, median_wake_ms=statistics.median(samples),
                    final_synced_files=synced)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("driver", type=Path, help="built native-e2e-driver executable")
    parser.add_argument("--scanner", choices=["full", "incremental"], default="full")
    parser.add_argument("--files", type=int, default=256, choices=range(1, 1025),
                        metavar="1..1024")
    parser.add_argument("--seconds", type=int, default=65, choices=range(1, 901),
                        metavar="1..900")
    args = parser.parse_args()
    args.driver = args.driver.resolve(strict=True)
    print(json.dumps(measure(args), indent=2))
