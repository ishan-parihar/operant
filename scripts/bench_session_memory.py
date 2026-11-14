#!/usr/bin/env python3
"""Measure what a session costs: one server with N attached clients.

The audit that opened this campaign measured the OLD shape by hand: 13 full
`operant` processes, 792 MiB RSS, and 13 of 13 owning a separate AFT bridge —
about 61 MiB per interactive session. This script measures the NEW shape the
same way jcode's `bench_headless_memory.py` does: start ONE server, attach N
clients to it, and sum the RSS of the whole process tree.

    scripts/bench_session_memory.py --clients 8
    scripts/bench_session_memory.py --clients 8 --json

It never touches the operator's live gateway: the server runs under a
temporary OPERANT_HOME with its own daemon lock, and the socket is in that
temporary directory. Nothing is installed and nothing is restarted.

The number this prints is the server's tree divided by the clients attached to
it — the marginal cost of a session, which is the thing the 100-1000 target
lives on.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def _find_binary() -> Path:
    """Where the release binary is.

    A worktree has no `target/` of its own — the fleet builds into the main
    tree's shared cache — so the search covers both, and `$OPERANT_BIN`
    overrides it for a checkout laid out some other way.
    """
    override = os.environ.get("OPERANT_BIN")
    if override:
        return Path(override)
    candidates = [
        REPO / "target" / "release" / "operant",
        REPO.parent / "target" / "release" / "operant",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return candidates[0]


OPERANT = _find_binary()


def rss_tree(root_pid: int) -> tuple[int, int]:
    """The RSS and process count of a process and everything under it."""
    out = subprocess.run(
        ["ps", "-eo", "pid=,ppid=,rss="], capture_output=True, text=True, check=True
    ).stdout
    children: dict[int, list[int]] = {}
    rss: dict[int, int] = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 3:
            continue
        pid, ppid, kb = int(parts[0]), int(parts[1]), int(parts[2])
        children.setdefault(ppid, []).append(pid)
        rss[pid] = kb
    seen: set[int] = set()
    stack = [root_pid]
    total = 0
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        total += rss.get(pid, 0)
        stack.extend(children.get(pid, []))
    return total, len(seen)


def wait_for_socket(path: Path, timeout: float = 30.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            return True
        time.sleep(0.1)
    return False


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--clients", type=int, default=8, help="how many clients to attach")
    ap.add_argument("--json", action="store_true", help="print the result as JSON")
    args = ap.parse_args()

    if not OPERANT.exists():
        print(f"the release binary is not built: {OPERANT}", file=sys.stderr)
        print("build it with: scripts/build.sh --release", file=sys.stderr)
        return 2

    home = Path(tempfile.mkdtemp(prefix="operant-bench-"))
    socket = home / "session.sock"
    env = dict(os.environ, OPERANT_HOME=str(home), HERMES_HOME=str(home))
    procs: list[subprocess.Popen] = []
    clients: list[subprocess.Popen] = []

    try:
        # ONE server: the shape the campaign is moving to.
        server = subprocess.Popen(
            [str(OPERANT), "gateway", "run", "--socket", str(socket)],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        procs.append(server)
        if not wait_for_socket(socket):
            print("the server did not bind its socket", file=sys.stderr)
            return 1

        # N clients: each attaches and builds no runtime of its own.
        for _ in range(args.clients):
            clients.append(
                subprocess.Popen(
                    [str(OPERANT), "chat", "--attach", str(socket)],
                    env=env,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            )
        # Let the clients finish their hello and settle.
        time.sleep(3.0)

        alive = [c for c in clients if c.poll() is None]
        server_kb, server_procs = rss_tree(server.pid)
        client_kb = 0
        client_procs = 0
        for client in alive:
            kb, count = rss_tree(client.pid)
            client_kb += kb
            client_procs += count

        attached = len(alive)
        total_kb = server_kb + client_kb
        per_session_kb = (total_kb - server_kb) / attached if attached else 0

        result = {
            "clients_requested": args.clients,
            "clients_attached": attached,
            "server_rss_mib": round(server_kb / 1024, 1),
            "server_processes": server_procs,
            "clients_rss_mib": round(client_kb / 1024, 1),
            "clients_processes": client_procs,
            "total_rss_mib": round(total_kb / 1024, 1),
            "marginal_mib_per_client": round(per_session_kb / 1024, 1),
            "old_shape_mib_per_session": 61.0,
            "note": (
                "the old-shape number is the audit's measurement (13 full operant "
                "processes, 792 MiB RSS, 13 of 13 owning a separate AFT bridge); "
                "this run measures the new shape on this machine"
            ),
        }

        if args.json:
            print(json.dumps(result, indent=2))
        else:
            print(f"one server + {attached} attached client(s), measured on this machine\n")
            print(f"  server            {result['server_rss_mib']:>8} MiB  ({server_procs} process(es))")
            print(f"  attached clients  {result['clients_rss_mib']:>8} MiB  ({client_procs} process(es))")
            print(f"  total             {result['total_rss_mib']:>8} MiB")
            print(f"  marginal per client {result['marginal_mib_per_client']:>6} MiB")
            print(f"\n  the old shape: {result['old_shape_mib_per_session']} MiB per session")
            print(f"  {result['note']}")
        return 0
    finally:
        for proc in clients:
            if proc.poll() is None:
                proc.send_signal(signal.SIGTERM)
        for proc in procs:
            if proc.poll() is None:
                proc.send_signal(signal.SIGTERM)
        for proc in clients + procs:
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
        shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())