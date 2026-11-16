# Process registry and terminal backends — background work and where it runs

Owns: the process registry behind the `process` tool (spawn, poll, wait, kill,
the output buffer, the reaping rule), and the terminal execution backends
(`terminal` tool over local / Docker / SSH, the rtk rewrite path, the failure
hints).
Not for: the code sandbox (`code_execution`, a different tool with its own
module), the tool registry itself ([tools.md](tools.md)), or approval and
guardrails ([security.md](security.md)).

Source: `crates/operant-core/src/process_registry.rs` (the registry, sessions,
reaping), `crates/operant-core/src/tools/process_tool.rs` (the `process` tool),
`crates/operant-core/src/tools/terminal_tool.rs` (the `terminal` tool and the
rtk path), `crates/operant-core/src/tools/terminal_backend.rs` (the backends),
`crates/operant-core/src/rtk_bridge.rs` (the rtk subprocess bridge),
`crates/operant-core/src/terminal_hints.rs` (failure hints).

## The rule

> A process the agent starts is the agent's child, not the operator's daemon:
> it is tracked, it is bounded, and it dies when the agent does.

Two mechanisms enforce that and they are separate concerns: the REGISTRY owns
lifetime and output (who spawned what, how much it may emit, who reaps it), and
the BACKEND owns where a command runs at all (this host, a container, a remote
box). The `process` tool is the model's handle onto the first; the `terminal`
tool is its handle onto the second.

## The process registry

`ProcessRegistry` holds two maps: `running` (sessions still executing) and
`finished` (completed ones, kept for polling). A `ProcessSession` is one
background process:

| Field | Meaning |
|---|---|
| `id` | `proc_` plus 12 hex chars — the handle the model passes back |
| `command`, `cwd` | what was spawned |
| `pid`, `started_at` | the OS pid and the wall-clock start |
| `status` | `ProcessStatus::{Running, Exited, Killed, Failed(String)}` |
| `exit_code` | set when the wait task observes the exit |
| `output_buffer` | the tail of the combined output |
| `notify_on_complete` | STORED, NEVER READ — nothing in the crate acts on it (RT-347) |

`notify_on_complete` is named here because it is the registry's one honestly
dead field: the flag stays for the serialized shape, and the tool argument that
used to set it was retracted alongside it.

### Lifetime rules

1. **A child runs in its OWN process group** (`process_group(0)` on unix at
   spawn, `cmd /c` on windows). That is what makes a kill reap the whole tree: a
   lone TERM to the shell pid orphans `cmd &` descendants, which would otherwise
   keep the stdout pipe open forever and hang the session's wait.
2. **`Drop` for the registry kills every running child.** The registry had no
   `Drop`, so a background command outlived the agent that started it — the
   child held the stdout pipe, kept its resources, and nothing could kill it but
   an explicit `kill`. It runs in `Drop` so it cannot await: a plain `kill`
   command, a failure logged and never fatal.
3. **Output is trimmed to the tail**, and the trim is CHAR-BOUNDARY safe: the
   first implementation cut with a byte slice, so a multi-byte character
   straddling the cut panicked INSIDE the spawned reader whose `JoinHandle` is
   discarded — the drain silently stopped, the child blocked on a full pipe, and
   the session only completed at the wait timeout.
4. **The finished map is pruned to `MAX_PROCESSES = 64`.** The oldest finished
   sessions are dropped once a spawn would push the map past the bound, so a
   long-lived agent does not grow it without limit.

### The cap and the buffer

- `MAX_OUTPUT_BYTES = 200_000` — the tail kept per session. It is the registry's
  own bound, separate from the terminal tool's `max_output_bytes` (1,000,000):
  the registry keeps the last 200 KB of a background session so `get_output` on
  a chatty build does not return a megabyte into the context.
- `MAX_PROCESSES = 64` — the finished-map bound (above).

### The `process` tool

Registered name `process` (`tools/builtin.rs`), actions dispatched by a `action`
string:

| Action | Requires | Returns |
|---|---|---|
| `list` | nothing | every session (running + finished) with `running_count` |
| `spawn` | `command`, optional `cwd` | the new session |
| `poll` | `session_id` | the session now, running or finished |
| `wait` | `session_id`, optional `timeout_secs` | the session when it settles, or now at timeout |
| `kill` | `session_id` | `{ "killed": true }` |
| `get_output` | `session_id` | the session's output, status and id |

