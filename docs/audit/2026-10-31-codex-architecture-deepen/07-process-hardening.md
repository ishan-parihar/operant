# Family 07 — process hardening

**Question this file answers:** codex disables core dumps, ptrace attach, and strips
`LD_PRELOAD`/`DYLD_*` before `main()`. operant holds provider API keys in process env. Is this an
import worth making, and what does it actually buy?

**Ground state:** operant `main` @ `f53907e8`. Reference `../external-repos/codex/codex-rs` @ `588a277b`.

---

## CODEX-SIDE

### What the mechanism is

A 193-LOC crate (`process-hardening/src/lib.rs`) designed to run **pre-main via `#[ctor::ctor]`**
(`lib.rs:10`). It does three things on Linux:

1. `prctl(PR_SET_DUMPABLE, 0)` — mark the process non-dumpable, so a same-user process cannot
   ptrace-attach (`lib.rs:66-67`).
2. `setrlimit(RLIMIT_CORE, {0, 0})` — no core files, ever (`lib.rs:99-115`).
3. `remove_env_vars_with_prefix(b"LD_")` — strip every `LD_*` variable from the environment, so a
   hostile library cannot be interposed (`lib.rs:61`, `:127-135`).

It also carries a `io::Result`-returning public for the non-ctor path,
`disable_process_dumping()` (`lib.rs:65-75`), and macOS/BSD/Windows branches (`PT_DENY_ATTACH`,
`DYLD_`, a Windows TODO at `:118-122`).

The failure posture is **fail-closed**: each `libc` call prints an `ERROR:` line and
`std::process::exit(N)` with a distinct exit code per step (`lib.rs:27-45`) — 5 for prctl, 6 for
PT_DENY_ATTACH, 7 for RLIMIT_CORE. It refuses to start rather than continue less hardened.

### The load-bearing unit

| unit | file:line | LOC |
|---|---|---|
| whole crate | `process-hardening/src/lib.rs:1` | **193** |
| — of which tests | `:159-193` | 35 |
| — Linux hardening path | `:65-75`, `:99-115`, `:127-141` | ~55 |

One production dependency beyond `libc`: `ctor` (for `#[ctor::ctor]`). The test module is
interesting for a reason that matters to the import: `env_keys_with_prefix_handles_non_utf8_entries`
(`:170-186`) proves the `LD_` sweep retains **non-UTF-8 env keys**, using raw `OsStr` bytes rather
than `String` — the bug class where a naive `env::vars()` (which panics on non-UTF-8) would crash
at startup.

---

## OPERANT-SIDE

### What exists today: nothing, but the exposure is real

`rg -rn "RLIMIT_CORE|no_new_privs|PR_SET_NO_NEW_PRIVS|setrlimit|prctl|ptrace" crates/` → **zero
hits**. `libc = "0.2"` is already a direct dependency of `operant-cli`
(`crates/operant-cli/Cargo.toml:73`), so no new dependency is needed for the syscalls.

The exposure it would address is real and specific:

- **API keys live in the process environment.** `~/.operant/.env` is read into the environment at
  startup, and `env_passthrough.rs` re-reads it into the live process env on every turn
  (`crates/operant-core/src/env_passthrough.rs` — the module doc at `:19-31` names the exact
  reason: "Writing the process environment alone does not rotate a running agent's credential:
  the client captured its key when it was constructed").
- **Core dumps would contain those keys.** A crash produces a core file holding the whole
  environment of a process that holds `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, etc. On a machine
  where files are 0644 or where core goes to a shared directory, that is a credential leak with no
  exploit required.
- **`ptrace` by a same-user process reads the same memory.** The operant daemon (`operant-gateway`
  systemd unit) runs as the same user as everything else on the box, so this is a low bar to clear
  for a local attacker and a high bar for anyone else.

### Why the file-permission posture makes this *more* relevant, not less

The repo has already paid for the file-side of this exact threat: `auth.json` is 0600
(`AuthStore::save` routes through `fs_secrets::write_secret_file`, iter-1185) and `.env` is 0600.
That work is recorded in the ledger as RT-154, and its lesson was "credentials readable by every
local account for the file's whole life." **The core-dump path is the same threat through a
different channel** — a 0644 or world-readable core file defeats a 0600 `.env`. So the existing
hardening investment argues *for* this row, not against it.

---

## VERDICT

| mechanism | codex unit + LOC | operant equivalent | gap severity | portable? | verdict |
|---|---|---|---|---|---|
| `RLIMIT_CORE = 0` | `process-hardening/src/lib.rs:99-115` (17) | none | **high** — core dumps carry the env, the env carries keys | yes, `libc` already present | **IMPORT-SMALL** |
| `PR_SET_DUMPABLE = 0` | `:65-75` (11) | none | med | yes | **IMPORT-SMALL** |
| Strip `LD_*` env | `:61`, `:127-141` (~15) | none | — | trivially | **DO-NOT — and this is a measured TRAP, not a judgement call.** See below |
| pre-main `#[ctor]` wiring | `lib.rs:10` + `ctor` dep | none | low | yes | DO-NOT — call from `main()` instead |
| macOS/BSD/Windows branches | `:79-122` | none | none on this box (linux) | n/a | DO-NOT |
| Exit-code-per-step fail-closed | `:27-45` | none | low | pattern only | DO-NOT — an early-return warn is enough |

**The measured trap in the `LD_*` row.** I went in expecting `LD_LIBRARY_PATH` stripping to be a
free win and measured the opposite. `target/release/operant` is:

```
ELF 64-bit LSB pie executable, dynamically linked, interpreter /lib64/ld-linux-x86-64.so.2
libonnxruntime.so.1 => /home/ishanp/.local/lib/operant/libonnxruntime.so.1
```

`scripts/dev-env.sh` sets `LD_LIBRARY_PATH="$LOCAL_DIR/lib:$LOCAL_DIR/onnxruntime-linux-x64-1.24.4/lib"`
and `ORT_LIB_LOCATION` for exactly this reason, and the loader is finding `libonnxruntime.so.1`
under `$HOME/.local/lib/operant/`. **Stripping `LD_*` from the environment of this binary would
break the Kokoro TTS default** — the repo's own `[tts] provider = "kokoro"` design preference.
codex is MUSL-linked and static in release (`lib.rs:57-59` says so explicitly: "Official Codex
releases are MUSL-linked, which means that variables such as LD_PRELOAD are ignored anyway, but
just to be sure, clear them here"), so for codex the strip is belt-and-braces on an already-hard
surface. For operant it is a runtime dependency. **The narrow version that would be safe is
stripping only `LD_PRELOAD`**, and even that has to be measured against the repo's own
provisioning before it ships.

**The ponytail shape:** ~45 lines in operant, zero new dependencies (`libc` is already there),
called at the top of `main.rs` rather than through a `#[ctor]`. No new crate, no new abstraction,
no config surface.

**One design decision to make rather than copy:** codex `exit()`s when a hardening call fails.
For an operator-facing agent, a hard exit on a machine that forbids `PR_SET_DUMPABLE` (some
container runtimes do) would turn a cosmetic loss into an unusable binary. The right posture here
is **log-and-continue with a distinguishable warning**, because the alternative is a tool that
refuses to run for a non-security-critical reason. Record that as a deliberate divergence rather
than an oversight.
