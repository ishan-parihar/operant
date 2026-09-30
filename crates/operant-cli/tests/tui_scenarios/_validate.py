#!/usr/bin/env python3
"""Corpus validator for tui_scenario. Authoring tool, not part of the contract.

Checks, in order:
  1. every *.json under the tree parses
  2. required fields present, correct types, unique kebab-case names
  3. every `keys` string uses only tokens parse_key_sequence actually recognises
  4. every `assert` path resolves against App::debug_snapshot()'s real shape,
     and every operator is one of == != contains
  5. every `assert_screen` clause is contains:/not-contains:
  6. agent_script files exist and parse as MockAgentEvent arrays
  7. completeness: the render-ladder surface list vs the scenario list
"""
import json, pathlib, re, sys

ROOT = pathlib.Path(__file__).parent
REPO = ROOT.parents[3]
errors, warnings = [], []

# ---- ground truth, transcribed from the source files -------------------------
# cmd_tui_debug.rs::parse_key_sequence — the explicit table, plus the two
# generic grammars parse_generic_chord() adds on top of it.
NAMED = {"enter", "esc", "escape", "tab", "up", "down", "left", "right",
         "backspace", "bs", "ctrl+a", "ctrl+c", "ctrl+t", "ctrl+r", "shift+tab"}
GENERIC_CHORD = re.compile(r"(?:(?:ctrl|control|alt|meta|option|shift)\+)+[^<>,+\s]$")
ESCAPES = set("nt\\")

# app/commands.rs::debug_snapshot()
SNAP_TOP = {"should_exit", "is_streaming", "is_simulating", "plan_mode", "show_help",
            "show_reasoning", "fast_mode", "messages", "status_message", "model",
            "provider", "focus", "token_count", "any_modal_open", "overlays",
            # non-modal surfaces, deliberately not in `overlays` — see the
            # comment in debug_snapshot()
            "usage_overlay", "debug_overlay"}
# app/commands.rs::overlay_flags()  (34 entries, array length asserted in the crate)
OVERLAYS = {
    "help_overlay", "history_search_overlay", "global_search", "rewind_flow",
    "settings_screen", "theme_screen", "stats_dialog", "mcp_view", "agents_menu",
    "diff_viewer", "memory_file_selector", "skills_view", "plugins_hub",
    "journey_view", "hooks_config_menu", "voice_mode_notice", "model_picker",
    "session_browser", "session_branching", "export_dialog", "context_viz",
    "mcp_approval", "bypass_permissions_dialog", "effort_picker", "key_input_dialog",
    "custom_provider_dialog", "free_mode_dialog", "device_auth_dialog",
    "connect_dialog", "import_config_picker", "import_config_dialog",
    "command_palette", "ask_user_dialog", "permission_request"}