The description is unusually explicit about one thing, and it is the commonest
model error: `list` is the agent's OWN background sessions, NOT the operating
system's process table. An empty list means nothing was spawned, not that the
machine has no processes. A missing `session_id`, or an unknown action, is an
error result naming the action it wanted — the model reads it and corrects, not
a loop-level failure that ends the turn.

## The terminal backends

`TerminalBackend` is a trait with one method (`execute_command`, taking the
command, an optional cwd, env vars, whether to use a shell, a timeout, and an
output cap) plus `name()` and `is_available()`. `create_backend(&AppConfig)`
picks the implementation from `config.terminal_backend`:

| Config value | Backend | Notes |
|---|---|---|
| `local` (the default) | `LocalBackend` | direct execution; a shell only when `use_shell` is set |
| `docker` | `DockerBackend` over `DockerConfig` | `docker exec` into a container |
| `ssh` | `SshBackend` over `SshConfig` | `ssh` with a control socket |
| `modal`, `daytona`, `vercel_sandbox`, `singularity` | **none** | REFUSED with an error, not downgraded to local |

The refusal is deliberate and worth stating: a hermes-style
`terminal_backend = "modal"` config must not run commands on the host just
because the Rust port does not implement that backend. Hermes implements
modal/vercel/daytona; the Rust port implements three, and it fails closed
rather than silently unsandboxing.

### Docker

`DockerTerminalSettings` (default) is image
`nikolaik/python-nodejs:python3.11-nodejs20`, `cpu = 1.0`, `memory_mb = 5120`,
no volumes, no env. `create_backend` overrides `cwd` to `/root` and honours the
`HERMES_DOCKER_BINARY` override for the docker executable, falling back to
`which docker` and then the default name. Each command is a `docker exec` with
`-e` per env var, `-w` for the cwd, and the container id first.

### SSH

`SshTerminalSettings` (default) is `port = 22` with no host, user or key. The
backend keeps a control socket under `<temp>/operant-ssh` and neutralizes every
model-controlled component before it reaches the remote shell — hermes parity
with `shlex.quote`. A remote host that is unreachable is an error result, never
a silent local fallback.

### The env a child gets

`sanitized_env` is the single choke point: the parent environment MINUS any
variable whose name looks credential-bearing (`SECRET_ENV_FRAGMENTS`: `API_KEY`,
`APIKEY`, `_TOKEN`, `TOKEN_`, `SECRET_`, `_SECRET`, `PASSWORD`, `PASSWD`,
`CREDENTIAL`, `PRIVATE_KEY`, `SESSION_KEY`), plus the caller's overrides, which
win. It is deliberately a DENYLIST rather than an allowlist: an allowlist would
break every legitimate command that needs `PATH`, `HOME`, `LANG`, or a user-set
variable. The fragments are suffix- or token-shaped rather than bare substrings,
because a bare `AUTH` would strip `SSH_AUTH_SOCK` and break every `ssh` / `scp`
/ `git push`, and a bare `KEY` would strip `SSH_KEYSCAN`.

## The rtk rewrite path

`[compression] shell = "rtk"` makes the `rtk` binary the EXECUTOR for terminal
commands — a backend substitution, not a second filter, so operant does not then
compress the result a second time (the evidence layer skips shell results in
this mode).

`rtk_bridge` is the seam. `find_rtk()` resolves the binary in order: the
`RTK_BINARY` environment override (accepted only if it is an existing file),
then `which rtk` (accepted only if it answers `--version`), then `None`. A dead
shim is never mistaken for a working binary.

Two rules govern the rewrite, and both are load-bearing:

1. **The rewrite is a MAPPING, not a permission.** `rtk rewrite` answers "how
   would I run this"; its exit code is rtk's own verdict, not operant's. A
   rewrite that introduces shell syntax is only used when the command was ALREADY
   going through a shell — otherwise `ls | grep foo` rewritten to
   `ls | rtk grep foo` would be handed to a direct exec, which would run `ls`
   with the rest as arguments, a silent change of meaning.
2. **The fence sees the command that WILL run.** `holon_command_fence_with_shell`
   runs on the rewrite, not the original. And when the fence refuses the REWRITE,
   the ORIGINAL runs instead: the rewrite is an optimisation, and an
   optimisation that is not allowed must not become a refusal of the command the
   model actually asked for.

