#!/usr/bin/env python3
"""Report lint suppressions (`#[allow(...)]`, `#[expect(...)]`) in production code.

rust-best-practices ch.2 ("Fix warnings, don't silence them") is explicit:
prefer `#[expect]` over `#[allow]`, because an `allow` never expires while an
`expect` reports itself as unfulfilled the moment the lint stops firing.

This script makes that measurable instead of a prose claim, because prose
numbers rot: `docs/RUST_BEST_PRACTICES_PLAN.md:25` still reports 242 suppressed
warnings from a 2026-08-02 measurement. The live figure after the config
retirement, the provider-surface retirement and the three-crate purge is 33.

KEYING (why not line numbers). A baseline keyed on `path:line` breaks the first
time anything above it moves, and a gate that breaks when the code it guards is
edited is a gate nobody keeps. Each suppression is therefore identified by
`<relative-path>::<lint>::<item>` — the attribute's lint plus the identifier of
the item it decorates (the first non-comment token on a following line). That
is stable across reordering and reindentation, and ambiguous only when two
suppressed items in one file share a name, which is itself worth reporting.

WHAT IT CHECKS:

  * `--check` exits non-zero when a suppression is NOT in the baseline. That is
    the gate: an unexplained suppression is a decision nobody made. (An
    always-red check carries no information — the exact defect
    `audit-config-surface.py`'s `--check` had before iter-1153.)
  * A baseline entry with no reason is reported. A suppression with no reason
    is the kind that silently outlives its subject.
  * `#[allow]` sites where `#[expect]` would work are listed as CONVERSION
    CANDIDATES: outstanding work, not a failure.

Usage: python3 scripts/audit-lint-suppressions.py [--check]
Exit: 0 (report only) unless --check is given and an unexplained or
unreasoned suppression exists.
"""

import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent / "crates"

TRACKED = {
    "dead_code",
    "unused_variables",
    "unused_mut",
    "unused_assignments",
}

# Matches BOTH spellings of a suppression. The bare form is
#     #[allow(dead_code)]                       / #[expect(dead_code, reason = "..")]
# and the cfg-conditional form puts the lint INSIDE cfg_attr's parens:
#     #[cfg_attr(test, allow(dead_code, reason = ".."))]
# The first attempt wrote the cfg group as an optional PREFIX
# (`(?:cfg_attr\([^()]*\)\s*,\s*)?allow...`) and matched NOTHING of the
# cfg_attr form — in cfg_attr the `allow` sits inside the outer parens, not
# after them, so the pattern could never align. Measured: that regex returned
# `False` for all three shapes above; this one returns a match for each, with
# `lints` holding the inner group so the `reason = ...` key is split out rather
# than read as a lint name.
RE_ATTR = re.compile(
    r"#\[\s*(?:cfg_attr\s*\(\s*(?P<cfg>[^()]*?)\s*,\s*)?"
    r"(?P<kind>allow|expect)\s*\(\s*(?P<lints>[^()]*?)\s*\)"
)