# Every surface in render/mod.rs::render_app, in Z-order.
SURFACES = [
    # (surface label, render target, expected scenario name or None)
    ("transcript", "render/messages.rs::render_messages", "transcript-assistant-message"),
    ("tool blocks", "render/tools.rs::render_tool_items_lines", "tool-block"),
    ("background-task rows", "background_tasks.rs::render_rows", "background-task-rows"),
    ("status row", "render/footer.rs::render_status_row", "status-row"),
    ("input pane", "render/footer.rs::render_input", "input-pane"),
    ("typeahead", "render/footer.rs::render_prompt_suggestions", "typeahead-suggestions"),
    ("footer bar", "render/footer.rs::render_footer", "footer-bar"),
    ("usage overlay", "usage_overlay.rs::render_usage_overlay", "usage-overlay"),
    ("permission dialog", "dialogs/permission.rs::render_permission_dialog", "permission-dialog"),
    ("rewind flow", "overlays.rs::render_rewind_flow", "rewind-flow"),
    ("help overlay", "overlays.rs::render_help_overlay", "help-overlay"),
    ("history search overlay", "overlays.rs::render_history_search_overlay", "history-search-overlay"),
    ("settings screen", "settings_screen.rs::render_settings_screen", "settings-screen"),
    ("theme screen", "theme_screen.rs::render_theme_screen", "theme-screen"),
    ("stats dialog", "stats_dialog.rs::render_stats_dialog", "stats-dialog"),
    ("mcp view", "mcp_view.rs::render_mcp_view", "mcp-view"),
    ("agents menu", "agents_view.rs::render_agents_menu", "agents-menu"),
    ("diff viewer", "diff_viewer.rs::render_diff_dialog", "diff-viewer-git"),
    ("global search", "overlays.rs::render_global_search", "global-search"),
    ("memory file selector", "memory_file_selector.rs::render_memory_file_selector", "memory-file-selector"),
    ("skills view", "skills_view.rs::render_skills_view", "skills-view"),
    ("plugins hub", "plugins_hub.rs::render_plugins_hub", "plugins-hub"),
    ("journey view", "journey_view.rs::render_journey_view", "journey-view"),
    ("hooks config menu", "hooks_config_menu.rs::render_hooks_config_menu", "hooks-config-menu"),
    ("voice mode notice", "voice_mode_notice.rs::render_voice_mode_notice", "voice-mode-notice"),
    ("import config dialog", "import_config_dialog.rs::render_import_config_dialog", "import-config-dialog"),
    ("bypass permissions dialog", "bypass_permissions_dialog.rs::render_bypass_permissions_dialog", "bypass-permissions-dialog"),
    ("ask user dialog", "ask_user_dialog.rs::render_ask_user_dialog", "ask-user-dialog"),
    ("effort picker", "effort_picker.rs::render_effort_picker", "effort-picker"),
    ("import config picker", "dialog_select.rs::render_dialog_select", "import-config-picker"),
    ("connect dialog", "dialog_select.rs::render_dialog_select", "connect-dialog"),
    ("key input dialog", "key_input_dialog.rs::render_key_input_dialog", "key-input-dialog"),
    ("custom provider dialog", "custom_provider_dialog.rs::render_custom_provider_dialog", "custom-provider-dialog"),
    ("free mode dialog", "free_mode_dialog.rs::render_free_mode_dialog", "free-mode-dialog"),
    ("device auth dialog", "device_auth_dialog.rs::render_device_auth_dialog", "device-auth-dialog"),
    ("command palette", "dialog_select.rs::render_dialog_select", "command-palette"),
    ("model picker", "model_picker.rs::render_model_picker", "model-picker"),
    ("session browser", "session_browser.rs::render_session_browser", "session-browser"),
    ("session branching", "session_branching.rs::render_session_branching", "session-branching"),
    ("export dialog", "export_dialog.rs::render_export_dialog", "export-dialog"),
    ("context viz", "context_viz.rs::render_context_viz", "context-viz"),
    ("mcp approval dialog", "dialogs/mcp_approval.rs::render_mcp_approval_dialog", "mcp-approval-dialog"),
    ("error modal", "render/utils.rs::render_error_modal", "error-modal"),
    ("notification banner", "notifications.rs::render_notification_banner", "notification-banner"),
    ("selection highlight", "render/selection.rs::apply_selection_highlight", "selection-highlight"),
    ("context menu", "render/selection.rs::render_context_menu", "context-menu"),
    ("thinking row (streaming)", "render/messages.rs::render_thinking_item", "thinking-block-streaming"),
    ("thinking row (reasoning attr)", "render/messages.rs::render_thinking_item", "thinking-block-reasoning-attr"),
    ("transcript tool-error line", "render/messages.rs::render_messages", "transcript-tool-error-line"),
    ("debug overlay", "debug/overlay.rs::render_debug_overlay", "debug-overlay"),
    # state-level alias of the permission branch (render_app tests `Option::is_some`)
    ("permission request (Option)", "dialogs/permission.rs::render_permission_dialog", "permission-request"),
    ("mcp approval (state)", "dialogs/mcp_approval.rs::render_mcp_approval_dialog", "mcp-approval"),
    # second opener for the shared diff renderer
    ("diff viewer (turn mode)", "diff_viewer.rs::render_diff_dialog", "diff-viewer-turn"),
]

