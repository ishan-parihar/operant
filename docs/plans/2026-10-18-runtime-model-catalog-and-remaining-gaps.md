# runtime-model-catalog-and-remaining-gaps

Dated-frozen: published `2026-10-18`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Supersedes the outline in the chat of 2026-10-18 and the 2026-10-17 plans
(`plan-2026-10-17-remaining-implementation-gaps.md`,
`plan-2026-10-17-redteam-audit-of-remaining-gaps.md`), which reference iter-903 and
list items that have since closed.

## Mechanism

### Decision (owner, 2026-10-18)

The model catalog is **runtime-only**. No static model list is compiled into the
binary and no generated snapshot is checked in. Every model list comes from one of:

1. the provider's own `/models` endpoint, when reachable and authorized;
2. the models.dev catalog, read from the on-disk cache and refreshed in the background;
3. the configured models (`[agent] model` + `fallback_models`), when neither answers.

Case 3 is shown with an explicit notice (already in place, iter-916). It is not padded
with a bundled list. This follows the runtime half of oh-my-pi's
`createModelManager` (live discovery, then cache, then fallback) and jcode's
`pricing.rs` precedence (static, then models.dev disk cache). Neither reference
ships a catalog that the code depends on at startup; oh-my-pi's 11.5 MB `models.json`
is a generated artifact we are not adopting.

### Corrections to the state this plan starts from

- **The iter-917 context-window path does not match real models.dev data.** The
  parser in `crates/operant-core/src/models_dev.rs` reads flat keys
  (`provider_id`, `context_window`, `max_output`, `tool_call`). models.dev's schema is
  nested: the window is `limit.context`, output is `limit.output`, and modalities are
  `modalities.input`. Verified against the models.dev-shaped cache on disk
  (`~/.hermes/models_dev_cache.json`): `openai/…` entries carry `limit` and no
  `context_window`. So `cached_capabilities` returns `None` for real rows and the
  context meter always takes the per-provider fallback. iter-917 is inert until P0
  lands. Its CHANGELOG entry overstates it.
- **No operant-written cache exists on this machine.** `~/.operant/models_dev_cache.json`
  is absent, so `load_models_dev` has never completed a successful write here (or
  the fetch fails silently; see P0.1).
- **The models.dev endpoint is unverified.** Operant fetches
  `https://models.dev/api/v1/models` and expects a `{models, providers}` body. The
  on-disk cache came from the per-provider shape. Both probes timed out from this
  machine on 2026-10-18 (TLS handshake), so the live shape is not yet confirmed.
- **Setup and the picker use two separate fetchers.** `fetch_models_for_provider`
  (`crates/operant-cli/src/provider.rs:1166`) serves setup; `fetch_endpoint_models`
  (`crates/operant-cli/src/tui/app/providers.rs:9`) serves `/model`. They disagree on
  auth and on which providers they cover.
- **The gateway does not pick up `/connect` retargets.** `pending_endpoint` appears
  only in TUI files (`tui/app/*`, `tui_app.rs`). A running gateway (Telegram, WebSocket)
  keeps the endpoint it was built with until restart. This confirms prior-outline item B4.
- **Provider count is 42, not 43.** `PROVIDERS` has 42 `ProviderDef` entries; the
  43rd grep hit was the struct definition.
- **Closed since the 10-17 plans:** H-3, H-5, H-7, H-8, H-9 (main part), W3-2, K-3,
  K-4, D-4 (closed in iter-916 docs; the cron agent passes `false` at
  `gateway_runner.rs:2230`), B2.6 (session picker deleted in iter-913), B5 (iter-916),
  B7 (iter-916), §0 blocker (iter-901), plan 07 steps 4–5 (iters 901/903).

### Live fetch coverage today

`fetch_models_for_provider` dispatches on the provider name. Of 42 catalog entries:

