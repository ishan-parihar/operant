# Worklog

Append-only: `kb log "<entry>"` — one entry per unit of work.
- **2026-10-17T13:13** · `omp-docs` — adopted the project-kb genome (iter-901): CLI copied to scripts/kb, kb.toml maps roles to existing paths, docs/plan-* + docs/audits/ re-homed into docs/plans//docs/audit/, plans/ series + docs/BUGS.md archived with tombstones, routing table + rules appended to AGENTS.md, STATE/entry/RUNBOOK/CLOSED seeded
- **2026-10-17T19:06** · `308` — iter-903 label duplicated on origin/main: 2b6bda01 (plan 07 step 4, peer) and 179b55bf (docs: 2026-10-16 audit integration, agent 308). No history rewrite per fleet rules — cosmetic defect; iteration numbering continues at iter-904.
- **2026-10-18T00:09** · `claude` — iter-904: audit correctness queue — H-5 (distillation ate scripted reply), H-7 (loop-warning annotation misread as malformed JSON), models.dev fetch backoff, /fast removed (H-8), 7 flaky relative-path test DBs moved to temp; BUGS/TODO/CHANGELOG updated
- **2026-10-18T00:29** · `claude` — iter-905: H-9 connect dead ends removed + MCP panel auth wired to real OAuth; plan 07 step 6 docs (README/example toml); K-4 verified closed
- **2026-10-18T00:31** · `claude` — iter-906: W3-2 /stop cancels queued gateway turns (race-free via running_session mutex); K-2 not reproducing; D-6 accepted
- **2026-10-18T01:45** — iter-908: retired stale org_reason tests (org layer deleted iter-838), fixed org --help banner, closed BUGS H-2/S6, archived the 10-13 QoL plan (kb lint clean)
- **2026-10-18T02:10** — iter-909: clippy gate green for operant-core + operant-os (duplicated expect attrs removed, poison-recovering locks, dead code deleted, test-crate headers); operant-cli remains
- **2026-10-18T02:48** — iter-910: clippy gate green workspace-wide; operant-cli dead-code purge (~120 items, 19 tests); device-auth scenario parked; BUGS R-LATEX/session-picker findings
- **2026-10-18T03:03** — iter-911: cache-stable layout (turn_context on latest user msg, ladder insight after system prefix) + Anthropic multi-system-message fix; batch-06 P0 landed (telemetry/P1+ remain)
- **2026-10-18T03:08** — iter-912: delete superseded latex/mermaid raster pipeline (no producer on any prod path); R-LATEX closed as superseded
- **2026-10-18T03:15** — iter-913: delete dead session-picker port + 6 orphaned operant_app modules (~12.8k lines); B2.6 closed
- **2026-10-18T03:28** — iter-914: memory tool descriptions corrected to memory-wire; AGENTS.md cache-layout note; release 913 deployed (md5 f37fdbc3)
- **2026-10-18T04:48** — iter-915: /connect retargets the live agent client (was: display-only, requests kept hitting client.base_url); setup labels loopback proxies Custom not Ollama; catalog anthropic/google/cohere/minimax URLs are OpenAI-compatible roots; dialog paste/Ctrl+V reach the open dialog; keyless local endpoints pass the credential pool. Verified in TUI sandbox: shell tool round trip via the connected endpoint; setup persists endpoint/model/key with matching .env pins. Tests: operant-core lib 2274, operant-cli bin 1601, clippy gate green. CHANGELOG Fixed entry added.
- **2026-10-18T12:28** · `claude` — runtime-only model catalog plan; iter-917 schema gap found
- **2026-10-18T12:57** — iter-921 — one live model fetcher (plan P1): picker and setup list through model_catalog::resolve; Anthropic x-api-key and Google key-query wire paths; legacy ProviderProfile layer and second fetcher deleted
