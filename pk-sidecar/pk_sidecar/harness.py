"""Continual-harness service over prime-agent's live vendored HarnessState.

Ported from hermes-prime-bridge ``bridge/harness.py`` with two plan-015
corrections:

* Kinds are locked to ``prompt`` and ``subagent`` — skills and memories stay
  owned by operant's curator/skills/MEMORY.md lanes (replace-never-duplicate).
* Refinement events carry a FULL before-snapshot in a sidecar-owned ledger so
  rollback is byte-exact (curator's tar.gz covers the skills dir only).
  All-or-nothing apply: any invalid edit aborts the whole pass on the snapshot.

Explicit state_dir everywhere — never ambient RLM_* env vars (host-path
aliasing immunity, upstream bridge lesson).
"""

from __future__ import annotations

import json
import time
import uuid
from pathlib import Path
from typing import Any

from . import vendor

HARNESS_KINDS = ("prompt", "subagent")
SCOPES = ("local", "global")

_LEDGER_FILE = "refinement-ledger.jsonl"


class HarnessError(RuntimeError):
    pass


def default_state_root(repo_data_home: Path | None = None) -> Path:
    base = repo_data_home or Path.home() / ".local" / "share" / "operant"
    return base / "pk" / "harness"


def _slugify(raw: str) -> str:
    normalized = "".join(ch.lower() if ch.isalnum() else "_" for ch in raw.strip())
    return "_".join(p for p in normalized.split("_") if p)[:80]


def _now_iso() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