| Path | Providers | Count |
|---|---|---|
| OpenAI-compatible `GET {base}/models`, bearer | openai, xai, mistral, groq, deepseek, together, openrouter, nvidia, perplexity, cohere, huggingface, zai, kimi-coding, stepfun, minimax, alibaba, xiaomi, opencode-zen, opencode-go, kilocode, ollama-cloud, vercel, moonshot, tencent-tokenhub | 24 |
| Anthropic `GET /v1/models`, `x-api-key` | anthropic | 1 |
| Google `GET /v1beta/models`, strips `models/` | google | 1 |
| Static only (no live fetch) | alibaba-coding-plan, arcee, azure, azure-foundry, bedrock, copilot, copilot-acp, gmi, google-gemini-cli, kimi-coding-cn, lmstudio, minimax-cn, nous, ollama, openai-codex, qwen-oauth | 16 |

26 live + 16 static-only = 42. The static-only set includes two local servers
(`ollama`, `lmstudio`) that expose a models list and need no key.

The picker path (`fetch_endpoint_models`) sends bearer auth only. It cannot reach
Anthropic (needs `x-api-key`) or Google (needs the `key` query or header). Those
providers therefore show only the configured rows in `/model`.

### Target architecture

One module in `operant-core`: `model_catalog`.

- `fetch_provider_models(spec, auth, base_url) -> Result<Vec<CatalogModel>, FetchError>`
  where `FetchError` carries the reason string the status bar shows. Adapters per wire
  family: `OpenAiCompat`, `Anthropic`, `Google`, `OllamaNative` (`/api/tags`), `Azure`
  (deployments with `api-version`), `Copilot` (token exchange), `OAuthOnly` (models.dev
  only).
- `CatalogModel { id, display_name, context_window, max_output, supports_tools,
  supports_reasoning, cost_in, cost_out, source: Live | ModelsDev | Configured }`.
- Source order per provider: live, then models.dev cache (TTL 1 h, background refresh),
  then configured. Each row carries its source so the picker can show it.
- `ProviderDef` keeps identity and transport only: `name`, `display_name`,
  `default_base_url`, `env_vars`, `signup_url`, `auth_type`, `models_dev_id`. The
  `models: &[…]` arrays and `default_model` are removed.
- Consumers switched to `model_catalog`: `cmd_setup.rs` (step 2), the `/model` picker,
  `model_registry.rs`, `free_catalog.rs` (its `default_model` fields), `insights.rs`
  cost table, `compress.rs` cost path (already models.dev).
- `provider_to_models_dev` (14 hand-written entries) is replaced by `models_dev_id` on
  `ProviderDef`, with a test that every catalog entry resolves or is marked
  `models_dev_id: None` on purpose.

### What stays, and why

- `reasoning_timeouts.rs` `REASONING_FLOORS`: behavior policy (stream stall floors per
  model family), not catalog data. Keep, and say so in its header.
- `provider_aliases.rs` `is_*_alias`: routing logic. Keep.
- Provider endpoints, env vars, signup URLs in `ProviderDef`: transport metadata, not
  model data. Keep.

### Default-model policy (decision needed)

Removing `default_model` means setup needs a rule for the pre-selected model. Proposal:
keep the configured model if one exists; otherwise the first live row whose
`supports_tools` is true, falling back to the first row. Owner to confirm.

## Gates / success criteria

Fixed before the run. Each phase is one iteration (one commit, pushed, deployed when it
touches code), per AGENTS.md.

- **P0 gate:** `fetch` against the live models.dev endpoint returns a non-empty list,
  verified by a recorded fixture test. `cached_capabilities("anthropic", "claude-haiku-4-5")`
  returns `context_window == Some(200000)` from a real cache file. `~/.operant/models_dev_cache.json`
  exists after one TUI start with network.
- **P1 gate:** one fetcher. `grep -rn "fetch_models_for_provider\|fetch_endpoint_models"`
  finds no definitions, only callers of `model_catalog`. Each of the 42 providers has a
  unit test that its adapter builds the right URL and auth header (no network).
  `/model` shows live rows for Anthropic and Google with a configured key.
- **P2 gate:** `grep -rn "models: &\["` in `crates/operant-cli/src/provider.rs` returns
  nothing. Setup offline shows configured models plus the notice; setup online shows
  live rows.
- **P3 gate:** one cost source. `insights.rs` has no literal pricing table.
- **P4 gate:** gateway reloads endpoint on `/connect` (or documented restart requirement
  with a status notice) and a test covers it.
