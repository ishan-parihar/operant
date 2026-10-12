# Operant Security & Privacy

- **Secrets**: API keys live in `~/.operant/.env` only (never operant.toml). The core loop redacts secrets at the LLM boundary (`redaction.rs` — token prefixes, env assignments, auth headers, private keys, JWTs, DB connstrings, URL credentials). `HERMES_REDACT_SECRETS` toggles.
- **Approval**: tool permission prompts; `--dangerously-skip-permissions` shows one confirmation then skips the rest.
- **Skills guard**: `skills_guard` scans skill content on install (unpinned pip, destructive shell patterns) with a trust matrix.
- **Local-first**: no telemetry, no account required.
- **PII**: user/chat IDs are hash-prefixed in gateway logs.
- **Checkpoints**: opt-in, stored in an isolated shadow store — user git repos are never modified.
- **Secrets in `.env`, settings in `operant.toml`** — never put a credential in the TOML.
