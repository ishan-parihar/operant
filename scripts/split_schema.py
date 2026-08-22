#!/usr/bin/env python3
"""Split operant-config/src/schema.rs monolith into schema/<domain>.rs modules.
Placement is navigation-only: mod.rs glob-re-exports every item so the public
path surface (`schema::X`) is unchanged."""
import os, re, sys

SRC = "crates/operant-config/src/schema.rs"
OUT = "crates/operant-config/src/schema"

src = open(SRC).read()

def net(line):
    out = []; i = 0; n = len(line); instr = None
    while i < n:
        c = line[i]
        if instr:
            if c == '\\': i += 2; continue
            if c == instr: instr = None
            i += 1; continue
        if c == '"': instr = '"'; i += 1; continue
        if c == "'":
            m = re.match(r"'(\\.|[^'\\])'", line[i:])
            if m: i += m.end(); continue
        if c == '/' and i + 1 < n and line[i + 1] == '/': break
        out.append(c); i += 1
    s = ''.join(out)
    return s.count('{') - s.count('}')

# ── segment into top-level chunks (attrs/docs attach forward) ──────────────
chunks = []; buf = []; depth = 0; started = False
for line in src.splitlines():
    buf.append(line)
    depth += net(line)
    if depth > 0: started = True
    hdr = [b for b in buf if b.strip() and not b.strip().startswith('//')]
    if depth <= 0 and started and hdr:
        chunks.append('\n'.join(buf))
        buf = []; started = False
if buf: chunks.append('\n'.join(buf))

# split off the leading use-block as shared header
c0_lines = chunks[0].splitlines()
i = 0; prelude = []
while i < len(c0_lines):
    s = c0_lines[i].strip()
    if not s:
        prelude.append(c0_lines[i]); i += 1; continue
    if s.startswith('//') or s.startswith('use ') or s.startswith('#['):
        stmt = [c0_lines[i]]
        while ';' not in stmt[-1] and i + 1 < len(c0_lines):
            i += 1; stmt.append(c0_lines[i])
        prelude.extend(stmt); i += 1; continue
    break
HEADER = '\n'.join(prelude) + '\n'
rest = '\n'.join(c0_lines[i:])
chunks = ([rest] if rest.strip() else []) + chunks[1:]
first_item = 1
print(f"chunks={len(chunks)} prelude_uses={len([p for p in prelude if p.strip()])}")

def subject(chunk):
    """Best-effort primary identifier of a chunk."""
    skip = ('#[', '///', '//')
    for ln in chunk.splitlines():
        s = ln.strip()
        if not s or any(s.startswith(p) for p in skip):
            continue
        m = re.match(
            r'(?:pub(?:\([^)]*\))? )?(?:async |unsafe )*(?:struct|enum|trait|type|const|static|fn)\s+([A-Za-z_]\w*)', s)
        if m: return m.group(1)
        m = re.match(r'macro_rules!\s+([A-Za-z_]\w*)', s)
        if m: return m.group(1)
        m = re.match(r'impl(?:<[^>]*>)?\s+(?:[A-Za-z_:<>, ]+\s+for\s+)?([A-Za-z_]\w*)', s)
        if m: return m.group(1)
        return None
    return None

