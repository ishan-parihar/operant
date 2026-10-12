---
name: plugin-authoring
description: Author a kernel-driven plugin for the operant harness (plan 016 Phase 5). Use when the task is "add a new tool", "add a hook", "add a prompt section", or "write a kernel plugin". Walks the author from intent to a signed, hot-swap-ready provider.
metadata:
  operant: {}
---

# Plugin Authoring for the Operant Harness

A plugin in the harness-kernel paradigm is a `Provider` whose `activate`
installs one or more `Effect` handles through kernel seams. The kernel
takes care of dispatch, late binding, transactional swap, and unwind.

## Five-step author loop

1. **Intent** — what capability does the agent need? Pick the seam:

   | Seam              | Family               | Use when                                       |
   |-------------------|----------------------|------------------------------------------------|
   | `tool/<name>`     | tool family          | You want the model to call it                  |
   | `hook/<name>`     | hook family          | You want to observe or modify events           |
   | `prompt/<name>`   | prompt section family| You want extra text in the system prompt       |
   | `memory.provider` | provider selection   | You're swapping the memory backend             |
   | `gateway.command` | gateway command map  | You want a new slash command                   |
   | `channel.adapter` | platform adapter     | You're wiring a new chat platform              |

2. **Sandbox validate** — write the implementation in a workspace
   branch. Use the `code_execution` tool with the existing AFT/test
   harness to confirm: the seam `install` succeeds, the effect-undo
   removes exactly the resource added, the kernel `dump` shows the new
   provider with the expected claims. Run `cargo test -p operant-harness`
   and the relevant seam test module.

3. **Sign** — if the provider is a WASM module, sign it with Ed25519
   (`operant-plugins/src/signature.rs`). The harness re-validates the
   signature on every load; unsigned WASM is rejected by policy.

4. **Mount** — `harness_mount` is the model-tool entry point. Call it
   with a `config_row` (id, source, kind, config). Approval policy
   membership is required. The kernel records the mount in the audit
   log. On success, the new resource is visible in the next
   `harness_dump` call.

5. **Observe & decide** — watch for at least 5 turns. If the plugin
   caused errors, regressions, or value drift, call `harness_unmount`
   (same approval gate). Otherwise leave it mounted; the next config
   patch can promote it to a permanent config-row.

## Red lines (read these before writing code)

- **Provider store carries NO skill/memory/prompt-note kinds.** The
  provider store is for capabilities (tools, hooks, prompt sections,
  channel adapters, gateway commands, memory providers). The curator
  and background_review lanes own skills/memory/prompts; do not put
  those in a provider.
- **Stateless across swaps.** Durable state lives host-side (seam
  slots, registries). Your plugin MUST NOT keep in-memory state across
  a swap — the kernel will throw it away.
- **Undo is bounded work.** No network calls, no unbounded awaits in
  the effect-undo closure. Clippy lint + review convention enforces.
- **Never write to another pool directly.** If the plugin is talking to
  a hermes pool, use the pool's DECLARED interfaces (read-only
  adapters in Phase 6).

## Worked example: a new prompt section

Suppose the agent needs a "current-deps" section listing installed
Python deps in `requirements.txt`. Goal: `harness_dump` shows
`prompt/current-deps` as a mounted provider; next turn's prompt
includes the deps.

```rust
use std::sync::Arc;
use operant_runtime::agent::prompt::{PromptSection, PromptContext};
use operant_harness::{ActivateCx, Provider, ProviderSpec, ProviderSource, Effect, Claim, HarnessError};

struct CurrentDepsSection {
    body: String,  // pre-rendered, captured at activate time
}

#[async_trait::async_trait]
impl PromptSection for CurrentDepsSection {
    fn name(&self) -> &str { "current-deps" }
    fn build(&self, _ctx: &PromptContext<'_>) -> anyhow::Result<String> {
        Ok(self.body.clone())
    }
}

struct CurrentDepsProvider;

impl ProviderSpec for CurrentDepsProvider {
    fn id(&self) -> &str { "current-deps" }
    fn source(&self) -> ProviderSource { ProviderSource::Native }
    fn provides(&self) -> &[Claim] { &[] }
    fn requires(&self) -> &[Claim] { &[] }
}

#[async_trait::async_trait]
impl Provider for CurrentDepsProvider {
    fn spec(&self) -> &dyn ProviderSpec { self }
    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
        let body = std::fs::read_to_string("requirements.txt")
            .unwrap_or_default();
        let section: Arc<dyn PromptSection> = Arc::new(CurrentDepsSection { body });
        cx.install_with("prompt", "current-deps", &section).await?;
        Ok(())
    }
}
```

Compile, run the seam tests (`cargo test -p operant-runtime --lib
prompt_seam`), then call `harness_mount` with the config row.

## Acceptance

- The plugin is reflected in `harness_dump` (read-only tool) within one
  turn of mounting.
- The kernel audit log shows mount + (when applicable) unmount with
  the provider id and source.
- If the plugin fails validation, the previous tree keeps serving; the
  failed candidate is disposed and surfaced via a structured error.
