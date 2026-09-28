# operant — Completion & Deployability Plan

Written 2026-09-28, against `origin/main` @ `79de8c31` (iter-430).

This is not the jcode-parity plan. That one (`.omo/plans/jcode-parity-integration.md`)
closed out at Phase 8. This one answers a different question: **what stands between
the current tree and a binary a stranger can install and run.**

Everything below is evidence-backed. Where something is unverified it says so.

---

## Where the project actually stands

**Verified good:**

- `origin/main` compiles clean from a fresh worktree with submodules initialised:
  `check --workspace` → exit 0 (5m01s).
- 801/801 `operant-cli` tests, 9,484 workspace tests at iter-423.
- The release pipeline is *structurally* correct. `build.yml` triggers on
  `tags: ['v*']`, builds 9 native+cross targets into `dist/`; `release.yml`
  chains off Build and extracts notes from `docs/CHANGELOG.md`, which **does**
  contain a `## [0.2.0]` section matching the workflow's regex. Crate version
  (`0.2.0`) and changelog agree.

**The uncomfortable truth:** the binary is not deployable today. Not because of
missing features — because of two hangs/crashes on non-interactive paths, a
native build chain that only works on one machine, and a release pipeline whose
success gate can be tripped by a target nobody needs.

**Scale:** 17 crates, ~570k lines of Rust in `crates/*/src`. `operant-core`
131k, `operant-cli` 95k, `operant-runtime` 89k, `operant-channels` 75k.

---

## The three things people conflate

1. **Feature parity** — done, to the extent it was worth doing.
2. **Correctness on a user's machine** — *not* done. Two blockers below.
3. **Release mechanics** — mostly built, never run. No `v0.2.0` tag exists
   despite **1,118 commits since v0.1.4**.

Phases A–D are what make it shippable. E–H are what make it good. I–J need you.

---

## Phase A — Ship blockers

Nothing ships until these close. Both are on non-interactive paths, which is
exactly where CI, scripts, and pipes live.

### A1. `operant chat` loops forever on non-TTY stdin — BLOCKER

`operant chat </dev/null` warns "no TTY detected — falling back to
non-interactive mode" and then loops anyway, printing `You: ` without bound.
**33 MB in 20 seconds, never exits.** `operant run --query` handles the same
condition correctly, so the fallback exists and is simply not wired to chat/tui.

The loop must treat `read_line` returning `Ok(0)` (EOF) as terminal. `operant
tui` needs the same check.

*Accept:* `operant chat </dev/null` exits 0 within one second; a test drives the
non-interactive loop against an empty `Cursor`.

### A2. Piping any long-output command aborts with a core dump — MAJOR

`[profile.release] panic = "abort"` (Cargo.toml:125) plus Rust's default
`SIGPIPE`-ignored disposition means the first `println!` after a closed pipe
panics in the stdout write path, and `panic = "abort"` turns that into SIGABRT.

```
$ operant doctor 2>/tmp/e | head -5 ; echo ${PIPESTATUS[0]}
134
thread 'main' panicked at library/std/src/io/stdio.rs:1166:9:
failed printing to stdout: Broken pipe (os error 32)
timeout: the monitored command dumped core
```

Same command redirected to a file exits 0. `operant doctor | head` is ordinary
usage. Restore the default `SIGPIPE` disposition at startup — a two-line change
in `main()` that protects every current and future command.

*Accept:* `operant doctor | head -5` exits 0, no core dump, no panic.

---

## Phase B — Make a fresh machine able to build it

This is the phase that decides whether "install" means anything. I did **not**
run a fresh release build (too slow), so the failure mode below is
mechanically-evidenced, not reproduced.

### B1. The documented install path provisions none of the native build deps

- `crates/operant-core/build.rs:7` unconditionally emits
  `cargo:rustc-link-lib=sonic`.
- `scripts/install.sh:9` runs a bare `cargo build --release -p operant-cli`. It
  does not source `dev-env.sh` and does not call `provision-build-deps.sh`.
- `kokoro-tiny = "0.1.0"` (`operant-core/Cargo.toml:54`) is **non-optional**,
  pulling `ort`, `ort-sys`, `espeak-rs-sys` and `cpal` — all in `Cargo.lock` —
  which need libclang, ONNX Runtime, cmake and alsa.
- Both env scripts hardcode a path that does not exist on this machine
  (`/home/z/my-project/local/...`, `/home/z/.venv/bin`). This box is
  `/home/ishanp`. `.cargo/config.toml` says outright that the native search
  path "is NOT configured here" and must be set per machine.

*Accept:* on a clean machine with only rustup + build-essential,
`./scripts/install.sh` either succeeds or fails with one actionable sentence
naming the missing package.

### B2. `local/lib/libsonic.so` is a committed dangling symlink