- Every phase: `cargo fmt --all`, `cargo clippy -p <crate> --all-targets -- -D warnings`,
  scoped `cargo test`, `./scripts/genome lint` (0 errors), clean-worktree release build
  with the provenance gate, md5 match against `/proc/<gateway>/exe`.

## Work breakdown

### P0 — correctness of what already shipped (first)

- **P0.1** Confirm the models.dev endpoint and shape live. Record a fixture under
  `crates/operant-core/tests/fixtures/`. Make `fetch_from_network_uncached` log the
  HTTP status and body length on failure, not silently return `Err` into a `let _`.
- **P0.2** Rewrite `match_capabilities` and `get_model_capabilities` to read the nested
  schema (`limit.context`, `limit.output`, `tool_call`, `modalities.input`,
  `cost.input` / `cost.output` per million). Add the fixture test.
- **P0.3** Correct the iter-917 CHANGELOG entry to say the schema fix landed in P0.2.
- **P0.4** Gateway retarget: call `set_endpoint` on the gateway agent when the config's
  `client.base_url` changes, or print a restart-required notice. (B4)

  **Status (iter-924):** done by reading `operant.toml` per message (`endpoint_changed_on_disk`),
  not by an in-process signal: the TUI and the gateway are separate processes.

### P1 — one live catalog fetcher

- **P1.1** Create `operant-core/src/model_catalog.rs` with the adapter enum and
  `CatalogModel`.
- **P1.2** Port the 24 OpenAI-compatible, Anthropic, and Google paths from
  `provider.rs`. Add the Ollama native path (`/api/tags`) for `ollama`, and
  OpenAI-compatible `/v1/models` for `lmstudio`.
- **P1.3** Switch the TUI picker to `model_catalog` and fix its auth so Anthropic and
  Google work (`x-api-key`, `key` query). Delete `fetch_endpoint_models`.
- **P1.4** Switch setup step 2 to `model_catalog`. Delete `fetch_models_for_provider`.
- **P1.5** Add the "could not list models" notice to setup, matching `/model`.

**Status (iter-921):** P1.1, P1.3, P1.4, P1.5 done; the legacy `ProviderProfile` layer
(second fetcher) deleted with its tests. P1.2 partly: Ollama uses `/v1/models`, not
`/api/tags` (the OpenAI-compatible route; native path dropped). Not done: a mock test per
one of the 42 providers (only the three wire families are mocked), and live verification
of Anthropic and Google listing (no key here).

**Status (iter-926):** the 42-provider gap is closed at the mapping level. `wire_family`
now covers the eight OpenAI-compatible providers it missed, and the seven others are named
in `NO_LISTING_ENDPOINT`. `every_provider_has_a_listing_request_or_is_named_as_having_none`
checks every entry. Still open: mock HTTP tests for the non-core families beyond the three
mocked, and live Anthropic and Google verification (no key here).

### P2 — delete the static catalog

**Status (iter-922):** P2.1–P2.4 done. P2.5 done in iter-923. `provider_metadata`'s static
`default_model` table removed in iter-925.

- **P2.1** Remove `models` and `default_model` from `ProviderDef`. Update the 42 entries.
- **P2.2** Make `model_registry.rs` a thin view over `model_catalog`. Remove
  `populate_default_models` and `ensure_provider_defaults`.
- **P2.3** Free-mode catalog: take `default_model` from the live list.
- **P2.4** Implement the default-model policy (decision above).
- **P2.5** Replace `provider_to_models_dev` with `models_dev_id` and its coverage test.

### P3 — one cost source

- **P3.1** `insights.rs` `estimate_session_cost` reads `CatalogModel.cost_*`. Delete the
  literal table.
- **P3.2** `compress.rs` cost path shares the same reader.

### P4 — remaining agent and model correctness

- **P4.1** D-4 regression test: none planned. The cron agent passes `false`; the spawn is
  side-effecting with no seam to count schedulers. Revisit only if a second caller
  appears.
- **P4.2** Batch-06 P1, `BustPermission` / `ContextExhausted` at 95%. Not started. The
  largest unstarted correctness item. Needs its own plan before code. Source:
  `docs/audit/2026-10-16-operant-audit/11-batch-redirection.md` (P1 row).