class HarnessService:
    def __init__(self, state_root: Path | None = None) -> None:
        self._root = Path(state_root) if state_root else default_state_root()
        self._stores: dict[tuple[Path, bool], Any] = {}

    # -- store binding ----------------------------------------------------
    def _ensure(self, scope: str = "local", session_key: str | None = None):
        if scope not in SCOPES:
            raise HarnessError(f"scope must be one of {SCOPES}, got {scope!r}")
        if not vendor._HAS_PRIME_RUNTIME:
            raise HarnessError(vendor._SUBMODULE_MISSING_HINT)
        global_ = scope == "global"
        if global_:
            state_dir = self._root / "global"
        elif session_key:
            # prime-agent semantics: local scope is PER-SESSION so lessons
            # never leak across sessions; global stays shared.
            slug = _slugify(session_key) or "session"
            state_dir = self._root / "sessions" / slug / "local"
        else:
            state_dir = self._root / "local"
        key = (state_dir.resolve(), global_)
        store = self._stores.get(key)
        if store is None:
            rlm = vendor.require_rlm()
            state_dir.mkdir(parents=True, exist_ok=True)
            store = rlm.get_harness_state(state_dir=str(state_dir), global_=global_)
            self._stores[key] = store
        return store

    def available(self) -> bool:
        return vendor._HAS_PRIME_RUNTIME

    def upstream_rev(self) -> str | None:
        return vendor.prime_upstream_rev()

    # -- CRUD (kinds locked) ----------------------------------------------
    def _check_kind(self, kind: str) -> None:
        if kind not in HARNESS_KINDS:
            raise HarnessError(
                f"harness kinds are locked to {HARNESS_KINDS} "
                f"(skills/memories live in operant's own lanes); got {kind!r}"
            )

    def upsert(self, kind: str, title: str, content: str, *, scope: str = "local",
               entry_id: str | None = None, metadata: dict | None = None,
               session_key: str | None = None) -> dict:
        self._check_kind(kind)
        store = self._ensure(scope, session_key)
        kw: dict[str, Any] = {}
        if metadata is not None:
            kw["metadata"] = metadata
        entry = store.upsert(kind=kind, title=title, content=content,
                             id=entry_id, **kw) if entry_id else \
            store.upsert(kind=kind, title=title, content=content, **kw)
        store.save()
        return _entry_dict(entry)

    def get(self, kind: str, entry_id: str, *, scope: str = "local",
                session_key: str | None = None) -> dict | None:
        self._check_kind(kind)
        store = self._ensure(scope, session_key)
        entry = store.get(kind, entry_id)
        return _entry_dict(entry) if entry else None

    def delete(self, kind: str, entry_id: str, *, scope: str = "local",
                   session_key: str | None = None) -> bool:
        self._check_kind(kind)
        store = self._ensure(scope, session_key)
        existed = store.delete(kind=kind, id=entry_id)
        if existed:
            store.save()
        return bool(existed)

    def overview(self, *, scope: str = "local",
                  session_key: str | None = None) -> str:
        store = self._ensure(scope, session_key)
        try:
            ov = store.overview(max_entries_per_kind=20)
            return ov if isinstance(ov, str) else json.dumps(ov, ensure_ascii=False)
        except Exception:
            entries = store.list()
            return "\n".join(
                f"[{e.kind}:{e.id}] {e.title}" for e in (entries or []) if hasattr(e, "kind")
            ) or "(empty harness)"

    # -- refinement ledger --------------------------------------------------
    def _ledger_path(self, scope: str, session_key: str | None = None) -> Path:
        if scope == "global":
            return self._root / "global" / _LEDGER_FILE
        if session_key:
            return (self._root / "sessions" / (_slugify(session_key) or "session")
                    / "local" / _LEDGER_FILE)
        return self._root / "local" / _LEDGER_FILE

    def record_manual(self, evidence: str, trigger: str, *, scope: str = "local",
                      session_key: str | None = None) -> dict:
        """Record a manual refinement event with a before-snapshot (manual pk_refine)."""
        store = self._ensure(scope, session_key)
        before = store.snapshot()
        event_id = f"rf-{uuid.uuid4().hex[:12]}"
        self._append_ledger(scope, {
            "id": event_id, "ts": _now_iso(), "trigger": trigger or "manual",
            "evidence": evidence[:4000], "applied_edits": [],
            "before": before, "status": "recorded",
        }, session_key)
        return {"refinement_id": event_id, "scope": scope,
                "snapshot_entry_count": _snapshot_count(before)}

    def apply_edits(self, edits: list[dict], *, trigger: str, evidence: str,
                    scope: str = "local", session_key: str | None = None) -> dict:
        """All-or-nothing CRUD pass. Snapshot first; rollback to it on any failure."""
        if not edits:
            raise HarnessError("edits[] must be non-empty")
        if len(edits) > 12:
            raise HarnessError("max_edits_per_pass exceeded (12)")
        store = self._ensure(scope, session_key)
        before = store.snapshot()
        applied: list[dict] = []
        try:
            for edit in edits:
                action = edit.get("action")
                kind = edit.get("kind")
                self._check_kind(kind)
                eid = edit.get("id")
                if action == "create":
                    entry = store.create(kind=kind, title=edit["title"],
                                         content=edit["content"])
                    applied.append({"action": "create", "kind": kind,
                                    "id": getattr(entry, "id", None)})
                elif action == "update":
                    if not eid:
                        raise HarnessError("update requires id")
                    store.update(kind=kind, id=eid, title=edit.get("title"),
                                 content=edit.get("content"))
                    applied.append({"action": "update", "kind": kind, "id": eid})
                elif action == "delete":
                    if not eid:
                        raise HarnessError("delete requires id")
                    store.delete(kind=kind, id=eid)
                    applied.append({"action": "delete", "kind": kind, "id": eid})
                else:
                    raise HarnessError(f"invalid action {action!r}")
            store.save()
        except Exception:
            self._restore_snapshot(store, before)
            store.save()
            raise
        event_id = f"rf-{uuid.uuid4().hex[:12]}"
        self._append_ledger(scope, {
            "id": event_id, "ts": _now_iso(), "trigger": trigger or "auto",
            "evidence": evidence[:4000], "applied_edits": applied,
            "before": before, "status": "applied",
        }, session_key)
        return {"refinement_id": event_id, "scope": scope, "applied": applied,
                "entry_count": _snapshot_count(store.snapshot())}

    def rollback(self, event_id: str, *, scope: str = "local",
                     session_key: str | None = None) -> dict:
        ledger = self._read_ledger(scope, session_key)
        target = next((e for e in ledger if e.get("id") == event_id), None)
        if target is None:
            raise HarnessError(f"no refinement event {event_id} in scope {scope}")
        store = self._ensure(scope, session_key)
        current = store.snapshot()
        self._restore_snapshot(store, target["before"])
        store.save()
        self._append_ledger(scope, {
            "id": f"rb-{uuid.uuid4().hex[:12]}", "ts": _now_iso(),
            "trigger": "rollback", "evidence": f"rollback of {event_id}",
            "applied_edits": [], "before": current, "status": "rolled_back",
            "rolled_back_event": event_id,
        }, session_key)
        return {"rolled_back": event_id, "restored_entry_count":
                _snapshot_count(target["before"])}

    def history(self, limit: int = 10, *, scope: str = "local",
                    session_key: str | None = None) -> list[dict]:
        rows = self._read_ledger(scope, session_key)[-limit:]
        slim = [{k: v for k, v in row.items() if k != "before"} for row in rows]
        for row, full in zip(slim, rows[-limit:]):
            row["entry_count_before"] = _snapshot_count(full.get("before"))
        return slim

    # -- ledger IO ----------------------------------------------------------
    def _append_ledger(self, scope: str, row: dict, session_key: str | None = None) -> None:
        path = self._ledger_path(scope, session_key)
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(row, ensure_ascii=False, default=str) + "\n")

    def _read_ledger(self, scope: str, session_key: str | None = None) -> list[dict]:
        path = self._ledger_path(scope, session_key)
        if not path.exists():
            return []
        rows: list[dict] = []
        for line in path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                continue  # torn tail write; keep validated prefix
        return rows

    @staticmethod
    def _restore_snapshot(store: Any, snapshot: dict) -> None:
        """Rebuild store contents exactly from a snapshot() dict."""
        snap_entries = (snapshot or {}).get("entries", {})
        for kind in ("prompt", "memory", "skill", "subagent"):
            current = store.list(kind=kind) or []
            for entry in list(current):
                store.delete(kind=kind, id=getattr(entry, "id", None))
        for kind, entries in snap_entries.items():
            values = entries.values() if isinstance(entries, dict) else entries
            for data in values or []:
                if not isinstance(data, dict):
                    continue
                try:
                    store.create(
                        kind=kind,
                        title=data.get("title", ""),
                        content=data.get("content", ""),
                        id=data.get("id"),
                        metadata=data.get("metadata"),
                    )
                except TypeError:
                    store.create(kind=kind, title=data.get("title", ""),
                                 content=data.get("content", ""), id=data.get("id"))


def _entry_dict(entry: Any) -> dict:
    from dataclasses import asdict, is_dataclass

    if is_dataclass(entry):
        return asdict(entry)
    if isinstance(entry, dict):
        return entry
    return {"repr": repr(entry)}


def _snapshot_count(snapshot: dict | None) -> int:
    total = 0
    for entries in ((snapshot or {}).get("entries") or {}).values():
        total += len(entries or {})
    return total