`RTK_COLLIDING_SUBCOMMANDS` (11 names: `test`, `read`, `log`, `diff`, `grep`,
`env`, `init`, `json`, `find`, `ls`, `tree`) are rtk's own subcommands that are
ALSO real binaries. Rewriting a command whose FIRST token is one of them would
run rtk's subcommand instead of the real binary, so `is_rtk_command` short-
circuits to no rewrite. Only the first token is inspected: a colliding name
further down a chain (`git status && ls`) is still rtk's to rewrite.

`RewriteVerdict` is four-valued: `Rewritten { command }`, `NoRule` (run it
unchanged), `Refused { reason }` (a deny rule matched; it must NOT run), and
`Failed { reason }` (rtk itself broke; run the original unchanged). Exit 0 and
exit 3 BOTH print the rewrite, so both classify as a rewrite when stdout
confirms one; exit 1 with no output is `NoRule`; exit 2 is `Refused`; anything
else is `Failed`.

## Failure hints

`terminal_hints::annotate_failure(command, exit_code, output)` returns an
actionable `💡 Try: ...` line for a failed command, which the terminal tool
attaches to the model-visible error content so the model can self-correct without
a human turn. Seven heuristics: `gh` unknown json field, command not found
(cargo / npm / gh, each with its own install line, and a generic fallback),
Python module not found, `cargo: command not found`, `cd: no such file or
directory`, `npm: command not found`, `git: not a git repository`.

The truncation markers are sniffed, not reported: `CommandOutput` carries no
"this stream was cut" flag, so the tool checks whether the stream ENDS with the
backend's own marker (`[output truncated]` for the local backend,
`...[truncated]` for docker and ssh) and says so in the envelope. Without that,
the model cannot tell "the command produced 200,000 characters" from "the
command produced 1 MB and the tool cut it at 200,000".

## Configuration

| Key | Default | What it changes |
|---|---|---|
| `terminal_backend` | `local` | which backend `create_backend` builds; `modal`/`daytona`/`vercel_sandbox`/`singularity` are refused, not downgraded |
| `[tools.terminal] max_timeout_secs` | 300 | the hard ceiling on one terminal call |
| `[tools.terminal] max_output_bytes` | 1,000,000 | the terminal tool's output cap |
| `[tools.terminal.docker] image` | `nikolaik/python-nodejs:python3.11-nodejs20` | the container image |
| `[tools.terminal.docker] cpu` / `memory_mb` | 1.0 / 5120 | the container's limits |
| `[tools.terminal.ssh] host` / `user` / `port` / `key_path` | none / none / 22 / none | the remote target |
| `[compression] shell` | unset | `"rtk"` routes terminal commands through the rtk bridge |

The registry's own bounds are constants, not config: `MAX_PROCESSES = 64` and
`MAX_OUTPUT_BYTES = 200_000` in `process_registry.rs`.

## Failure modes

| What breaks | What you see |
|---|---|
| `terminal_backend = "modal"` (or daytona / vercel_sandbox / singularity) | startup-time error: "Terminal backend 'modal' is not implemented in the Rust port — refusing to fall back to unsandboxed local execution" |
| rtk configured but not installed | the command runs UNCHANGED and the runtime label says `rtk not found — commands were not compressed` |
| rtk denies the command | "rtk denied this command and it was not run: `<reason>`" |
| The docker CLI is absent | the command fails at `docker exec` with an error result |
| The ssh host is unreachable | an error result, never a silent local fallback |
| A background child outlives its agent | it is killed in `ProcessRegistry::drop`, whole process group |
| A session's output exceeds the buffer | the tail is kept (200 KB), trimmed on a char boundary |
| `process` called with an unknown action | an error result naming the valid actions |
| A `process list` that is empty | nothing was spawned — it is NOT a verdict on the OS process table |

## Intention

The registry exists because the agent needs long-running work (a dev server, a
build watcher, a REPL) that must survive across turns but must never outlive the
agent: an untracked child is a resource leak and a security surface, and one that
survives its parent is a process nobody can account for. `Drop` and the
process-group spawn are the two halves of that promise.

The terminal backends exist because "run a command" is not one environment: a
local agent, a containerized agent and a remote agent all need the same tool, and
the config choice has to be honest about what it can and cannot do. That is why
the unimplemented backends REFUSE instead of falling back — a silent downgrade to
unsandboxed local execution would be the one failure an operator could not
detect from the outside.