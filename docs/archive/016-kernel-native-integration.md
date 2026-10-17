# 016 — Kernel Native Integration: make the persistent kernel a first-class operant subsystem

Stamped against `pk/kernel-harness` @ 43ce0962. Priority: P1 (follow-on to 015).

## Context

Plan 015 delivered the kernel as a **dark, gated add-on** under `[tools.kernel]` (default-off) with prime-branded internals. Commit 43ce0962 canonicalized naming (`pk_*` -> `kernel_*`, `prime_kernel` -> `kernel`, `pk-sidecar` -> `kernel-sidecar`) but left five gaps that prevent the kernel from feeling native:

1. **Config is still an add-on**: `[tools.kernel]` is correct structurally (all tools live under `[tools.*]`), but the feature reads as "extra" because it is default-off and undocumented in `operant.example.toml`.
2. **Vendor path leaks**: `vendor/prime-agent` is an implementation detail visible in `.gitmodules`, `kernel_sidecar/vendor.py`, and docs. Native code should not expose upstream branding.
3. **Session GC missing**: `<state_dir>/sessions/<id>/local` grows without bound; no sweep.
4. **Per-turn injection cost**: `injection_block()` does a sidecar roundtrip every turn when enabled; no cache within a turn, so concurrent tool calls could double-pay.
5. **Executable skills (phase 6) not native**: Plan 015 deferred `SKILL.toml [reference]`; without it, learned skills remain prose-only, not executable.

## Perfect native solution

**Architecture stays**: Rust-supervised Python sidecar over NDJSON, policy in host, state in sidecar. What changes is *integration surface*:

```
Before (015 dark):   [tools.kernel] enabled=false, toolset "kernel" hidden, docs as add-on
After  (016 native):  [tools.kernel] enabled=true by default once provisioned,
                      toolset "kernel" listed alongside code_execution in `operant tools list`,
                      `operant.example.toml` documents it, session GC and per-turn cache are built-in,
                      SKILL.toml [reference] is a native field (opt-in via allowlist)
```

### Decision log

| Decision | Choice | Rationale |
|---|---|---|
| Config location | Keep `[tools.kernel]` | All tools live under `[tools.*]`; moving to top-level `[kernel]` would be *less* native |
| Default enabled | **Keep default-off in code**, flip to true in `operant.example.toml` + docs | Code default-off is safe for users without `uv`/`python3.11` or submodule; example + docs signal "core" without breaking existing installs |
| Vendor path | Keep `vendor/prime-agent` on disk, hide in user docs; add `vendor/kernel-runtime` symlink or keep as-is with comment | Renaming submodule path requires `git submodule` rewrite and breaks `git submodule update` for existing clones; cost > benefit. Hide branding in docs, keep path. |
| Session GC | Add `KernelState::gc_sessions(ttl_hours)` called at startup and on idle reaper | Cheap, native, no new config |
| Injection cache | `OnceCell<String>` per turn, invalidated on any `kernel_state`/`kernel_refine` write | Eliminates double roundtrip within one turn |
| Executable skills | Add `SkillManifest [reference]` as optional top-level table (mirrors `[forge]` precedent), kernel loads via allowlist | Smallest schema change; respects `deny_unknown_fields` via `Option` |

## Implementation steps (016)

### Phase 6a — Native surface polish (no new behavior, just integration)

1. **operant.example.toml**: add `[tools.kernel]` section with native defaults and comments (enabled false with note "set true after `git submodule update --init --recursive`").
2. **Session GC**: add `KernelRuntime::gc_sessions()` that scans `<state_dir>/sessions/*` and prunes dirs older than `session_gc_ttl_hours` (default 168h). Call at `KernelRuntime::new()` and in idle reaper.
3. **Per-turn cache**: add `Arc<Mutex<Option<(String, Instant)>>>` cache in `KernelRuntime`; `injection_block()` checks cache (TTL 5s), `kernel_state`/`kernel_refine` writes invalidate it.
4. **Docs**: update `docs/kernel.md` to remove "opt-in" framing, add "Native vs add-on" section, update README bullet from "opt-in" to "native (provision once)".
5. **Vendor hiding**: update `docs/kernel.md` and `kernel_sidecar/vendor.py` docstring to say "vendored rlm runtime" without prime branding; keep path.

### Phase 6b — Executable skills (SKILL.toml [reference])

6. **Schema**: add `#[derive(Deserialize)] struct SkillReference { type, import, callable, call_pattern }` as `Option<SkillReference>` on `SkillManifest` top-level (not on `SkillMeta`), with `#[serde(default)]`. Mirrors `[forge]` precedent (FND-001 4.2).
7. **Kernel load**: at session init, if `[tools.kernel.pyskill.enabled]` and import allowlisted, bind `callable` into globals. Handle sync vs async, capture ImportError as `_import_error` binding.
8. **Injection**: extend `injection_block()` third section "executable skills:" when references exist.
9. **Audit**: `audit.rs` warns when `SKILL.toml` contains `[reference]` but allowlist empty or `pyskill.enabled=false`.
10. **No auto-authoring**: refiner does NOT write `[reference]` in 016; that is 017.

## Out of scope

- Renaming `vendor/prime-agent` on disk (see decision log)
- Changing `KernelSettings` field from `kernel` to top-level (would break config structure)
- Auto-authoring of skill references by background review (defer to 017)

## Verification

- `uv run --directory kernel-sidecar pytest -q` (14)
- `cargo test -p operant-core --lib tools::kernel` (6)
- `cargo test -p operant-core --lib` (1725)
- `cargo clippy -p operant-core --all-targets -- -D warnings`
- Manual: `operant tools list` shows `kernel` toolset when enabled; `operant.example.toml` parses