# ---------------------------------------------------------------------------
# THE BASELINE. Every suppression that is known, with the reason it is still
# there. Key: "<relative-path>::<lint>::<item>". Value: (expectable, reason),
# where `expectable` is "yes" (an `allow` that could be an `expect,` i.e.
# outstanding work) or "no" (`expect` is the WRONG choice here — a macro
# expansion, a cfg-conditional body, or a feature-conflicting type).
# ---------------------------------------------------------------------------
BASELINE = {
    # ---- #[expect] sites that are already correct -------------------------
    # Listed so a regression to `#[allow]` is visible as an UNEXPLAINED entry
    # rather than silently matching an old allow-shaped baseline.
    "operant-core/src/schema.rs::dead_code::TestParams": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"test-only schema struct\")]`",
    ),
    "operant-core/src/schema.rs::dead_code::CoercionParams": (
        "n/a",
        "already an `#[expect(dead_code, reason = ...)]`",
    ),
    "operant-core/src/schema.rs::dead_code::NumberParams": (
        "n/a",
        "already an `#[expect(dead_code, reason = ...)]`",
    ),
    "operant-core/src/tools.rs::dead_code::TestArgs": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"test-only argument struct\")]`",
    ),
    "operant-core/src/mcp_oauth.rs::dead_code::is_interactive": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"reserved for TTY-only OAuth flows\")]`",
    ),
    "operant-core/src/agent/clients/prompt_caching.rs::dead_code::as_str": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"reserved for TTL introspection/debugging\")]`",
    ),
    "operant-core/tests/seat_policy_run_path.rs::dead_code::prompt": (
        "n/a",
        "already an `#[expect(dead_code, reason = \"schema-only: the stub never reads the prompt\")]`",
    ),
    # ---- conversions landed in iter-1163 -----------------------------------
    "operant-core/src/tools/skills_tool.rs::dead_code::USAGE_TELEMETRY_LOCK": (
        "n/a",
        "converted to `#[expect]` in iter-1163: this doc comment claimed an "
        "`#[expect]` while the attribute was an `#[allow]`, so the suppression "
        "never expired when the legacy `.usage.json` writer it covers came back",
    ),
    "operant-cli/src/config.rs::dead_code::Parse": (
        "n/a",
        "converted to `#[expect]` in iter-1163 — a public error variant with a "
        "Display arm and no constructor",
    ),
    "operant-plugins/src/host.rs::dead_code::verification": (
        "n/a",
        "converted to `#[expect]` in iter-1163 — a SECURITY VERDICT written and "
        "never read; the escape hatch expires when it is finally surfaced",
    ),
    "operant-providers/src/bedrock.rs::dead_code::role": (
        "yes",
        "the response field is an #[expect(dead_code)] carrying a role for "
        "payload parity with the wire shape",
    ),
    # ---- #[allow(dead_code)] that must stay ---------------------------------
    "operant-providers/src/gemini.rs::dead_code::InternalGenerateContentResponse": (
        "no",
        "feature-conflicting: used by the default-feature response path and "
        "unused under `--all-features`, so `#[expect]` is unfulfilled under "
        "one of the two feature sets. Its own comment says so and it is true.",
    ),
    "operant-core/src/agent/background_review.rs::dead_code::summarize_review_actions": (
        "no",
        "`#[cfg_attr(not(test), ...)]`: exercised only by unit tests today, so "
        "the lint fires only OUTSIDE `cfg(test)` and `expect` would be "
        "unfulfilled in exactly the build that reads it",
    ),
    "operant-cli/src/tui/operant_app/todo.rs::dead_code::as_str": (
        "no",
        "`semantic_state!` generates ONE impl for eleven enums whose consumers "
        "differ — `ConfidenceState` aggregates numerically through "
        "`legacy_score`. Verified by experiment (iter-1163): `#[expect]` "
        "produces two 'lint expectation is unfulfilled' warnings, because a "
        "macro expansion cannot express per-expansion expectations.",
    ),
    "operant-cli/src/tui/operant_app/todo.rs::dead_code::legacy_score": (
        "no",
        "the `legacy_score` arm of the same `semantic_state!` expansion",
    ),
    # ---- #[allow] sites that could be #[expect] -----------------------------
    "operant-core/src/tools/kernel/sidecar.rs::dead_code::bridge_rx": (
        "yes",
        "`SidecarHandle::bridge_rx` — bridged tool-call requests awaiting host "
        "execution; the drain loop is the phase 2.5 wiring. A concrete field, "
        "so `#[expect]` is available.",
    ),
    "operant-providers/src/bedrock.rs::dead_code::Other": (
        "yes",
        "an enum variant carrying a parsed payload for a response shape only "
        "some calls produce",
    ),
    "operant-harness/src/row.rs::dead_code::config": (
        "yes",
        "Phase-3 stub carriage — the host is meant to read `config`",
    ),
    "operant-harness/src/row.rs::dead_code::config (2)": (
        "yes",
        "the SECOND `config` field in row.rs (line 75) — a Phase-3 stub field "
        "the host is meant to read. Two sites in one file share the name; the "
        "item-only key cannot tell them apart, which is the ambiguity the "
        "script's docstring says is itself worth reporting.",
    ),
    "operant-harness/src/row.rs::dead_code::FakeFilePromptSection": (
        "yes",
        "Phase-3 stub struct (third site in row.rs)",
    ),
    "operant-harness/src/pool_provider.rs::dead_code::PoolBundlePayload": (
        "yes",
        "Phase-3 stub payload struct",
    ),
    "operant-core/examples/simple_agent.rs::dead_code::EchoArgs": (
        "yes",
        "example code — an argument struct set up for a later step of the "
        "walkthrough",
    ),
    "operant-core/examples/simple_agent.rs::dead_code::CalcArgs": (
        "yes",
        "example code (second struct in the same file)",
    ),
    "operant-harness/tests/host_boot.rs::dead_code::_ensure_noop_linked": (
        "yes",
        "test helper kept for a later test (leading underscore already says "
        "intentional)",
    ),
    "operant-harness/tests/builder_factories.rs::dead_code::_host_smoke": (
        "yes",
        "test helper kept for a later test",
    ),
    # ---- #[allow(unused_*)] ------------------------------------------------
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::centered": (
        "yes",
        "consumed by the gated inline-image block until W6 (3 sites: `centered`, "
        "`diagram_mode`, `pinned_diagrams`)",
    ),
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::diagram_mode": (
        "yes",
        "the same W6 inline-image block's second local",
    ),
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::pinned_diagrams": (
        "yes",
        "the same W6 inline-image block's third local",
    ),
    "operant-config/src/policy.rs::unused_mut::mut": (
        "yes",
        "the command vec is mutated only on some target OSes",
    ),
    "operant-core/src/agent/run.rs::unused_assignments::mut": (
        "yes",
        "`tool_names` is collected before the match and read after it, which "
        "the compiler cannot see across the match",
    ),
    "operant-core/src/mcp_oauth.rs::unused_variables::handle_401": (
        "yes",
        "the 401-recovery path keeps the parameter for the poisoning handler's "
        "signature",
    ),
    # ------------------------------------------------------------------
    # iter-1168. The first version of this gate's regex matched only the bare
    # `#[allow(...)]`/`#[expect(...)]` spelling, so every `#[cfg_attr(not(test),
    # allow(...))]` and every rustfmt-split multi-line attribute was invisible
    # to it — 50 of the repo's suppressions, including ones carrying their own
    # `reason = "..."`. A gate that cannot see a suppression cannot govern it,
    # which is the same defect class RG-013 records for the doc gate. The regex
    # now matches both spellings and the scan joins a rustfmt-split attribute
    # before matching; these entries are what the widened scan found, each
    # carrying the reason the site's own text states where it states one.
    # ------------------------------------------------------------------
    "operant-cli/src/commands.rs::dead_code::TuiSlashCommand": ("yes", "description/aliases are table data rendered only by the legacy help path"),
    "operant-cli/src/config.rs::dead_code::Parse": ("no", "iter-1163: a Construct error variant nothing reads yet; expect because clippy's dead_code fires on every build"),
    "operant-cli/src/tui/adapter_types/types.rs::dead_code::ToolResultContent": ("yes", "AD-034: adapter wire enum; variants are parsed from adapter JSON, not built here"),
    "operant-cli/src/tui/keybindings.rs::dead_code::Redo": ("yes", "AD-034: kept so a future redo has a variant to bind; the catalogue guard test pins that no binding uses it"),
    "operant-cli/src/tui/operant_app/app.rs::dead_code::ProcessingStatus": ("yes", "jcode-parity port: no live transport emits a connection phase"),
    "operant-cli/src/tui/operant_app/config_shim.rs::dead_code::KeybindingsConfig": ("yes", "jcode-parity port: shape-compat shim field with no live reader"),
    "operant-cli/src/tui/operant_app/config_shim.rs::dead_code::keybindings": ("yes", "jcode-parity port: shape-compat aggregate field, read only by its own tests"),
    "operant-cli/src/tui/operant_app/config_shim.rs::dead_code::SwarmStripLayout": ("yes", "jcode-parity port: config-named alternative layout, never selected live"),
    "operant-cli/src/tui/operant_app/config_shim.rs::dead_code::swarm_strip_layout": ("yes", "jcode-parity port: shape-compat field; the live strip never reads it in this profile"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::content": ("yes", "jcode-parity port: the panel reads `section`, not the payload"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::detail": ("yes", "cfg_attr(test): test-profile only, so the lint does NOT fire every build and expect would be unfulfilled in production"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::InjectedMemoryItem": ("yes", "jcode-parity port: leaf type, read only by its own tests"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::latency_ms": ("yes", "jcode-parity port: no live step reports its latency"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::MemoryEventKind": ("yes", "jcode-parity port: four variants never emitted today"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::MemoryState": ("yes", "jcode-parity port: never constructed today"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::StepResult": ("yes", "jcode-parity port: never constructed today"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::StepStatus": ("yes", "jcode-parity port: never constructed today"),
    "operant-cli/src/tui/operant_app/memory.rs::dead_code::timestamp": ("yes", "cfg_attr(test): test-profile only, so the lint does NOT fire every build and expect would be unfulfilled in production"),
    "operant-cli/src/tui/operant_app/todo.rs::dead_code::as_str": ("no", "macro-generated uniform schema for eleven enums; ConfidenceState consumes both, so expect is unfulfilled on 2 of 11 expansions (measured iter-1160)"),
    "operant-cli/src/tui/operant_app/todo.rs::dead_code::legacy_score": ("no", "same macro expansion as as_str"),
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::centered": ("yes", "W6 inline-image block local; allow where expect would work (the lint fires every build)"),
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::diagram_mode": ("yes", "W6 inline-image block local; allow where expect would work"),
    "operant-cli/src/tui/operant_ui/ui_viewport.rs::unused_variables::pinned_diagrams": ("yes", "W6 inline-image block local; allow where expect would work"),
    "operant-config/src/policy.rs::unused_mut::mut": ("yes", "unused_mut that a future policy arm needs; allow where expect would work"),
    "operant-core/examples/simple_agent.rs::dead_code::CalcArgs": ("yes", "example serde args struct; allow where expect would work"),
    "operant-core/examples/simple_agent.rs::dead_code::EchoArgs": ("yes", "example serde args struct; allow where expect would work"),
    "operant-core/src/agent/clients/prompt_caching.rs::dead_code::as_str": ("yes", "reserved for TTL introspection/debugging"),
    "operant-core/src/agent/run.rs::unused_assignments::mut": ("yes", "unused_assignments on a diagnostic accumulator; allow where expect would work"),
    "operant-core/src/agent/turn_finalizer.rs::dead_code::explanation": ("yes", "Prepared for UI integration; will surface turn-exit reasons"),
    "operant-core/src/mcp_oauth.rs::dead_code::is_interactive": ("yes", "reserved for TTY-only OAuth flows"),
    "operant-core/src/mcp_oauth.rs::unused_variables::handle_401": ("yes", "reserved for the 401-retry path; allow where expect would work"),
    "operant-core/src/mcp.rs::dead_code::data": ("yes", "Deserialized from MCP server; may contain debug info"),
    "operant-core/src/mcp.rs::dead_code::jsonrpc": ("yes", "Deserialized from MCP server; protocol field kept for completeness"),
    "operant-core/src/mcp.rs::dead_code::list_changed": ("yes", "Deserialized from MCP server response; kept for protocol completeness"),
    "operant-core/src/mcp.rs::dead_code::list_changed": ("yes", "Deserialized from MCP server response; kept for protocol completeness"),
    "operant-core/src/mcp.rs::dead_code::subscribe": ("yes", "Deserialized from MCP server response; kept for protocol completeness"),
    "operant-core/src/schema.rs::dead_code::CoercionParams": ("yes", "test-only schema struct"),
    "operant-core/src/schema.rs::dead_code::NumberParams": ("yes", "test-only schema struct"),
    "operant-core/src/schema.rs::dead_code::TestParams": ("yes", "test-only schema struct"),
    "operant-core/src/tools/browser_cdp_tool.rs::dead_code::BrowserCdpArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/checkpoint_tool.rs::dead_code::CheckpointArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/config_tool.rs::dead_code::ConfigManageArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON"),
    "operant-core/src/tools/debug_helpers.rs::dead_code::EnvVarArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/feishu_tool.rs::dead_code::FeishuDocArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/feishu_tool.rs::dead_code::FeishuDriveArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/home_assistant_tool.rs::dead_code::HomeAssistantArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/kernel/sidecar.rs::dead_code::bridge_rx": ("yes", "sidecar channel half kept for the shutdown path; allow where expect would work"),
    "operant-core/src/tools/notification_tool.rs::dead_code::ApprovalRequestArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/notification_tool.rs::dead_code::NotifyArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/osv_check.rs::dead_code::OsvVulnerability": ("yes", "deserialize-only response struct from the OSV API"),
    "operant-core/src/tools/process_tool.rs::dead_code::ProcessToolArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/reaction_tool.rs::dead_code::ReactArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools.rs::dead_code::TestArgs": ("yes", "test-only argument struct"),
    "operant-core/src/tools/send_message_tool.rs::dead_code::SendMessageArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/session_search_tool.rs::dead_code::SessionSearchArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/skills_tool.rs::dead_code::SkillsListArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/skills_tool.rs::dead_code::SkillViewArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/skills_tool.rs::dead_code::USAGE_TELEMETRY_LOCK": ("no", "iter-1163: a mutex read only when telemetry is enabled; cfg-conditional, so expect cannot expire correctly"),
    "operant-core/src/tools/spotify_tool.rs::dead_code::DevicesArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/spotify_tool.rs::dead_code::QueueArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/spotify_tool.rs::dead_code::SearchArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/sub_agent_tool.rs::dead_code::DelegationTask": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/sub_agent_tool.rs::dead_code::SubAgentArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/tool_backend_helpers.rs::dead_code::ToolBackendArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/src/tools/verification_tool.rs::dead_code::VerifyArgs": ("yes", "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"),
    "operant-core/tests/cron_session_isolation.rs::dead_code::cron": ("yes", "keeps the cron agent alive for the scheduler's lifetime"),
    "operant-core/tests/seat_policy_run_path.rs::dead_code::prompt": ("yes", "schema-only: the stub never reads the prompt"),
    "operant-harness/src/row.rs::dead_code::config": ("no", "carried for the host to read; the Phase 3 stub does not consume it yet (row.rs:16 and row.rs:74 share the name, so both keys exist)"),
    "operant-harness/src/row.rs::dead_code::FakeFilePromptSection": ("no", "a test stub the config-row prompt tests drive; cfg(test)-adjacent so expect would be unfulfilled outside the test profile"),
    "operant-harness/tests/builder_factories.rs::dead_code::_host_smoke": ("no", "a smoke harness kept for the next host boot; test-only by construction"),
    "operant-harness/tests/host_boot.rs::dead_code::_ensure_noop_linked": ("no", "a verification harness kept for the next host boot; test-only by construction"),
    "operant-memory/src/knowledge_graph.rs::dead_code::db_path": ("yes", "db path retained for diagnostics/reindex alongside the live conn"),
    "operant-plugins/src/host.rs::dead_code::verification": ("no", "iter-1163 converted this to #[expect]; a security verdict written and never read, to be wired into plugins() when verification is surfaced"),
    "operant-providers/src/anthropic.rs::dead_code::cache_creation_input_tokens": ("yes", "deserialized usage field for cache-accounting parity"),
    "operant-providers/src/bedrock.rs::dead_code::Other": ("no", "the deserialized catch-all arm of a wire enum; kept so an unknown block decodes rather than failing the response"),
    "operant-providers/src/bedrock.rs::dead_code::role": ("yes", "deserialized message role for payload parity"),    "operant-harness/src/pool_provider.rs::dead_code::PoolBundlePayload": ("no", "a serde payload shape the pool keeps for its own decode round-trip; feature-adjacent, so expect would go unfulfilled under one feature set"),
    "operant-providers/src/bedrock.rs::dead_code::stop_reason": ("yes", "deserialized stop reason for response metadata parity"),
    "operant-providers/src/gemini.rs::dead_code::InternalGenerateContentResponse": ("no", "cfg-dependent: read by the default-feature response path, unused with the full provider set; expect would go unfulfilled under one of the two feature sets (the site's own comment says so)"),
}

# Files whose text mention of the attribute is a note about its REMOVAL, not
# an attribute. `tui/banner.rs` and `tui/rustle.rs` each carry a head comment
# saying the module-level `#![allow(dead_code)]` was dropped in iter-392.
MENTION_ONLY = {
    "crates/operant-cli/src/tui/banner.rs",
    "crates/operant-cli/src/tui/rustle.rs",
    # `session_sqlite.rs` documents why its `db_path` field was deleted by
    # naming the attribute that used to cover it (iter-1163).
    "crates/operant-infra/src/session_sqlite.rs",
    # `redraw_reason.rs`'s test asserts the absence of an allow.
    "crates/operant-cli/src/tui/app/redraw_reason.rs",
    # `operant_model/adapter.rs` and `tui/copy_targets.rs` each carry MODULE
    # docs that spell the attribute out while explaining a decision about it;
    # the text is prose, not an attribute (adapter.rs:44 says its variants are
    # never constructed, copy_targets.rs:26 points at an expect on the module
    # declaration in tui/mod.rs).
    "crates/operant-cli/src/tui/operant_model/adapter.rs",
    "crates/operant-cli/src/tui/copy_targets.rs",
}


# Keywords that may PRECEDE the identifier the attribute decorates.
# They are skipped, not matched, so `pub config:` yields `config` and an
# inline `#[allow(dead_code)] serde_json::Value` yields the line fallback.
LEADING_KEYWORDS = re.compile(
    r"^(?:pub\s+(?:\([^)]*\)\s*)?)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\S+\s+)?"
    r"(?:const\s+)?(?:fn|struct|enum|static|let|type|impl|mod|trait)\s+"
)
# `pub` is optional on every item, so it has to be skipped here too — the
# old version matched the KEYWORD as the item name and keyed `pub config:` as
# `pub`, which is how two suppressed `config` fields in row.rs collided.
FALLBACK_IDENT = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)")
FIELD = re.compile(r"^(?:pub\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:")


def _relkey(path: str) -> str:
    """The canonical form of a suppression site: no leading `crates/`.

    The baseline is written crate-relative (`operant-core/src/...`) because that
    reads better, while `scan()` produces repo-relative paths
    (`crates/operant-core/src/...`) because it walks `crates/`. Normalising in
    ONE place — rather than remembering which spelling a given key uses — is
    what keeps the two in step; an earlier version had the prefix on neither
    side and the gate reported every entry as unexplained.
    """
    return path[len("crates/") :] if path.startswith("crates/") else path


BASELINE = {_relkey(k): v for k, v in BASELINE.items()}


def item_after(lines: list[str], start: int) -> str | None:
    """The identifier of the item the attribute decorates.

    Walks forward from the attribute, skipping blank lines, comments and other
    attributes, and returns the first item name it can read. Handles:

      * declarations  — `pub fn foo`, `struct Bar`, `static BAZ: T = ...`
      * fields        — `pub config: serde_json::Value,`
      * enum variants — `Other(#[allow(...)] serde_json::Value)` (the
        attribute is INLINE, so the identifier is on the SAME line and to the
        LEFT — handled by `item_on_line`)

    `start` is the 0-based index of the attribute's FIRST line, and the walk
    must first get out of the attribute's own body: rustfmt splits a long
    `#[cfg_attr(...)]` across lines, so the line after the opener is `test,` —
    which reads as an identifier and was being returned as the item name for
    every cfg_attr site in the repo. The walk therefore skips until the
    attribute's paren depth closes, THEN looks for the item.

    Returns None when no name can be read, and the caller keys the site by its
    line number instead — which is safe as long as it is rare and reported.
    """
    depth = 0
    seen_open = False
    idx = start
    while idx < len(lines):
        raw = lines[idx]
        for ch in raw:
            if ch == "(":
                depth += 1
                seen_open = True
            elif ch == ")":
                depth -= 1
        idx += 1
        if seen_open and depth <= 0:
            break
    for raw in lines[idx : idx + 4]:
        text = raw.strip()
        if not text or text.startswith("//") or text.startswith("#["):
            continue
        # A declaration: `pub struct Foo {`, `fn bar(`, `static BAZ:`.
        after = LEADING_KEYWORDS.sub("", text)
        if after is not text:
            m = FALLBACK_IDENT.match(after)
            if m:
                return m.group(1)
        # A field or a bare item: `pub config: T,` / `role: String,`.
        m = FIELD.match(text)
        if m:
            return m.group(1)
        # `let mut cmds = ...` / `prompt: String,` inside a struct body.
        m = re.match(r"(?:let\s+)?(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)", text)
        if m:
            return m.group(1)
    return None


def item_on_line(line: str) -> str | None:
    """The identifier an INLINE attribute decorates on its own line.

    `Other(#[allow(dead_code)] serde_json::Value)` — the variant name is to the
    left of the attribute, so read back from the `#\[`.
    """
    idx = line.find("#[")
    if idx <= 0:
        return None
    left = line[:idx].strip()
    m = re.search(r"([A-Za-z_][A-Za-z0-9_]*)\s*\($", left)
    if m:
        return m.group(1)
    m = re.search(r"([A-Za-z_][A-Za-z0-9_]*)", left)
    return m.group(1) if m else None



def scan() -> list[tuple[str, str, str, int]]:
    """Yield (relpath, lint, item, lineno) for every tracked suppression."""
    out = []
    for path in sorted(ROOT.rglob("*.rs")):
        rel = str(path.relative_to(ROOT.parent))
        if rel in MENTION_ONLY:
            continue
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError):
            continue
        # rustfmt splits any attribute whose text exceeds the line width, so
        # `#[cfg_attr(test, allow(dead_code, reason = "..."))]` reaches the disk
        # as four lines. A per-line regex cannot see it, and a gate that cannot
        # see a suppression is the blind spot this script exists to close.
        # Join each unclosed `#[` through its matching `)]` into one logical
        # line, keeping the ORIGINAL line numbers so a report still names the
        # line the reader sees. Comment lines are never joined.
        logical: list[tuple[int, str]] = []
        buf: list[str] = []
        start = 0
        depth = 0
        for i, line in enumerate(lines):
            if depth == 0:
                stripped = line.lstrip()
                if stripped.startswith(("//", "/*", "*")):
                    logical.append((i + 1, line))
                    continue
                start = i + 1
                buf = [line]
                depth = line.count("(") - line.count(")")
                if "#[" in line and depth <= 0:
                    logical.append((start, " ".join(buf)))
                    buf = []
                continue
            buf.append(line)
            depth += line.count("(") - line.count(")")
            if depth <= 0:
                logical.append((start, " ".join(buf)))
                buf = []
                depth = 0
        if buf:
            logical.append((start, " ".join(buf)))

        for lineno, line in logical:
            # A `#[allow(...)]` written INSIDE a doc comment is prose, not an
            # attribute — this repo's module heads cite the suppressions they
            # carry (`//! ... #[allow(checked_code)] ...`). Rust does not read
            # those; a gate that does reports the same site twice and invents
            # items named after the sentence around it (observed on memory.rs:
            # "dead_code on `None`" for the words `line16`/`test` nearby).
            if line.lstrip().startswith(("//", "/*", "*")):
                continue
            m = RE_ATTR.search(line)
            if not m:
                continue
            for lint in m.group("lints").split(","):
                if lint.strip() not in TRACKED:
                    continue
                # An INLINE attribute (`Other(#[allow(dead_code)] Value)`) names
                # its item on the same line, to the LEFT. A LEADING attribute
                # names the item on a following line. `m.start() > 0` is NOT a
                # test for inline — an indented `    #[expect(...)]` also
                # starts past 0, and every indented attribute in this repo
                # would be keyed as inline and read the wrong identifier. The
                # test is: is there NON-WHITESPACE before the `#[`?
                before = line[: m.start()]
                inline = bool(before.strip())
                # `lineno` is 1-based (what a reader sees in an editor) and
                # `item_after` takes the 0-based index of the line AFTER the
                # attribute. Passing `lineno` directly started the walk one line
                # late and skipped the item, so every leading attribute in this
                # repo was keyed by line number instead of by name — which is
                # exactly the ambiguity the key was chosen to avoid.
                item = item_on_line(line) if inline else (item_after(lines, lineno - 1) or f"line{lineno}")
                out.append((rel, lint.strip(), item, lineno))
    return out


