"""End-to-end sidecar protocol tests (no pipes; frames routed directly)."""

import asyncio
import json

import pytest

from kernel_sidecar.server import SidecarServer


class Captured:
    def __init__(self):
        self.frames = []

    def __call__(self, frame):
        self.frames.append(frame)

    def last(self, rid):
        for f in reversed(self.frames):
            if f.get("id") == rid:
                return f
        raise AssertionError(f"no frame for {rid}")


@pytest.fixture
async def server(tmp_path, monkeypatch):
    cap = Captured()
    monkeypatch.setattr("kernel_sidecar.server._send", cap)
    s = SidecarServer(state_root=str(tmp_path / "harness-root"))
    return s, cap


async def test_ping(server):
    s, cap = server
    await s.route_line(json.dumps({"id": "1", "method": "ping"}).encode())
    f = cap.last("1")
    assert f["ok"] and f["result"]["pong"] is True


async def test_unknown_method_errors_cleanly(server):
    s, cap = server
    await s.route_line(json.dumps({"id": "2", "method": "nope"}).encode())
    f = cap.last("2")
    assert not f["ok"] and "unknown method" in f["error"]


async def test_exec_persists_and_bridge_roundtrip(server):
    s, cap = server
    code1 = "marker = 'state-alive'\nprint('set')"
    await s.route_line(json.dumps(
        {"id": "e1", "method": "exec",
         "params": {"session_key": "s1", "code": code1}}).encode())
    assert cap.last("e1")["result"]["stdout"] == "set\n"

    # Bridged call: kernel awaits operant_tool → bridge frame out → reply in.
    code2 = (
        "r = await operant_tool('read_file', {'path': '/tmp/x'})\n"
        "print(marker)\n"
        "print(r.get('echo'))"
    )
    task = asyncio.ensure_future(s.route_line(json.dumps(
        {"id": "e2", "method": "exec",
         "params": {"session_key": "s1", "code": code2}}).encode()))
    # Yield until the bridge request appears on the wire.
    for _ in range(100):
        await asyncio.sleep(0)
        bridges = [f for f in cap.frames if "bridge_id" in f]
        if bridges:
            break
    assert bridges, "bridge frame never sent"
    bf = bridges[0]
    assert bf["method"] == "tool_call"
    assert bf["params"]["name"] == "read_file"
    await s.route_line(json.dumps(
        {"reply_for": bf["bridge_id"], "ok": True,
         "result": {"echo": "tool-result"}}).encode())
    await task
    res = cap.last("e2")["result"]
    assert res["ok"], res
    lines = res["stdout"].splitlines()
    assert lines[0] == "state-alive"      # persistence across cells
    assert lines[1] == "tool-result"      # bridged value returned into program


async def test_bridge_error_returns_as_value(server):
    s, cap = server
    task = asyncio.ensure_future(s.route_line(json.dumps(
        {"id": "e3", "method": "exec",
         "params": {"session_key": "s2",
                    "code": "r = await operant_tool('bash', {})\nprint('caught', '_bridge_error' in r)"}}).encode()))
    for _ in range(100):
        await asyncio.sleep(0)
        bridges = [f for f in cap.frames if "bridge_id" in f]
        if bridges:
            break
    await s.route_line(json.dumps(
        {"reply_for": bridges[0]["bridge_id"], "ok": False,
         "error": "permission denied"}).encode())
    await task
    res = cap.last("e3")["result"]
    assert res["ok"] and res["stdout"] == "caught True\n"


async def test_harness_crud_and_rollback(server):
    s, cap = server
    scope = "local"

    async def rpc(method, params, tag):
        await s.route_line(json.dumps(
            {"id": tag, "method": method, "params": {**params, "scope": scope}}).encode())
        return cap.last(tag)

    f = await rpc("harness_upsert",
                  {"kind": "prompt", "title": "Tone", "content": "Be terse."},
                  "h1")
    assert f["ok"], f
    entry_id = f["result"]["entry"]["id"]

    f = await rpc("harness_get", {"kind": "prompt", "id": entry_id}, "h2")
    assert f["result"]["entry"]["content"] == "Be terse."

    # Locked kinds: memory/skill rejected (operant owns those lanes).
    f = await rpc("harness_upsert", {"kind": "memory", "title": "x", "content": "y"}, "h3")
    assert not f["ok"] and "locked" in f["error"]

    # Apply a create edit all-or-nothing, then roll the whole thing back.
    f = await rpc("refine_apply",
                  {"edits": [{"action": "create", "kind": "subagent",
                              "title": "Reviewer role", "content": "Reviews diffs."}],
                   "trigger": "test", "evidence": "unit"},
                  "h4")
    assert f["ok"], f
    event_id = f["result"]["refinement_id"]

    f = await rpc("refine_history", {}, "h5")
    ids = [e["id"] for e in f["result"]["events"]]
    assert event_id in ids

    f = await rpc("refine_rollback", {"event_id": event_id}, "h6")
    assert f["ok"], f

    # After rollback the created subagent must be gone again:
    f = await rpc("harness_overview", {}, "h8")
    assert "Reviewer role" not in f["result"]["overview"]
