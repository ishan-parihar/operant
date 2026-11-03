# Architecture

**What this document is.** The shape of the whole system on one page, written
for someone who will not read the code. If you want the interface list, `docs/`
holds it. If you want to know what this thing *is*, this is the page.

Version 0.3.0, Rust edition 2024.

---

## The one sentence

Operant is a runtime for AI agents that have to do real work on a real machine:
it holds the tools, the memory, the schedule and the permissions in one place, so
that an agent can be pointed at a job and left to run it without anyone watching.

---

## The seven parts

A Rust workspace of seven crates. They are separate because they change at
different rates and are tested separately, and they are one program because they
ship together.

| Crate | What it is | Why it is its own crate |
|---|---|---|
| `operant-api` | The contracts every other crate agrees on. No implementations, no heavy dependencies. | The compiler stops one part importing another directly, so the boundaries hold under pressure instead of only in the design document. |
| `operant-core` | The agent loop: call a model, decide, run a tool, see the result, repeat. | This is where the product lives, and it changes more than anything else here. |
| `operant-os` | The operating layer: the schedule, the execution record, the read surface, the events. | Persistence and bookkeeping are a different concern from reasoning, and they fail differently. |
| `operant-harness` | How capabilities register themselves: claims, late binding, reversible effects. | Extension is a mechanism with its own rules, and it is the part that has to stay stable while features move. |
| `operant-config` | The settings file, the schema that checks it, and where secrets go. | Configuration is the only part of the system a non-developer touches, so it has its own tests and its own errors. |
| `operant-plugins` | WebAssembly extensions that add tools and channels without a rebuild. | Plugin code runs untrusted, so it needs its own sandbox boundary and its own error types. |
| `operant-cli` | Every command a human types. | The interface changes fastest and breaks nobody when it does. |

Two vendored crates sit outside the workspace and are excluded from the build:
`vendor/ndarray-npy` and `vendor/kokoro-tiny`. They are pinned copies of things
this project needed at a specific version.

---

## How a job runs

```
 you: a CLI command, a schedule, or a message
        |
        v
   operant-config ....... the settings file is checked before anything starts
        |
        v
   operant-core ......... the loop: think, act, observe, repeat
        |    ^
        |    |
        v    |
   tools + memory ....... registered through operant-harness
        |
        v
   operant-os ........... what ran, what it did, what it cost
```

The loop is the whole product. Everything else is either what the loop needs or
what keeps it honest.

**What the loop owns:** deciding what to do next, calling the model, running the
tools in parallel where they do not depend on each other, and feeding results
back.

**What the loop does not own:** where the schedule comes from, where the record
is stored, or who is allowed to run. Those are `operant-os` and
`operant-config`, and the split is deliberate: reasoning is the part that changes,
bookkeeping is the part that must not.

---

## The boundaries that matter

**Nothing imports what it is not allowed to.** `operant-api` holds the contracts
and every other crate depends on it. The compiler enforces this: an
implementation crate that reaches into another implementation crate fails to
build, not just to pass review.

**Every registration can be undone.** When a capability registers, it gets a
handle back, and unloading unwinds in reverse order. This is what makes
long-running operation safe: a plugin that misbehaves can be removed without
restarting the whole process and losing the work in flight.

**Replacement is all-or-nothing.** When a capability is swapped, the new one is
installed only if it initialises. If it fails, the old one is still there. A
counter distinguishes "the version I am looking at is the version I registered"
from a swap that happened while I was not looking.

**Requirements can arrive late.** A capability can declare what it needs without
that thing existing yet, and it activates the moment its dependency appears. The
alternative, loading everything in a fixed order, is what makes agent systems
fragile at startup.

**The operating layer is a read surface, not a second engine.** The SQL views it
exposes are the sanctioned way for a governor to see what this runtime did. The
governor (holon) decides; this runtime records. Neither reaches into the other.

---

## Extension

Three ways in, in order of how much they cost:

1. **Tools** — declared in code, registered through the harness, available to the
   agent immediately. The normal path.
2. **Channels** — how work arrives. Also declared, also registered.
3. **WebAssembly plugins** — code that has not been reviewed, loaded into a
   sandbox. This is the path for anything a third party wrote, and it is the only
   path where a crash cannot take the process with it.

The distinction is load-bearing: the first two are trusted because they are
reviewed, the third is untrusted because it cannot be. Treating them as the same
thing is how a plugin system becomes an attack surface.

---

## Configuration

One settings file, checked against a schema before the process starts. Two rules
that are worth knowing before you deploy this:

- A malformed settings file fails at startup, with a message that names the
  field. It does not fail later, at the moment the value is needed.
- Secrets are not in that file. They live wherever the deployment keeps them, and
  the config crate knows how to read them without logging them.

---

## Testing

The gate is the whole workspace plus a strict lint, and a green run means both:
7,168 tests across 72 binaries, zero failures. `--no-fail-fast` is part of the
method, because a plain run stops at the first failing binary and reports a
fraction of the suite.

Run it yourself:

```
cargo test --workspace --no-fail-fast
```

---

## What this is not

- **Not a kernel.** This is the runtime. The kernel that governs it is holon,
  and the boundary between them is a public contract, not a private
  arrangement.
- **Not finished.** It is active development. A green gate is not a
  production-readiness claim, and this document does not present it as one.
- **Not a single binary you can install and forget.** The install path is a real
  job with real requirements. The README covers it; this page covers the shape.