MOCK_EVENTS = {
    # channel-fed variants — not AgentEvents; run_headless dispatches them
    "user": ({"text"}, set()),
    "permission_request": ({"tool"}, {"tool_id", "description"}),
    "user_question": ({"question"}, {"choices"}),
    "background_task": ({"goal"}, {"model", "status"}),
    "thinking": ({"content"}, set()),
    "reasoning": ({"text"}, set()),
    "content": ({"text"}, set()),
    "tool_start": ({"id", "name"}, {"arguments"}),
    "tool_complete": ({"id"}, {"name", "output"}),
    "tool_error": ({"id", "error"}, {"name"}),
    "usage": ({"input_tokens", "output_tokens"}, set()),
    "done": (set(), {"text", "reasoning"}),
    "error": ({"error"}, set()),
}

# ---- 1. JSON parses ----------------------------------------------------------
json_files = sorted(ROOT.rglob("*.json"))
for f in json_files:
    try:
        json.loads(f.read_text())
    except Exception as e:
        errors.append(f"JSON PARSE {f.relative_to(ROOT)}: {e}")
print(f"[1] json files parsed: {len(json_files)}")

scen_files = sorted(ROOT.glob("*.scenario.json"))
scenarios = {}
for f in scen_files:
    scenarios[f.name[: -len(".scenario.json")]] = json.loads(f.read_text())
print(f"[1] scenarios: {len(scenarios)}")


# ---- 2. field shape ----------------------------------------------------------
REQ = {"name": str, "surface": str, "opens_with": str, "keys": str, "size": str,
       "assert": list, "baseline": str, "style_baseline": str}
for name, d in scenarios.items():
    for k, t in REQ.items():
        if k not in d:
            errors.append(f"{name}: missing required field '{k}'")
        elif not isinstance(d[k], t):
            errors.append(f"{name}: field '{k}' is {type(d[k]).__name__}, want {t.__name__}")
    if d.get("name") != name:
        errors.append(f"{name}: name field '{d.get('name')}' != filename")
    if not re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", name):
        errors.append(f"{name}: not kebab-case")
    if not re.fullmatch(r"[0-9]+x[0-9]+", d.get("size", "")):
        errors.append(f"{name}: bad size '{d.get('size')}'")
    if not d.get("assert"):
        errors.append(f"{name}: assert list is empty")
    if d.get("baseline") != f"{name}.txt":
        errors.append(f"{name}: baseline '{d.get('baseline')}' != '{name}.txt'")
    if d.get("style_baseline") != f"{name}.style.txt":
        errors.append(f"{name}: style_baseline '{d.get('style_baseline')}' != '{name}.style.txt'")
    if d.get("reachability") == "blocked" and not d.get("blocked_by"):
        errors.append(f"{name}: reachability=blocked but no blocked_by")
    if not d.get("notes"):
        warnings.append(f"{name}: no notes")
print(f"[2] field shape checked: {len(scenarios)}")


# ---- 3. key-token legality ---------------------------------------------------
def check_keys(seq, where, errors):
    i = 0
    while i < len(seq):
        if seq[i] == "<":
            j = seq.find(">", i)
            if j != -1:
                tok = seq[i + 1:j].lower()
                fkey = re.fullmatch(r"f([1-9]|1[0-2])", tok)
                chord = GENERIC_CHORD.match(tok) and len(tok) - len(tok.rsplit("+", 1)[-1]) > 0
                if tok not in NAMED and not fkey and not chord:
                    errors.append(
                        f"{where}: '{seq[i:j+1]}' is NOT a parse_key_sequence token — it "
                        f"will be typed as literal characters. Legal: {sorted(NAMED)}, "
                        f"<f1>..<f12>, <ctrl|alt|shift>+<char>")
                i = j + 1
                continue
        if seq[i] == "\\":
            if i + 1 >= len(seq) or seq[i + 1] not in ESCAPES:
                errors.append(f"{where}: '\\{seq[i+1] if i+1<len(seq) else ''}' is not a "
                              f"recognised escape (only \\n \\t \\\\ are)")
            i += 2
            continue
        i += 1


