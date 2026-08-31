"""NDJSON JSON-RPC stdio server for the kernel-sidecar.

Framing contract with the Rust supervisor (operant-core/src/tools/pk/sidecar.rs):

  Rust → sidecar : one JSON object per line {"id","method","params"}
  sidecar → Rust : {"id","ok":true,"result"} | {"id","ok":false,"error"}
  sidecar → Rust : bridged tool call {"bridge_id","method":"tool_call",
                                     "params":{"session_key","name","args"}}
  Rust → sidecar : its reply {"reply_for","ok","result"|"error"}

User code output never touches this channel: cells run under
redirect_stdout/stderr into StringIO (see kernel.SessionKernel), and protocol
frames are written to the ORIGINAL stdout captured at boot.

Single asyncio loop, no locks: non-awaiting cells are atomic; awaiting cells
interleave across DIFFERENT session keys but are serialized within a key by a
per-key lock (preserves upstream RLock ordering semantics).
"""

from __future__ import annotations

import asyncio
import json
import sys
from typing import Any

from .harness import HarnessError, HarnessService
from .kernel import KernelRegistry, make_operant_tool
from . import vendor

_PROTO_OUT = sys.__stdout__  # captured BEFORE any redirect can occur

MAX_LINE_BYTES = 8 * 1024 * 1024


def _send(frame: dict[str, Any]) -> None:
    line = json.dumps(frame, ensure_ascii=False, default=str)
    if len(line.encode("utf-8")) > MAX_LINE_BYTES:
        frame = {"id": frame.get("id"), "ok": False,
                 "error": f"response exceeded {MAX_LINE_BYTES} bytes"}
        line = json.dumps(frame, ensure_ascii=False)
    _PROTO_OUT.write(line + "\n")
    _PROTO_OUT.flush()


