# Worklog

Append-only: `kb log "<entry>"` — one entry per unit of work.
- **2026-10-17T13:13** · `omp-docs` — adopted the project-kb genome (iter-901): CLI copied to scripts/kb, kb.toml maps roles to existing paths, docs/plan-* + docs/audits/ re-homed into docs/plans//docs/audit/, plans/ series + docs/BUGS.md archived with tombstones, routing table + rules appended to AGENTS.md, STATE/entry/RUNBOOK/CLOSED seeded
- **2026-10-17T19:06** · `308` — iter-903 label duplicated on origin/main: 2b6bda01 (plan 07 step 4, peer) and 179b55bf (docs: 2026-10-16 audit integration, agent 308). No history rewrite per fleet rules — cosmetic defect; iteration numbering continues at iter-904.
- **2026-10-18T00:09** · `claude` — iter-904: audit correctness queue — H-5 (distillation ate scripted reply), H-7 (loop-warning annotation misread as malformed JSON), models.dev fetch backoff, /fast removed (H-8), 7 flaky relative-path test DBs moved to temp; BUGS/TODO/CHANGELOG updated
- **2026-10-18T00:29** · `claude` — iter-905: H-9 connect dead ends removed + MCP panel auth wired to real OAuth; plan 07 step 6 docs (README/example toml); K-4 verified closed
- **2026-10-18T00:31** · `claude` — iter-906: W3-2 /stop cancels queued gateway turns (race-free via running_session mutex); K-2 not reproducing; D-6 accepted
- **2026-10-18T01:45** — iter-908: retired stale org_reason tests (org layer deleted iter-838), fixed org --help banner, closed BUGS H-2/S6, archived the 10-13 QoL plan (kb lint clean)