for name, d in scenarios.items():
    # Blocked scenarios legitimately use a not-yet-existing key token: naming it IS
    # the contract. Only `live` scenarios must parse against today's token table.
    if d.get("reachability") != "blocked":
        check_keys(d["keys"], f"{name}.keys", errors)
        for v in d.get("variants", []) or []:
            if "keys" in v:
                check_keys(v["keys"], f"{name}.variants[{v['size']}].keys", errors)
    for v in d.get("variants", []) or []:
        if "keys" in v:
            check_keys(v["keys"], f"{name}.variants[{v['size']}].keys", errors)
        if not re.fullmatch(r"[0-9]+x[0-9]+", v.get("size", "")):
            errors.append(f"{name}: variant bad size {v.get('size')!r}")
print(f"[3] key tokens checked: {len(scenarios)}")


# ---- 4. assert paths resolve -------------------------------------------------
def check_asserts(items, where, errors):
    for a in items:
        if "==" in a:
            key, val = a.split("==", 1)
        elif "!=" in a:
            key, val = a.split("!=", 1)
        elif " contains " in a:
            key, val = a.split(" contains ", 1)
        else:
            errors.append(f"{where}: '{a}' has no ==, != or ' contains ' operator")
            continue
        key, val = key.strip(), val.strip()
        path = key[:-len(".visible")] and f"overlays.{key[:-len('.visible')]}" \
            if key.endswith(".visible") else key
        segs = path.split(".")
        if segs[0] not in SNAP_TOP:
            errors.append(f"{where}: '{key}' — top-level '{segs[0]}' is not in "
                          f"debug_snapshot()")
            continue
        if segs[0] == "overlays":
            if len(segs) != 2:
                errors.append(f"{where}: '{key}' — overlays.* takes exactly one segment")
            elif segs[1] not in OVERLAYS:
                errors.append(f"{where}: '{key}' — '{segs[1]}' is not an overlay_flags() "
                              f"key (a path that does not resolve FAILS the run)")
        elif len(segs) != 1:
            errors.append(f"{where}: '{key}' — '{segs[0]}' is a scalar, cannot descend")
        if not val:
            errors.append(f"{where}: '{a}' has an empty value")


for name, d in scenarios.items():
    check_asserts(d["assert"], f"{name}.assert", errors)
    for v in d.get("variants", []) or []:
        if "assert" in v:
            check_asserts(v["assert"], f"{name}.variants[{v['size']}].assert", errors)
print(f"[4] assert paths checked: {len(scenarios)}")


# ---- 5. assert_screen clauses ------------------------------------------------
for name, d in scenarios.items():
    for cl in d.get("assert_screen", []):
        if not (cl.startswith("contains:") or cl.startswith("not-contains:")):
            errors.append(f"{name}: assert_screen '{cl}' must start with contains: or "
                          f"not-contains:")
        if "," in cl:
            errors.append(f"{name}: assert_screen '{cl}' contains a comma, which "
                          f"evaluate_screen_assertions uses as the clause separator")
    for v in d.get("variants", []) or []:
        for cl in v.get("assert_screen", []):
            if not (cl.startswith("contains:") or cl.startswith("not-contains:")):
                errors.append(f"{name}.variants: bad assert_screen '{cl}'")
print(f"[5] assert_screen clauses checked: {len(scenarios)}")


# ---- 6. agent scripts --------------------------------------------------------
used_scripts = set()
for name, d in scenarios.items():
    s = d.get("agent_script")
    if not s:
        continue
    used_scripts.add(s)
    p = ROOT / "agent_scripts" / s
    if not p.exists():
        errors.append(f"{name}: agent_script '{s}' not found under agent_scripts/")
        continue
    try:
        evs = json.loads(p.read_text())
    except Exception as e:
        errors.append(f"{name}: agent_script '{s}' does not parse: {e}")
        continue
    if not isinstance(evs, list) or not evs:
        errors.append(f"{name}: agent_script '{s}' must be a non-empty array")
        continue
    for i, ev in enumerate(evs):
        t = ev.get("type")
        if t not in MOCK_EVENTS:
            errors.append(f"{name}: {s}[{i}] type '{t}' is not a MockAgentEvent variant "
                          f"({sorted(MOCK_EVENTS)})")
            continue
        req, opt = MOCK_EVENTS[t]
        keys = set(ev) - {"type"}
        if not req <= keys:
            errors.append(f"{name}: {s}[{i}] ({t}) missing {sorted(req - keys)}")
        if keys - req - opt:
            errors.append(f"{name}: {s}[{i}] ({t}) has unknown field(s) "
                          f"{sorted(keys - req - opt)}")
    if evs[-1].get("type") not in ("done", "error"):
        errors.append(f"{name}: {s} must terminate with a done or error event, got "
                      f"{evs[-1].get('type')!r}")
