# CLOSED — tried and rejected

Append-only. An entry names the item, the verdict, and the measurement that
killed it. Re-proposing a closed item needs new evidence, not enthusiasm.
`kb status`-style verdicts cite the pointer that proves them.

> **Note (2026-10-20, iter-1093):** the line above cites `kb status`, and `kb`
> is dead — the binary is `./scripts/genome`, and the verdict vocabulary lives
> in `genome status`. The line is left as written because this file is
> append-only; read the note, not the command. (Recorded as RG-009.)

- **[2026-10-17]** Re-adding phantom gateway platforms (matrix, mattermost, signal, …) without a real adapter implementation — **rejected**, iter-50 purged 20 phantoms and the registry stays at the 7 platforms with real code; AGENTS.md "Design Preferences" forbids re-adding. Do not re-litigate.
- **[2026-10-17]** Running operant's web/browser engine in-process — **rejected**: the sourcehound `[patch.crates-io]` evaporates for consumers and drags an unconditional render stack; the MCP subprocess seam is the shipped design (AGENTS.md "Path Dependencies").
- **[2026-10-17]** Re-triggering GitHub Actions on push/tag — **rejected**: the Actions quota is exhausted and every run burns minutes; workflows are `workflow_dispatch`-only and all verification is local (AGENTS.md release procedure).
- **[2026-10-17]** Per-worktree `target/` caches for fleet builds — **rejected**, iter-631 by owner order: measured 6–62 GB per worktree on a storage-critical box; the fleet builds against ONE shared `target/` (AGENTS.md fleet rule 4).
- **[2026-10-17]** Resurrecting the 7 dead TUI dialogs deleted in iter-58 (elicitation, onboarding, file_injection, invalid_config, memory_update_notification, overage_upsell, desktop_upsell_startup) — **closed**: unreachable code, deleted ~2900 LOC; the surviving 7 dialogs are alive via indirect `open()` calls.
- **[2026-10-17]** Parallel memory providers (TDG/Hindsight/RetainDb/Mem0/LocalVector/agentmemory) — **rejected**: removed; unknown provider names silently downgrade to builtin (AGENTS.md "Memory").
- **[2026-10-17]** Re-homing AGENTS.md's routing table out of AGENTS.md — **rejected**: the entry contract must stay the one file every agent reads first; the genome's routing table is a section OF AGENTS.md, mapped through `kb.toml`.
- **[2026-10-19]** Body runs that write a `sessions` row (AD-001 option a) — **rejected**: the session browser would list employee runs beside the operator's own chats, and the body home has no `database.db`. Kept: trajectory attribution (AD-001 b).
- **[2026-10-19]** `/session` shelling out to `operant org holon <x> --json` for org reads (AD-002 a) — **rejected**: `/org` suspends the alternate screen on every use, and a panel that refreshes cannot leave it. Kept: in-process `HolonApi` (AD-002 b).
- **[2026-10-19]** The Holon home as the TUI launch home (AD-003 b) — **rejected**: it puts the operator's seat into the body stores that the driver writes under `HERMES_HOME`. Kept: the default operant home (AD-003 a).
- **[2026-10-19]** A local employee registry for `/session` validation (AD-004 b), and keeping any-id binding in org mode (AD-004 d) — **rejected**: no such registry exists and Holon owns the roster; any-id binding is the personal behaviour AD-004 refuses in org mode.
- **[2026-10-19]** `premiere` as the default employee for new org-mode chats (G-1 option a) — **rejected**: `premiere` is not on the Holon roster (AD-004). Options (b) and (c) are in `## Review` of TODO.md.