class SidecarServer:
    def __init__(self, state_root: str | None = None) -> None:
        self.kernels = KernelRegistry()
        self.harness = HarnessService(
            state_root=__import__("pathlib").Path(state_root) if state_root else None
        )
        self._key_locks: dict[str, asyncio.Lock] = {}
        self._bridge_futures: dict[str, asyncio.Future] = {}
        self._bridge_counter = 0
        self._stopping = False

    # -- kernel plumbing ----------------------------------------------------
    def _lock_for(self, key: str) -> asyncio.Lock:
        lock = self._key_locks.get(key)
        if lock is None:
            lock = asyncio.Lock()
            self._key_locks[key] = lock
        return lock

    def _kernel_for(self, key: str):
        k = self.kernels.get(key)
        if not hasattr(k, "_kernel_operant_tool"):
            shim = make_operant_tool(k.session_key, self._bridged_call)
            k.install("operant_tool", shim)
            k._kernel_operant_tool = shim  # type: ignore[attr-defined]
        return k

    async def _bridged_call(self, session_key: str, name: str, args: dict) -> dict:
        """One bridged tool call to the Rust host (Phase 2.5)."""
        self._bridge_counter += 1
        bridge_id = f"b{self._bridge_counter}"
        fut: asyncio.Future = asyncio.get_running_loop().create_future()
        self._bridge_futures[bridge_id] = fut
        try:
            _send({"bridge_id": bridge_id, "method": "tool_call",
                   "params": {"session_key": session_key, "name": name, "args": args}})
            return await fut
        finally:
            self._bridge_futures.pop(bridge_id, None)

    # -- dispatch -----------------------------------------------------------
    async def handle(self, req: dict[str, Any]) -> None:
        rid = req.get("id")
        method = req.get("method")
        params = req.get("params") or {}
        try:
            result = await self._dispatch(method, params)
            _send({"id": rid, "ok": True, "result": result})
        except (HarnessError, FileNotFoundError) as exc:
            _send({"id": rid, "ok": False, "error": str(exc)})
        except Exception as exc:  # noqa: BLE03 - report, keep serving
            _send({"id": rid, "ok": False,
                   "error": f"{type(exc).__name__}: {exc}"})

    async def _dispatch(self, method: Any, p: dict[str, Any]) -> Any:
        if method == "ping":
            return {"pong": True, "prime_upstream_rev": self.harness.upstream_rev(),
                    "has_runtime": self.harness.available()}
        if method == "exec":
            key = str(p.get("session_key") or "default")
            code = p.get("code")
            if not isinstance(code, str) or not code.strip():
                raise ValueError("'code' must be a non-empty string")
            timeout = p.get("cell_timeout_secs")
            async with self._lock_for(key):
                kernel = self._kernel_for(str(key))
                return await kernel.execute(code, float(timeout) if timeout else None)
        if method == "reset":
            key = str(p.get("session_key") or "default")
            self.kernels.get(key).reset()
            return {"reset": key}
        if method in ("harness_list", "harness_get", "harness_upsert", "harness_delete",
                      "harness_overview", "refine_record", "refine_apply",
                      "refine_rollback", "refine_history"):
            return await asyncio.to_thread(self._harness_sync, method, p)
        if method == "skill_bind":
            # Plan 016, phase 6b: bind SKILL.toml [reference] into a session's globals.
            key = str(p.get("session_key") or "default")
            import_path = str(p.get("import") or "")
            callable = str(p.get("callable") or "")
            if not import_path or not callable:
                raise ValueError("skill_bind requires 'import' and 'callable'")
            async with self._lock_for(key):
                kernel = self._kernel_for(key)
                return kernel.bind_skill(import_path, callable)
        if method == "shutdown":
            self._stopping = True
            return {"bye": True}
        raise ValueError(f"unknown method {method!r}")

    # Harness store IO is blocking (json file CRUD) — off the event loop.
    def _harness_sync(self, method: str, p: dict[str, Any]) -> Any:
        scope = str(p.get("scope") or "local")
        sk_raw = p.get("session_key")
        sk = str(sk_raw) if sk_raw else None
        if method == "harness_list":
            kind = p.get("kind")
            out: dict[str, Any] = {}
            for k in (("prompt", "subagent") if kind in (None, "") else (kind,)):
                overview = self.harness.overview(scope=scope, session_key=sk)
                out[k] = {"overview_present": bool(overview.strip() and
                          overview.strip() != "(empty harness)")}
            out["overview"] = self.harness.overview(scope=scope, session_key=sk)
            return out
        if method == "harness_get":
            entry = self.harness.get(str(p["kind"]), str(p["id"]),
                                     scope=scope, session_key=sk)
            if entry is None:
                raise HarnessError(f"no {p['kind']} entry {p['id']}")
            return {"entry": entry}
        if method == "harness_upsert":
            return {"entry": self.harness.upsert(
                str(p["kind"]), str(p["title"]), str(p.get("content", "")),
                scope=scope, entry_id=p.get("id"),
                metadata=p.get("metadata"), session_key=sk)}
        if method == "harness_delete":
            return {"deleted": self.harness.delete(
                str(p["kind"]), str(p["id"]), scope=scope, session_key=sk)}
        if method == "harness_overview":
            return {"overview": self.harness.overview(scope=scope, session_key=sk)}
        if method == "refine_record":
            return self.harness.record_manual(
                str(p.get("evidence", "")), str(p.get("trigger") or "manual"),
                scope=scope, session_key=sk)
        if method == "refine_apply":
            edits = p.get("edits")
            if not isinstance(edits, list):
                raise HarnessError("refine_apply requires edits[]")
            return self.harness.apply_edits(
                edits, trigger=str(p.get("trigger") or "auto"),
                evidence=str(p.get("evidence", "")), scope=scope,
                session_key=sk)
        if method == "refine_rollback":
            return self.harness.rollback(str(p["event_id"]), scope=scope,
                                          session_key=sk)
        if method == "refine_history":
            return {"events": self.harness.history(int(p.get("limit") or 10),
                                                    scope=scope, session_key=sk)}
        raise AssertionError("unreachable")

    # -- framing ------------------------------------------------------------
    def _resolve_reply(self, frame: dict) -> bool:
        """If frame is a bridge reply, resolve its future; True when handled."""
        if "reply_for" not in frame:
            return False
        fut = self._bridge_futures.pop(str(frame["reply_for"]), None)
        if fut is not None and not fut.done():
            if frame.get("ok"):
                fut.set_result(frame.get("result") or {})
            else:
                fut.set_result({"_bridge_error": str(frame.get("error", "unknown"))})
        return True

    async def route_line(self, raw: bytes | None = None, *, frame: dict | None = None):
        """Route one inbound NDJSON frame (request OR bridge reply).

        Direct-caller semantics (tests): request frames are handled to
        COMPLETION before returning; bridge replies resolve inline.
        """
        if frame is None:
            try:
                frame = json.loads(raw.decode("utf-8"))  # type: ignore[union-attr]
            except (UnicodeDecodeError, json.JSONDecodeError, AttributeError):
                return
        if self._resolve_reply(frame):
            return
        if not isinstance(frame, dict) or "method" not in frame:
            return
        await self.handle(frame)

    async def serve(self) -> int:
        loop = asyncio.get_running_loop()
        reader: asyncio.StreamReader = asyncio.streams.StreamReader(limit=MAX_LINE_BYTES)
        protocol = asyncio.streams.StreamReaderProtocol(reader)
        await loop.connect_read_pipe(lambda: protocol, sys.stdin.buffer)
        while not self._stopping:
            line = await reader.readline()
            if not line:
                break  # EOF: supervisor died or closed — exit promptly
            try:
                frame = json.loads(line.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError):
                continue
            if self._resolve_reply(frame):
                continue  # inline: unblocks an awaiting cell immediately
            if not isinstance(frame, dict) or "method" not in frame:
                continue
            # Fire-and-forget: a cell awaiting a bridged call blocks only its
            # own handler task; the loop stays free to read reply lines.
            asyncio.create_task(self.handle(frame))
        return 0


def main() -> int:
    state_root = None
    argv = sys.argv[1:]
    if "--state-root" in argv:
        state_root = argv[argv.index("--state-root") + 1]
    server = SidecarServer(state_root=state_root)
    try:
        return asyncio.run(server.serve())
    except KeyboardInterrupt:
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