def main() -> int:
    found = scan()
    print(f"lint suppressions: {len(found)} across {len({s[0] for s in found})} files")
    print(f"baseline entries:  {len(BASELINE)}\n")

    unbaselined, no_reason, conversion, already_expect = [], [], [], []

    for rel, lint, item, line in found:
        # The key must be unique; a collision means two suppressed items in one
        # file share a name and the baseline cannot tell them apart (which is
        # exactly the row.rs `config` case below).
        key = f"{_relkey(rel)}::{lint}::{item}"
        keys = [key] if key in BASELINE else [
            k for k in BASELINE if k.split("::", 1)[1] == f"{lint}::{item}" and k.startswith(f"{rel}::")
        ]
        if not keys:
            unbaselined.append(f"{rel}:{line}: {lint} on `{item}`")
            continue
        expectable, reason = BASELINE[keys[0]]
        if not reason:
            no_reason.append(f"{rel}:{line}: {lint} on `{item}`")
        if expectable == "n/a":
            already_expect.append(f"{rel}:{line}: {lint} on `{item}`")
        elif expectable == "yes":
            conversion.append(f"{rel}:{line}: {lint} on `{item}`")

    if no_reason:
        print("BASELINE ENTRIES WITH NO REASON (add one — a suppression with no")
        print("reason is the kind that silently outlives its subject):")
        for k in no_reason:
            print(f"  ? {k}")
        print()

    if conversion:
        print("CONVERSION CANDIDATES — an `allow` where `#[expect]` would work")
        print("(the lint fires on every build, so the escape hatch would expire")
        print("when it stops being needed). Outstanding work, not a failure:")
        for k in conversion:
            print(f"  ~ {k}")
        print()

    if unbaselined:
        print("UNEXPLAINED SUPPRESSIONS (not in the baseline — add an entry with")
        print("the reason, or remove the attribute):")
        for k in unbaselined:
            print(f"  ! {k}")
        print()

    print(f"correct `#[expect]` sites: {len(already_expect)}")
    print(f"conversion candidates:    {len(conversion)}")

    if "--check" in sys.argv[1:]:
        if unbaselined or no_reason:
            return 1
        print("audit-lint-suppressions: pass")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
