import re
P="crates/operant-channels/src/orchestrator/mod.rs"
src=open(P).read(); lines=src.split("\n")
pat=re.compile(r'^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:fn|struct|enum|trait|impl|const|static|type|mod)\b')
starts=[i for i,l in enumerate(lines,1) if pat.match(l) and i>=130]
TESTS=[s for s in starts if lines[s-1].startswith("mod tests")][0]
starts=[s for s in starts if s<TESTS]
def ext(s):
    j=s
    while j>1:
        st=lines[j-2].strip()
        if st=="" or not (st.endswith(";") or st.endswith("}") or st.endswith("{")):
            j-=1; continue
        break
    return j
exts=[ext(s) for s in starts]
blocks=[]
for k,s in enumerate(starts):
    lo=exts[k]
    hi=starts[k+1] if k+1<len(starts) else TESTS
    if k+1<len(starts): hi=min(hi,exts[k+1])
    blocks.append((s,lo,hi))
# name parsing
def name_of(s):
    l=lines[s-1]
    if l.startswith(("const ","static ")) or re.match(r'^(pub\s+)?(const|static)\b',l):
        m=re.search(r'\b(?:const|static)\s+(\w+)',l); return ("const",m.group(1))
    if l.startswith("impl"):
        if " for " in l.split("{")[0]:
            return ("impl",re.search(r'for\s+([A-Za-z_]\w*)',l).group(1))
        m=re.match(r'impl\b\s*(?:<[^>]*>\s*)?([A-Za-z_]\w*)',l); return ("impl",m.group(1))
    m=re.match(r'(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(fn|struct|enum|type|trait|mod)\s+(\w+)',l)
    return (m.group(1),m.group(2)) if m else (None,None)
T={}
for grp,names in {
 "runtime_types":["ChannelRouteSelection","ChannelRuntimeCommand","ModelCacheState","ModelCacheEntry","ChannelRuntimeDefaults","ConfigFileStamp","RuntimeConfigState","runtime_config_store","InterruptOnNewMessageConfig","ChannelCostTrackingState","ChannelRuntimeContext","InFlightSenderTaskState","InFlightTaskCompletion"],
 "history":["conversation_memory_key","conversation_history_key","followup_thread_id","interruption_scope_key","normalize_cached_channel_turns","strip_tool_result_content","strip_tool_summary_prefix","clear_sender_history","mark_sender_for_new_session","take_pending_new_session","compact_sender_history","proactive_trim_turns","append_sender_turn","extract_current_turn_tool_messages","strip_old_tool_context","is_tool_call_content","rollback_orphan_user_turn","should_rollback_failed_user_turn","is_context_window_overflow_error"],
 "commands":["is_stop_command","supports_runtime_model_switch","is_matrix_channel_name","parse_runtime_command","handle_runtime_command_if_needed"],
 "routing":["resolve_provider_alias","resolved_default_provider","resolved_default_model","runtime_defaults_from_config","runtime_config_path","runtime_defaults_snapshot","config_file_stamp","decrypt_optional_secret_for_runtime_reload","load_runtime_defaults_from_config_file","maybe_apply_runtime_config_update","default_route_selection","get_route_selection","set_route_selection","load_cached_model_preview","provider_cache_key","get_or_create_provider","create_resilient_provider_nonblocking"],
 "prompts":["channel_delivery_instructions","build_channel_system_prompt","replace_available_skills_section","refreshed_new_session_system_prompt","channel_runtime_string","channel_runtime_string_with_args","build_current_route_summary","build_models_help_response","build_providers_help_response","build_config_text_response","build_config_block_kit"],
 "memory_ctx":["should_skip_memory_context_entry","build_memory_context","build_memory_context_for_sessions","append_recalled_memory_entries","format_memory_context","is_group_reply_target","sender_memory_session_ids","extract_tool_context_summary"],
 "sanitize":["NoReplyKind","AssistantChannelOutcome","classify_channel_reply_intent","parse_reply_intent","outcome_for_no_reply","looks_like_meta_instruction_echo","strip_think_tags_inline","sanitize_channel_response","strip_tool_narration","is_tool_call_payload","is_tool_result_payload","sanitize_tool_json_value","is_line_isolated_json_segment","strip_isolated_tool_json_artifacts"],
 "supervision":["effective_channel_message_timeout_secs","channel_message_timeout_budget_secs","channel_message_timeout_budget_secs_with_cap","spawn_supervised_listener","spawn_supervised_listener_with_health_interval","compute_max_in_flight_messages","log_worker_join_result","spawn_scoped_typing_task"],
 "dispatch":["process_channel_message","dispatch_worker","run_message_dispatch_loop"],
 "identity":["normalize_telegram_identity","bind_telegram_identity","maybe_restart_managed_daemon_service"],
 "factory":["build_channel_by_id","send_channel_message"],
 "health":["ChannelHealthState","classify_health_result","ConfiguredChannel","collect_configured_channels","doctor_channels"],
 "startup":["start_channels"]}.items():
    for n in names: T[n]=grp