orphans = {p.name for p in (ROOT / "agent_scripts").glob("*.json")} - used_scripts
HARNESS_FIXTURES = {"empty.json"}  # documented in schema.json as the runner's own
# minimal-Done termination check, not tied to a surface
for o in sorted(orphans - HARNESS_FIXTURES):
    warnings.append(f"agent_scripts/{o} is not referenced by any scenario")
print(f"[6] agent scripts checked: {len(used_scripts)} referenced, "
      f"{len(orphans)} unreferenced")


# ---- 7. completeness ---------------------------------------------------------
excluded = {}
_excl_path = ROOT / "excluded.json"
if _excl_path.exists():
    for e in json.loads(_excl_path.read_text())["excluded"]:
        excluded[e["name"]] = e
        for f in ("surface", "reason", "baseline", "style_baseline"):
            if not e.get(f):
                errors.append(f"excluded.json[{e.get('name')}]: missing '{f}'")
        if not e.get("unblock_when"):
            warnings.append(f"excluded.json[{e.get('name')}]: no unblock_when")

expected = {s for _, _, s in SURFACES if s}
# Every excluded surface must correspond to a ledger entry with no scenario.
# A ledger entry with a name but no scenario file on disk IS a parked surface.
parked = {s for _, _, s in SURFACES if s and s not in scenarios}
if set(excluded) != parked:
    errors.append(f"excluded.json does not match the ledger: "
                  f"unlisted={sorted(parked - set(excluded))} "
                  f"stale={sorted(set(excluded) - parked)}")
for n, e in excluded.items():
    if n in scenarios:
        errors.append(f"{n}: listed in excluded.json but a scenario file still exists")
missing = sorted(expected - set(scenarios))
extra = sorted(set(scenarios) - expected)
print()
print("=== surface list (render/mod.rs::render_app, Z-order) vs scenarios ===")
for label, target, sname in SURFACES:
    present = ("EXCL" if sname in excluded else
               "OK  " if sname in scenarios else "MISS")
    reach = scenarios.get(sname, {}).get("reachability", "live") if sname in scenarios else "-"
    print(f"  {present} [{reach:7}] {label:28} {target}")
print(f"\nsurfaces enumerated : {len(SURFACES)}")
print(f"scenarios on disk   : {len(scenarios)}")
print(f"parked in excluded  : {len(excluded)} -> {', '.join(sorted(excluded)) or 'none'}")
_unlisted = sorted(set(missing) - set(excluded))
print(f"missing             : {_unlisted or 'none'}"
      f"{'  [parked in excluded.json: ' + ', '.join(sorted(set(missing) & set(excluded))) + ']' if set(missing) & set(excluded) else ''}")
print(f"extra               : {extra or 'none'}")
live = [n for n, d in scenarios.items() if d.get("reachability") != "blocked"]
blocked = [n for n, d in scenarios.items() if d.get("reachability") == "blocked"]
if blocked:
    warnings.append(f"still marked reachability=blocked: {', '.join(sorted(blocked))}")
print(f"live ({len(live)})         : {', '.join(sorted(live))}")
print(f"blocked ({len(blocked)})      : {', '.join(sorted(blocked))}")

print()
if warnings:
    print(f"WARNINGS ({len(warnings)}):")
    for x in warnings:
        print(f"  ! {x}")
if errors:
    print(f"ERRORS ({len(errors)}):")
    for x in errors:
        print(f"  x {x}")
    sys.exit(1)
print("VALIDATION PASSED — 0 errors")