RULES = [
    ("__always_helpers__", "helpers"),
    ("proxy", "proxy"), ("no_proxy", "proxy"), ("service_selector", "proxy"),
    ("tts", "tts"), ("elevenlabs", "tts"), ("piper", "tts"), ("edge_tts", "tts"),
    ("stt", "stt"), ("transcription", "stt"), ("whisper", "stt"), ("deepgram", "stt"),
    ("assemblyai", "stt"),
    ("mcp", "mcp"),
    ("delegate", "delegate"), ("swarm", "delegate"), ("max_depth", "delegate"), ("nodes_config", "delegate"), ("max_nodes", "delegate"),
    ("skill", "skills"), ("pacing", "agent_cfg"), ("creation_nudge", "skills"), ("memory_nudge", "agent_cfg"),
    ("cost", "cost"), ("pricing", "cost"), ("reserve_percent", "cost"), ("daily_limit", "cost"), ("monthly_limit", "cost"), ("warn_percent", "cost"),
    ("hardware", "hardware"), ("baud", "hardware"), ("peripheral", "hardware"),
    ("multimodal", "media"), ("image_", "media"), ("dalle", "media"), ("imagen", "media"), ("stability_", "media"), ("flux_", "media"), ("card_accent", "media"),
    ("claude_code", "runners"), ("codex_cli", "runners"), ("gemini_cli", "runners"), ("opencode_cli", "runners"), ("signature_mode", "runners"), ("runtime_kind", "runners"), ("docker", "runners"),
    ("gateway", "gateway"), ("pairing", "gateway"), ("webhook_rate", "gateway"), ("idempotency", "gateway"), ("otp_", "gateway"), ("webauthn", "gateway"), ("estop", "gateway"), ("pair_rate", "gateway"),
    ("channel", "channels_cfg"), ("telegram", "channels_cfg"), ("slack", "channels_cfg"), ("matrix", "channels_cfg"), ("whatsapp", "channels_cfg"), ("mqtt", "channels_cfg"),
    ("irc", "channels_cfg"), ("line_webhook", "channels_cfg"), ("wati", "channels_cfg"), ("mochat", "channels_cfg"), ("notion", "channels_cfg"), ("jira", "channels_cfg"),
    ("cloud_ops", "channels_cfg"), ("conversational_ai", "channels_cfg"), ("nevis", "channels_cfg"), ("ms365", "channels_cfg"), ("linkedin", "channels_cfg"),
    ("dingtalk", "channels_cfg"), ("feishu", "channels_cfg"), ("qq", "channels_cfg"), ("discord", "channels_cfg"), ("signal", "channels_cfg"), ("imessage", "channels_cfg"),
    ("autonomy", "agent_cfg"), ("loop_detection", "agent_cfg"), ("reasoning_effort", "agent_cfg"), ("max_tool_iterations", "agent_cfg"),
    ("agent_max", "agent_cfg"), ("tool_result_chars", "agent_cfg"), ("keep_tool_context", "agent_cfg"), ("inject_system_prompt", "agent_cfg"), ("system_prompt_chars", "agent_cfg"),
    ("tool_dispatcher", "agent_cfg"), ("history_messages", "agent_cfg"), ("context_tokens", "agent_cfg"), ("session_backend", "agent_cfg"),
    ("draft_update_interval", "channels_cfg"), ("multi_message_delay", "channels_cfg"), ("approval_timeout", "channels_cfg"), ("typing_cooldown", "channels_cfg"), ("dm_topic", "channels_cfg"),
    ("ollama", "providers_cfg"), ("wire_api", "providers_cfg"), ("provider_retries", "providers_cfg"), ("provider_backoff", "providers_cfg"),
    ("storage", "memory_store"), ("qdrant", "memory_store"), ("retrieval", "memory_store"), ("rerank", "memory_store"), ("fts_", "memory_store"),
    ("pgvector", "memory_store"), ("embedding", "memory_store"), ("hygiene", "memory_store"), ("archive_after", "memory_store"), ("purge_after", "memory_store"),
    ("conversation_retention", "memory_store"), ("vector_weight", "memory_store"), ("keyword_weight", "memory_store"), ("min_relevance", "memory_store"),
    ("cache_size", "memory_store"), ("chunk_size", "memory_store"), ("response_cache", "memory_store"), ("knowledge_", "memory_store"), ("conflict_threshold", "memory_store"), ("namespace", "memory_store"), ("audit_retention", "memory_store"),
    ("runtime_trace", "trace"),
    ("web_fetch", "web_tools"), ("web_search", "web_tools"), ("http_max_response", "web_tools"), ("http_timeout", "web_tools"),
    ("firecrawl", "web_tools"), ("link_enricher", "web_tools"), ("text_browser", "web_tools"), ("browser_", "web_tools"), ("shell_tool_timeout", "web_tools"),
    ("backup", "backup"), ("retention_days", "backup"),
    ("sop", "sop"),
    ("heartbeat", "scheduler"), ("scheduler", "scheduler"), ("cron", "scheduler"), ("job_type_decl", "scheduler"), ("delivery_mode", "scheduler"), ("two_phase", "scheduler"), ("max_run_history", "scheduler"),
    ("workspace", "workspace_cfg"), ("config_dir", "workspace_cfg"), ("tilde", "workspace_cfg"), ("active_workspace_state", "workspace_cfg"), ("temp_directory", "workspace_cfg"), ("workspaces_dir", "workspace_cfg"),
    ("plugins_dir", "plugins_cfg"), ("max_plugins", "plugins_cfg"),
    ("vi_strictness", "vi"), ("verifiable_intent", "vi"), ("deferred_loading", "vi"),
]

MACRO = "impl_enum_prop_kind"

def bucket_of(name):
    if not name: return "core"
    n = name.lower()
    if MACRO in n: return "core"
    # Generic serde-default helpers must be reachable from every domain file.
    if n.startswith("default_") or n in {"is_false", "normalize_comma_values"}:
        return "helpers"
    for pat, b in RULES:
        if pat in n: return b
    return "core"

buckets = {}
order = []
for ch in chunks:
    b = bucket_of(subject(ch))
    if b not in buckets:
        buckets[b] = []; order.append(b)
    buckets[b].append(ch)

def widen_visibility(chunk):
    """Top-level decls become pub(crate) so cross-domain globs work;
    pub(crate) items are NOT re-exported beyond the crate by mod.rs."""
    out = []
    for ln in chunk.splitlines():
        if re.match(r'^(async |unsafe )*(fn|const|static|struct|enum|type|trait)\b', ln):
            ln = re.sub(r'^(async |unsafe *)*', lambda m: 'pub(crate) ' + m.group(0), ln)
        out.append(ln)
    return '\n'.join(out)

os.makedirs(OUT, exist_ok=True)
for b in order:
    with open(f"{OUT}/{b}.rs", "w") as f:
        f.write(f"//! `{b}` configuration surface — extracted verbatim from the\n")
        f.write("//! former schema.rs monolith (dedup pass). Placement is navigational;\n")
        f.write("//! every item is re-exported from `schema::`.\n\n")
        f.write(HEADER)
        f.write("use super::*;\n")
        f.write("\n")
        f.write(widen_visibility("\n".join(c.rstrip() + "\n" for c in buckets[b])))

mods = sorted(order) + ["tests"]
with open(f"{OUT}/mod.rs", "w") as f:
    f.write("//! Configuration schema root. The former 19.8K-line schema.rs was\n")
    f.write("//! split into domain modules; this file re-exports every public item so\n")
    f.write("//! `schema::Name` paths are byte-for-byte compatible with the monolith.\n\n")
    for m in mods:
        if m == "tests":
            f.write("#[cfg(test)]\nmod tests;\n")
        else:
            f.write(f"mod {m};\n")
    f.write("\n")
    for m in mods:
        if m != "tests":
            f.write(f"pub use {m}::*;\n")

print("domains:", ", ".join(mods))