```
$ git ls-tree -r HEAD --long | grep local/
100644 blob 3cee4bc8    18354  local/lib/libsonic.a
120000 blob e9783ef5       39  local/lib/libsonic.so   <-- symlink
100644 blob bd1ad54b 7623496  local/libclang.deb       <-- 7.6 MB .deb in git

$ git cat-file -p e9783ef5
/usr/lib/x86_64-linux-gnu/libsonic.so.0
$ ls /usr/lib/x86_64-linux-gnu/libsonic.so.0
No such file or directory
```

Only the 18 KB `.a` rescues the link here. `build.yml` nonetheless targets
`aarch64-apple-darwin`, `x86_64-pc-windows-msvc` and 6 musl/cross targets,
where an ELF/Linux `.a` is useless. **Whether those legs link is unverified.**

Also: a 7.6 MB binary `.deb` committed to git is repo hygiene worth settling.

*Accept:* `cargo build --release` works on Linux, macOS and Windows with no
machine-specific path baked in; `libsonic` is either optional behind a feature
or resolved per-target.

---

## Phase C — Make the release pipeline able to publish

### C1. One failing cross leg blocks every release — MAJOR

`release.yml:14` gates on `github.event.workflow_run.conclusion == 'success'` —
the **whole** Build workflow. `build.yml`'s `cross` job fans out to 6 targets
(musl x64/ARM64, ARM64-gnu, ARMv7, Android ARM64, FreeBSD) using `cargo install
cross` on the fly. `fail-fast: false` still yields a failed conclusion. So one
unnecessary target failing means no GitHub Release is ever published.

Publish from the `native` job's artifacts; let best-effort targets fail
independently.

### C2. Dead and fragile steps

- `build.yml:119-122` clones `tdg-rust` in the cross job, contradicting
  `build.yml:50-52` which states the clone "was removed". The repo still exists
  so it doesn't fail — it's pure waste.
- `release.yml:56` calls `python`, not `python3`. Reliable enough on
  GitHub's images, not guaranteed.
- Every checkout uses `submodules: recursive`, so **every** build depends on
  the external `vendor/prime-agent` repo being reachable — for a feature
  (`[pk]`) that ships `enabled = false`.

*Accept:* a release publishes even when every cross target fails; a release
publishes with the submodule unreachable.

---

## Phase D — Ship 0.2.0 (mechanical, ~an afternoon)

This is not engineering. It is a checklist, and it is the actual finish line.

1. **Resolve the 22 uncommitted files.** All belong to a concurrent agent
   (BUGS.md, `operant-cli/src/config.rs`, operant-core/-harness/-runtime/
   -plugins). A release cannot be cut from a dirty tree. Either land them or
   stash them deliberately.
2. **Update `docs/CHANGELOG.md`.** `[Unreleased]` stops at iter-409; it is 21
   iterations stale. Move 410–430 in.
3. **Tag `v0.2.0`.** Crate version, changelog section and workflow regex all
   already agree — this just has not been done.
4. **Deal with the orphaned `v0.1.3` / `v0.1.4` tags.** Both are lightweight
   tags pointing at commits reachable only from `archive/stash-*` branches,
   never from `main`. `git describe` correctly reports `v0.1.2`. Decide whether
   to delete them or document them, because a `git tag` listing that lies about
   the release history is worse than no tags.
5. **Note, don't fix:** `iter-347` and `iter-413` are each used twice in pushed
   history. AGENTS.md forbids the force-push that would repair it.

---

## Phase E — First-run quality

### E1. The first error a new user sees is internal jargon

```
$ HOME=$(mktemp -d) operant run --query "hi"
warning: no TTY detected — falling back to non-interactive mode
Error: Agent error: Credential pool for provider 'openai' has no available keys
```

No mention of `operant setup`, even though `operant doctor` gets it right.
Detect "no config AND no credentials" before constructing the agent.

### E2. `operant doctor` always exits 0

`cmd_doctor/mod.rs` ends in an unconditional `Ok(())`. It printed "Found 4
issue(s) to address" and still exited 0, so `operant doctor && operant chat`
proceeds into a guaranteed failure. Add `--strict`, or exit 1 on non-manual
issues.

### E3. Dead doctor check

`cmd_doctor/checks_tools.rs:678` probes for a `tinker-atropos` submodule that
is in neither the repo nor `.gitmodules`, and prints
`git submodule update --init --recursive` as the remedy — which cannot fix it,
since only `vendor/prime-agent` is registered. Unactionable advice.

### E4. Shipped binary has no dashboard frontend

`operant-gateway/build.rs` only activates `embedded-web` when `web/dist/index.html`
exists. `web/` does not exist anywhere in the repo. So the feature is never on
and the released binary falls back to a filesystem `web_dist_dir` — a
single-binary install gets a 404 dashboard. Either vendor `web/dist` or stop
advertising the embedded path.