- **P4.3** Batch-06 P3/P4 (historian, `ctx_reduce`), personal mode. Still in TODO.md
  Review.
- **P4.4** Anthropic live turn (B6). Blocked until a key is available; the P1 picker
  fix can be checked without a turn.

### P5 — live-only (needs your terminals)

- **P5.1** Plan 07 step 3: dynamic live-region height. Start with a ratatui 0.30
  inline-resize spike.
- **P5.2** Plan 07 step 6: confirm mouse capture is off in native mode; document
  Shift+PgUp and tmux copy-mode.
- **P5.3** Plan 07 step 7: four-terminal sweep (tmux, kitty, ghostty, alacritty, one
  non-kitty), then decide `terminal_scroll_mode` (default `false`, `config.rs:905`).
- **P5.4** S1/S2: OSC-8 post-draw writes and wide-glyph widths. Pair with P5.3.

### P6 — owner decisions and hygiene

- **P6.1** `device_auth_dialog` scaffold (H-9 residual). Remove it (re-prove the corpus,
  since it changes the overlay-flag list) or finish the github-copilot and openai-codex
  flows.
- **P6.2** TODO.md Pending: rebuild `/settings` over `operant.toml`; `[keybindings]`
  user overrides; unify gateway `show_reasoning` with the TUI.
- **P6.3** Dead config: R13-3 (`run_gateway` has no callers), R13-4
  (`[memory] audit_enabled` never applied), R13-5 (response cache is dead config). Each
  needs wire-or-delete.
- **P6.4** R40-25: 14 of 23 channel features have no dispatch arm. Prune from default
  builds or wire.
- **P6.5** Corpus drift: `command-palette`, `connect-dialog`, `context-viz`,
  `typeahead-suggestions`, parked `device-auth-dialog`. Re-prove after any overlay change.
- **P6.6** BUGS.md hygiene: R15-1 is stale (the `operant-channels` crate is gone). R42-12
  is historical. R42-8 was declined by the owner; leave as declined.
- **P6.7** Release path: `/usr/local/bin/operant` is still the pre-iter-915 build.
  Needs sudo from the owner:
  `! sudo cp ~/.local/bin/operant /usr/local/bin/operant.new && sudo mv /usr/local/bin/operant.new /usr/local/bin/operant`.
- **P6.8** Hidden dead code behind `expect(dead_code)` blankets in `operant_app`,
  `operant_model`, `operant_render`, `operant_ui`.
- **P6.9** Owner's `~/.operant/.env` still pins the dead `:20129` endpoint. Re-run
  `/connect` once.
- **P6.10** Duplicate label: `fdfd4f29` and `c676b988` are both iter-918. Recorded per the
  AGENTS dup-label rule; the next commit carries `(dup-label iter-918)`. History is not
  rewritten.
- **P6.11** Flaky test: `tui::render::tests::accent_role_reaches_the_frame` fails in
  the full binary run and passes alone. It reads the global palette that
  `switching_theme_repaints_the_frame` mutates in parallel. Serialize or isolate the
  palette state.

  **Status (iter-927):** the two locks are one (`ACTIVE_LOCK` aliases `STYLE_TEST_LOCK`).
  Not reproduced before or after. Still open: tests that call `App::new` without the lock
  also write the theme (`scroll_anchor`, `messaging`, `scroll_emit`, `redraw_reason`,
  `ui_header`, `permissions`, `adapter`, `debug`).

## Cost

- **P0–P3 (runtime catalog):** the critical path. About one iteration per phase. Most of
  the work is the adapter table and tests. No new dependencies: `reqwest` and `serde_json`
  are already in use. No generated artifacts.
- **Network cost at runtime:** one models.dev fetch per hour when online (TTL), plus one
  `/models` call per provider when the picker or setup opens. The picker fetch is
  bounded to 10 s.
- **Not measured:** how many of the 24 OpenAI-compatible endpoints return a usable
  `/models` list without a key; some return 401 or a non-`data` shape. Each unverified
  case falls back to models.dev, which is why P0.1 comes first.
- **Risk:** removing static lists is user-visible for offline setup. Mitigated by the
  configured-models row and the notice, not by a bundled snapshot. This is the owner's
  chosen trade-off.