MODS=["consts","commands","context" if False else None]
ORDER=["consts","runtime_types","history","commands","routing","prompts","memory_ctx","sanitize","supervision","dispatch","identity","factory","health","startup"]
assign={}
unassigned=[]
for s,lo,hi in blocks:
    kind,name=name_of(s)
    if name in ("ConversationHistoryMap","PendingNewSessionSet","ProviderCacheMap","RouteSelectionMap"): g="runtime_types"
    elif name=="strip_tool_call_tags": g="sanitize"
    elif kind in("const","static"): g="consts"
    else: g=T.get(name)
    assign[s]=(lo,hi,g,name,kind)
    if g is None: unassigned.append((s,name,lines[s-1][:60]))
print("UNASSIGNED:",*unassigned,sep="\n  ")
# USES header: use-lines from top of file (exclude mod decls)
uses=[]
i=0
while i<136:
    l=lines[i]
    if re.match(r'^use\b',l):
        uses.append(l)
        while not l.rstrip().endswith(";"):
            i+=1; l=lines[i]; uses.append(l)
    i+=1
USES="\n".join(uses)+"\n"
import os
OUT="crates/operant-channels/src/orchestrator"
os.makedirs(OUT,exist_ok=True)
def widen(text):
    out=[]; mode=None
    hdr=re.compile(r'^(?:pub(?:\(crate\))?\s+)?(struct|enum|impl|fn|const|static|type)\b')
    for ln in text.split("\n"):
        if ln.strip()=="":
            out.append(ln); continue
        if ln==ln.lstrip():
            m=hdr.match(ln)
            if m:
                k=m.group(1)
                mode=('struct' if k=='struct' else 'enum' if k=='enum'
                      else ('impl_trait' if " for " in ln.split("{")[0] else 'impl_inherent') if k=='impl' else None)
            elif not ln.startswith(("#","///","//","}")):
                mode=None
            if not ln.startswith("pub") and re.match(r'^(async fn |fn |struct |enum |const |static |type )',ln):
                out.append("pub(crate) "+ln); continue
        else:
            if mode=='struct' and re.match(r'^    ([a-z_][a-z0-9_]*)\s*:',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]); continue
            if mode=='impl_inherent' and re.match(r'^    (?=(?:async\s+)?fn\b)',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]); continue
        out.append(ln)
    return "\n".join(out)

bodies={g:[] for g in ORDER}
kept=[]
kept_ranges=[]
for s,(lo,hi,g,n,k) in sorted(assign.items()):
    seg="\n".join(lines[lo-1:hi-1])
    if g is None: kept.append(seg)
    else: bodies[g].append(seg)
for g in ORDER:
    body="\n".join(x for x in bodies[g] if x.strip())
    with open(f"{OUT}/{g}.rs","w") as f:
        f.write(f"//! `{g}` — extracted verbatim from the former orchestrator/mod.rs monolith.\n//! Re-exported from `orchestrator` so every import path is unchanged.\n\n")
        f.write(USES+"\nuse super::*;\n\n"+widen(body).rstrip()+"\n")
# tests file
inner=lines[TESTS-1:]  # from "mod tests {"
assert inner[0].strip().startswith("mod tests"), inner[0][:40]
body_t=inner[1:]
while body_t and body_t[-1].strip()=="": body_t.pop()
assert body_t[-1].strip()=="}", body_t[-1][:40]
body_t=body_t[:-1]
open(f"{OUT}/tests.rs","w").write("//!\n"+"\n".join(body_t).rstrip()+"\n")
# rebuild mod.rs
head_end=blocks[0][1]-1  # up to first block start
new=lines[:head_end]
decl=["",
 "// Concern-group modules extracted verbatim from the former monolith (see BUGS.md).",
 " ".join(f"mod {g};" for g in ORDER),
 "#[cfg(test)]",
 "mod tests;",
 ""]
decl+= [" ".join(f"pub use {g}::*;" for g in ORDER[i:i+4]) + "" for i in range(0,len(ORDER),4)]
new+=decl
for s,(lo,hi,g,n,k) in sorted(assign.items()):
    if g is None:
        new+=lines[lo-1:hi-1]
open(P,"w").write("\n".join(new).rstrip()+"\n")
print("split done; mod.rs now", len(new),"lines")