---

## Phase F — Config template sync

`operant.example.toml` parses clean (verified — important, because every
settings struct is `deny_unknown_fields`, so a *stale* key would be a hard boot
failure; there are none). But it does not document the config surface:

- **9 top-level keys absent:** `version`, `vision`, `credential_pool`,
  `auxiliary_models`, `checkpoints`, `pk`, `database_path`, `providers`,
  `command_allowlist`
- **5 whole sections absent:** `[vision]`, `[credential_pool]`, `[checkpoints]`,
  `[pk]`, `[auxiliary_models]`
- **13 fields absent** within existing sections
- **49 keys present but commented out**

AGENTS.md requires example/Rust sync in the same change. This has drifted.

One substantive mismatch: `BrowserSettings` defaults to `provider = "obscura"`
in Rust, while `operant.example.toml` sets `igs`. Documented default and actual
default disagree.

---

## Phase G — Dead code in the shipped binary

From BUGS.md, unfixed and material:

- **R15-1 — `operant-channels` is 75k lines and dead-linked.** Plus a
  14,094-line channels orchestrator. Nothing in the shipped binary reaches it.
- **R40-13 (HIGH) — 4 default-feature crates sit in the normal dependency graph
  with no reference from `operant-cli/src`.** They are compiled in and never
  called.
- **R40-12 — AGENTS.md's "7 platforms only" is false.** 23 channel features ship
  in the default build.
- **R5-3 — `operant-runtime`'s `RuntimeAgent` stack is dead-linked legacy**, and
  `operant-runtime` is 89k lines.
- **R13-3 — `run_gateway` has zero callers in the shipped binary.**
- **R13-4 / R13-5 — `[memory] audit_enabled` and the response cache are dead
  config.**

Together these are the largest single lever on binary size, attack surface and
"why does this behave like that" confusion. Pick one and prove it: find the
entry point that *should* reach the code, or delete the code.

---

## Phase H — CI and doc health

- **R40-19 — 74 rustdoc errors** under `-Dwarnings`. The `doc` job in `ci.yml`
  is gated off (`if: false`), so it cannot catch regressions. Fixing 74 errors
  and re-enabling the job is a real afternoon.
- **R40-18 — the fmt sweep is blocked** on the concurrent agent's files.
- **R40-7 — the main-branch trigger** was fmt-blocked at iter-369.

Green CI on `main` is what makes a release trustworthy. Right now the `doc` and
`msrv` jobs are both disabled.

---

## Phase I — Blocked on you (design, not engineering)

I will not decide these unilaterally; each changes intended appearance.

1. **Theme the transcript.** `messages/mod.rs` has 7 local colour consts and 28
   use sites, zero `theme_colors` references. The default theme is warm cream +
   dim amber; the transcript is a deliberate cool grey. Theming it means
   choosing values for 8 themes × ~3 roles.
2. **The remaining colour literals** (71 explicit + 188 `DarkGray`/`Black`).
   I refuted the automatic migration four separate ways — value mismatch on the
   transcript, ANSI-slot invariance on `DarkGray`, near-invisible `disabled()` on
   the light theme, role mismatch on `text_selection_bg`. Every remaining site
   is a per-theme appearance choice.
3. **Should Ctrl+A / Ctrl+B work in a focused prompt?** They currently open the
   model picker and branch browser. Both are now catalogued truthfully, so
   `/keys` is honest — but readline muscle memory still can't reach them.
   Granting the prompt precedence removes two working shortcuts.

---

## Phase J — Deliberately deferred, with reasons

Not forgotten. Each is a decision, not an omission.

- **`semantic_compaction_cutoff` stays unwired.** iter-406 measured hybrid
  retrieval as *worse* than plain compaction (R@3 2/5 → 1/5). Wiring an unproven
  primitive is how a knob comes to look like protection while providing none —
  `safe_compaction_cutoff` sat wired for ~50 iterations protecting nothing before
  iter-407 fixed it.
- **No pin for `Prompt`/`Dialog`/`Completion` keybindings.** A string-literal
  probe is the wrong tool — those dispatch on `KeyCode`, not `&str`. The Global
  and all six vim contexts are pinned (iter-409/420/426/427).
- **ACP remainder.** `session/load`, token-level streaming and non-text content
  blocks are still absent. iter-422 shipped the four methods real clients need
  to connect at all.

---

## Recommended order

**A → B → C → D** gets you a released, installable 0.2.0. That is the finish
line, and A is two small fixes.

**E → F → H** make it trustworthy and honest. H matters most for CI confidence.

**G** is the largest engineering effort and the biggest size/surface win.

**I** needs you. **J** is closed.
