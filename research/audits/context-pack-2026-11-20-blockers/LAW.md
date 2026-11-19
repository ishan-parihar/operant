# Law — the rules a proposal must survive (2026-11-20)

These are the operator's binding constraints. A finding that proposes violating one of
these is not a finding; it is a proposal the operator will reject. The reviewer should
treat these as the fitness function, not as suggestions.

## Design preferences (DO NOT CHANGE)

- **Voice/TTS**: Phonon (`phonon-31m-en`, GGUF q8_0, ~65MB). Do NOT switch to Edge TTS,
  OpenAI TTS, or any other provider as the default.
- **Memory**: memory-wire (in-process, cargo git-dep). The only engine. Do NOT re-add
  legacy providers (agentmemory, tdg, hindsight, retaindb, mem0, local-vector).
- **Browser/Web**: sourcehound (binary over MCP stdio). Do NOT switch the default away.
- **Gateway**: 7 platforms (Telegram, Discord, Slack, WhatsApp, Email, SMS, Webhooks).
  Do NOT re-add purged platforms (matrix, mattermost, signal) without a real adapter.
- **Native tool integrations**: AFT (20 IDE tools), sourcehound, ponytail (content).
  Do NOT add a LifeOS/Notion built-in tool surface (it was removed; the `ntn` CLI + skill
  is the Notion path).
- **WASM plugins**: Extism host ON by default. One manifest authority (`manifest.toml`).
  Do NOT re-add Cordis's service-store/fiber machine or patch language.
- **Context management**: tiered eviction + decay curve. Prompt-cache frozen prefix +
  volatile suffix. Do NOT change the eviction order (T3→T2→T1).
- **One home resolver**: `operant_core::platform::operant_home()`. Every store resolves
  through it. Do NOT build a second home resolver.

## The iteration loop (binding on every change)

- Every change is one iteration: commit + push to `origin/main` + deploy.
- A change that lives only in a working tree is NOT done.
- Verify locally before push: `cargo fmt`, `cargo check -p <crate>`, `cargo test`,
  `clippy-warning-gate.sh`. Then commit, push, release-build, deploy, restart.
- The iteration number comes from `origin/main`, never local history.

## The parallel-agent rules (load-bearing)

- Never `git reset --hard`, `git clean`, or `checkout` over a peer's uncommitted WIP.
- Never `git add -A`. Stage explicit paths only.
- Push immediately after every commit. An unpushed commit in a shared tree is one peer
  `reset` away from being orphaned.
- If a peer's subsystem is red, do not fix it and do not block on it. Prove your delta
  is zero, push.

## What a finding must carry to be actionable

- A `file:line` citation the operator can open.
- A proposed fix that does not violate any law above.
- A falsifier: what would prove the finding wrong.

A finding without these is a judgment call, not a blocker. The operator will not act
on it, and the round was wasted.