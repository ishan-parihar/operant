//! Background memory/skill review — fork the agent to evaluate the turn.
//!
//! After every turn, `OperantAgent::run()` may trigger a background review
//! that evaluates whether any skill or memory should be saved or updated.
//! Writes go straight to the memory + skill stores. Main conversation and
//! prompt cache are never touched.
//!
//! ## Auxiliary Model Routing
//!
//! The review agent runs on the MAIN model by default ("auto"), replaying
//! the full conversation — already warm in the prompt cache, so cheap cache
//! reads. A user can route the review to a different, cheaper model via
//! `auxiliary.background_review.{provider,model}`. A different model cannot
//! reuse the parent's cache (different key), so the fork is cold regardless
//! — replaying the full transcript would just cold-write it. So when (and
//! only when) routed to a different model, we replay a compact DIGEST to
//! minimise cold-written tokens. Same model -> full replay; different model
//! -> digest. That's the whole policy.
//!
//! Ported from `hermes-agent/agent/background_review.py`.

use tracing::debug;

// ---------------------------------------------------------------------------
// Review prompt strings
// ---------------------------------------------------------------------------

/// Review prompt for skill updates. This is the message sent to the
/// review agent fork when a skill review is triggered.
pub const SKILL_REVIEW_PROMPT: &str = "\
Review the conversation above and update the skill library. Be \
ACTIVE — most sessions produce at least one skill update, even if \
small. A pass that does nothing is a missed learning opportunity, \
not a neutral outcome.

Target shape of the library: CLASS-LEVEL skills, each with a rich \
SKILL.md and a `references/` directory for session-specific detail. \
Not a long flat list of narrow one-session-one-skill entries. This \
shapes HOW you update, not WHETHER you update.

Signals to look for (any one of these warrants action):
  • User corrected your style, tone, format, legibility, or \
verbosity. Frustration signals like 'stop doing X', 'this is too \
verbose', 'don't format like this', 'why are you explaining', \
'just give me the answer', 'you always do Y and I hate it', or an \
explicit 'remember this' are FIRST-CLASS skill signals, not just \
memory signals. Update the relevant skill(s) to embed the \
preference so the next session starts already knowing.
  • User corrected your workflow, approach, or sequence of steps. \
Encode the correction as a pitfall or explicit step in the skill \
that governs that class of task.
  • Non-trivial technique, fix, workaround, debugging path, or \
tool-usage pattern emerged that a future session would benefit \
from. Capture it.
  • A skill that got loaded or consulted this session turned out \
to be wrong, missing a step, or outdated. Patch it NOW.

Preference order — prefer the earliest action that fits, but do \
pick one when a signal above fired:
  1. UPDATE A CURRENTLY-LOADED SKILL. Look back through the \
conversation for skills the user loaded via /skill-name or you \
read via skill_view. If any of them covers the territory of the \
new learning, PATCH that one first. It is the skill that was in \
play, so it's the right one to extend.
  2. UPDATE AN EXISTING UMBRELLA (via skills_list + skill_view). \
If no loaded skill fits but an existing class-level skill does, \
patch it. Add a subsection, a pitfall, or broaden a trigger.
  3. ADD A SUPPORT FILE under an existing umbrella. Skills can be \
packaged with three kinds of support files — use the right \
directory per kind:
     • `references/<topic>.md` — session-specific detail (error \
transcripts, reproduction recipes, provider quirks) AND \
condensed knowledge banks: quoted research, API docs, external \
authoritative excerpts, or domain notes you found while working \
on the problem. Write it concise and for the value of the task, \
not as a full mirror of upstream docs.
     • `templates/<name>.<ext>` — starter files meant to be \
copied and modified (boilerplate configs, scaffolding, a \
known-good example the agent can `reproduce with modifications`).
     • `scripts/<name>.<ext>` — statically re-runnable actions \
the skill can invoke directly (verification scripts, fixture \
generators, deterministic probes, anything the agent should run \
rather than hand-type each time).
     Add support files via skill_manage action=write_file with \
file_path starting 'references/', 'templates/', or 'scripts/'. \
The umbrella's SKILL.md should gain a one-line pointer to any \
new support file so future agents know it exists.
  4. CREATE A NEW CLASS-LEVEL UMBRELLA SKILL when no existing \
skill covers the class. The name MUST be at the class level. \
The name MUST NOT be a specific PR number, error string, feature \
codename, library-alone name, or 'fix-X / debug-Y / audit-Z-today' \
session artifact. If the proposed name only makes sense for \
today's task, it's wrong — fall back to (1), (2), or (3).

User-preference embedding (important): when the user expressed a \
style/format/workflow preference, the update belongs in the \
SKILL.md body, not just in memory. Memory captures 'who the user \
is and what the current situation and state of your operations \
are'; skills capture 'how to do this class of task for this \
user'. When they complain about how you handled a task, the \
skill that governs that task needs to carry the lesson.

If you notice two existing skills that overlap, note it in your \
reply — the background curator handles consolidation at scale.

Protected skills (DO NOT edit these):
  • Bundled skills (shipped with Hermes, e.g. 'hermes-agent').
  • Hub-installed skills (installed via 'hermes skills install').
Pinned skills (marked via 'hermes curator pin') CAN be improved — \
pin only blocks deletion/archive/consolidation by the curator, not \
content updates. Patch them when a pitfall or missing step turns up, \
same as any other agent-created skill.
If the only skills that need updating are protected, say\
'Nothing to save.' and stop.

Do NOT capture (these become persistent self-imposed constraints \
that bite you later when the environment changes):
  • Environment-dependent failures: missing binaries, fresh-install \
errors, post-migration path mismatches, 'command not found', \
unconfigured credentials, uninstalled packages. The user can fix \
these — they are not durable rules.
  • Negative claims about tools or features ('browser tools do not \
work', 'X tool is broken', 'cannot use Y from execute_code'). These \
harden into refusals the agent cites against itself for months \
after the actual problem was fixed.
  • Session-specific transient errors that resolved before the \
conversation ended. If retrying worked, the lesson is the retry \
pattern, not the original failure.
  • One-off task narratives. A user asking 'summarize today's \
market' or 'analyze this PR' is not a class of work that warrants \
a skill.

If a tool failed because of setup state, capture the FIX (install \
command, config step, env var to set) under an existing setup or \
troubleshooting skill — never 'this tool does not work' as a \
standalone constraint.

'Nothing to save.' is a real option but should NOT be the \
default. If the session ran smoothly with no corrections and \
produced no new technique, just say 'Nothing to save.' and stop. \
Otherwise, act.";

/// Review prompt for memory updates.
pub const MEMORY_REVIEW_PROMPT: &str = "\
Review the conversation above and consider saving to memory if appropriate.

Focus on:
1. Has the user revealed things about themselves — their persona, desires, \
preferences, or personal details worth remembering?
2. Has the user expressed expectations about how you should behave, their work \
style, or ways they want you to operate?

If something stands out, save it using the memory tool. \
If nothing is worth saving, just say 'Nothing to save.' and stop.";

/// Combined review prompt for both memory and skill updates.
pub const COMBINED_REVIEW_PROMPT: &str = "\
Review the conversation above and update two things:

**Memory**: who the user is. Did the user reveal persona, \
desires, preferences, personal details, or expectations about \
how you should behave? Save facts about the user and durable \
preferences with the memory tool.

**Skills**: how to do this class of task. Be ACTIVE — most \
sessions produce at least one skill update. A pass that does \
nothing is a missed learning opportunity, not a neutral outcome.

Target shape of the skill library: CLASS-LEVEL skills with a rich \
SKILL.md and a `references/` directory for session-specific detail. \
Not a long flat list of narrow one-session-one-skill entries.

Signals that warrant a skill update (any one is enough):
  • User corrected your style, tone, format, legibility, \
verbosity, or approach. Frustration is a FIRST-CLASS skill \
signal, not just a memory signal. 'stop doing X', 'don't format \
like this', 'I hate when you Y' — embed the lesson in the skill \
that governs that task so the next session starts fixed.
  • Non-trivial technique, fix, workaround, or debugging path \
emerged.
  • A skill that was loaded or consulted turned out wrong, \
missing, or outdated — patch it now.

Preference order for skills — pick the earliest that fits:
  1. UPDATE A CURRENTLY-LOADED SKILL. Check what skills were \
loaded via /skill-name or skill_view in the conversation. If one \
of them covers the learning, PATCH it first. It was in play; \
it's the right place.
  2. UPDATE AN EXISTING UMBRELLA (skills_list + skill_view to \
find the right one). Patch it.
  3. ADD A SUPPORT FILE under an existing umbrella via \
skill_manage action=write_file. Three kinds: \
`references/<topic>.md` for session-specific detail OR condensed \
knowledge banks (quoted research, API docs excerpts, domain \
notes) written concise and task-focused; `templates/<name>.<ext>` \
for starter files meant to be copied and modified; \
`scripts/<name>.<ext>` for statically re-runnable actions \
(verification, fixture generators, probes). Add a one-line \
pointer in SKILL.md so future agents find them.
  4. CREATE A NEW CLASS-LEVEL UMBRELLA when nothing exists. \
Name at the class level — NOT a PR number, error string, \
codename, library-alone name, or 'fix-X / debug-Y' session \
artifact. If the name only fits today's task, fall back to (1), \
(2), or (3).

User-preference embedding: when the user complains about how \
you handled a task, update the skill that governs that task — \
memory alone isn't enough. Memory says 'who the user is and \
what the current situation and state of your operations are'; \
skills say 'how to do this class of task for this user'. Both \
should carry user-preference lessons when relevant.

If you notice overlapping existing skills, mention it — the \
background curator handles consolidation.

Protected skills (DO NOT edit these):
  • Bundled skills (shipped with Hermes, e.g. 'hermes-agent').
  • Hub-installed skills (installed via 'hermes skills install').
Pinned skills (marked via 'hermes curator pin') CAN be improved — \
pin only blocks deletion/archive/consolidation by the curator, not \
content updates. Patch them when a pitfall or missing step turns up, \
same as any other agent-created skill.
If the only skills that need updating are protected, say\
'Nothing to save.' and stop.

Do NOT capture as skills (these become persistent self-imposed \
constraints that bite you later when the environment changes):
  • Environment-dependent failures: missing binaries, fresh-install \
errors, post-migration path mismatches, 'command not found', \
unconfigured credentials, uninstalled packages. The user can fix \
these — they are not durable rules.
  • Negative claims about tools or features ('browser tools do not \
work', 'X tool is broken', 'cannot use Y from execute_code'). These \
harden into refusals the agent cites against itself for months \
after the actual problem was fixed.
  • Session-specific transient errors that resolved before the \
conversation ended. If retrying worked, the lesson is the retry \
pattern, not the original failure.
  • One-off task narratives. A user asking 'summarize today's \
market' or 'analyze this PR' is not a class of work that warrants \
a skill.

If a tool failed because of setup state, capture the FIX (install \
command, config step, env var to set) under an existing setup or \
troubleshooting skill — never 'this tool does not work' as a \
standalone constraint.

Act on whichever of the two dimensions has real signal. If \
genuinely nothing stands out on either, say 'Nothing to save.' \
and stop — but don't reach for that conclusion as a default.";

// ---------------------------------------------------------------------------
// Notification mode
// ---------------------------------------------------------------------------

/// Controls how background review actions are surfaced to the user.
/// Matches hermes-agent's `memory_notifications` setting.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotificationMode {
    /// Show no actions (silent review).
    Off,
    /// Show generic "Memory updated" / tool messages.
    #[default]
    On,
    /// Include compact content previews from tool-call arguments.
    Verbose,
}

impl std::fmt::Display for NotificationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::On => write!(f, "on"),
            Self::Verbose => write!(f, "verbose"),
        }
    }
}

impl std::str::FromStr for NotificationMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "on" => Ok(Self::On),
            "verbose" => Ok(Self::Verbose),
            _ => Err(format!("Unknown notification mode: {}", s)),
        }
    }
}

// ---------------------------------------------------------------------------
// Build review prompt
// ---------------------------------------------------------------------------

/// Build the review prompt based on which triggers fired.
///
/// Returns the appropriate prompt string for the background review agent.
pub fn build_review_prompt(review_memory: bool, review_skills: bool) -> String {
    if review_memory && review_skills {
        COMBINED_REVIEW_PROMPT.to_string()
    } else if review_memory {
        MEMORY_REVIEW_PROMPT.to_string()
    } else {
        SKILL_REVIEW_PROMPT.to_string()
    }
}

// ---------------------------------------------------------------------------
// Configuration for the background review daemon
// ---------------------------------------------------------------------------

/// Configuration for the background review daemon.
///
/// Used to construct `SelfEvolutionState` and configure the review agent.
///
/// NOTE: Auxiliary model routing and notification mode are read directly
/// from `runtime_config()` in `spawn_background_review`, not from these
/// fields. Those fields were removed (YAGNI) because the config was
/// constructed but never read by the review daemon.
#[derive(Debug, Clone)]
pub struct BackgroundReviewConfig {
    /// Skill nudge interval (default: 10).
    pub skill_nudge_interval: usize,
    /// Memory review interval in turns (default: 5).
    pub memory_review_interval: usize,
}

impl Default for BackgroundReviewConfig {
    fn default() -> Self {
        Self {
            skill_nudge_interval: 10,
            memory_review_interval: 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Self-evolution state tracking
// ---------------------------------------------------------------------------

/// State tracking for the self-evolution pipeline.
///
/// Tracks iteration counts and nudge thresholds across turns.
/// Used by `OperantAgent::run()` to determine when skill/memory
/// reviews should be triggered.
pub struct SelfEvolutionState {
    /// Number of iterations since the last skill_manage call.
    pub iters_since_skill: usize,
    /// How many iterations between skill nudges (0 = disabled).
    pub skill_nudge_interval: usize,
    /// Number of turns since the last memory review.
    pub turns_since_memory_review: usize,
    /// How many turns between memory reviews (0 = disabled).
    pub memory_review_interval: usize,
}

impl SelfEvolutionState {
    /// Create a new state with the given configuration.
    pub fn new(config: &BackgroundReviewConfig) -> Self {
        Self {
            iters_since_skill: 0,
            skill_nudge_interval: config.skill_nudge_interval,
            turns_since_memory_review: 0,
            memory_review_interval: config.memory_review_interval,
        }
    }

    /// Increment the skill iteration counter (called each agent iteration).
    pub fn bump_skill_counter(&mut self) {
        self.iters_since_skill += 1;
    }

    /// Reset the skill iteration counter (called when skill_manage is used).
    pub fn reset_skill_counter(&mut self) {
        self.iters_since_skill = 0;
    }

    /// Check if a skill review should be triggered.
    pub fn should_review_skills(&self) -> bool {
        self.skill_nudge_interval > 0 && self.iters_since_skill >= self.skill_nudge_interval
    }

    /// Increment the memory turn counter (called each completed turn).
    pub fn bump_memory_counter(&mut self) {
        self.turns_since_memory_review += 1;
    }

    /// Reset the memory turn counter (called after a memory review fires).
    pub fn reset_memory_counter(&mut self) {
        self.turns_since_memory_review = 0;
    }

    /// Check if a memory review should be triggered.
    pub fn should_review_memory(&self) -> bool {
        self.memory_review_interval > 0
            && self.turns_since_memory_review >= self.memory_review_interval
    }

    // ── Hydration / Persistence (Phase 4) ───────────────────────────
    // When a session is resumed via persistent_session_id, the in-memory
    // counters start at 0. Hydrate them from session_metadata so the
    // review cadence continues where it left off.

    /// Hydrate counters from a metadata map (loaded from session_metadata).
    ///
    /// Keys: `evo_turns_since_memory`, `evo_iters_since_skill`.
    /// Missing keys are treated as 0 (first run of a session).
    pub fn hydrate_from_metadata(&mut self, metadata: &std::collections::HashMap<String, String>) {
        if let Some(val) = metadata.get("evo_turns_since_memory") {
            if let Ok(n) = val.parse::<usize>() {
                self.turns_since_memory_review = n;
                debug!(
                    turns = n,
                    "Hydrated memory review counter from persisted session"
                );
            }
        }
        if let Some(val) = metadata.get("evo_iters_since_skill") {
            if let Ok(n) = val.parse::<usize>() {
                self.iters_since_skill = n;
                debug!(
                    iters = n,
                    "Hydrated skill nudge counter from persisted session"
                );
            }
        }
    }

    /// Serialize current counters to a key-value map suitable for
    /// `Database::set_session_metadata`. Call after each turn to persist
    /// the counters so they survive session restarts.
    pub fn persist_counters(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "evo_turns_since_memory",
                self.turns_since_memory_review.to_string(),
            ),
            ("evo_iters_since_skill", self.iters_since_skill.to_string()),
        ]
    }
}

// ---------------------------------------------------------------------------
// Digest history for routed models
// ---------------------------------------------------------------------------

/// Compact replay for the routed (different-model) path only.
///
/// Keeps the recent `tail` messages verbatim, collapses older turns into one
/// synthetic user-role digest, preserving role alternation. Used ONLY when
/// routed to a different model (cache cold regardless, so fewer cold-written
/// tokens is a pure win). Never on the main-model path (full replay stays warm).
///
/// Ported from `hermes-agent/agent/background_review.py::_digest_history`.
pub fn digest_history(
    messages_snapshot: &[crate::client::Message],
    tail: usize,
) -> Vec<crate::client::Message> {
    use crate::client::{Message, Role};

    let msgs = messages_snapshot;
    if msgs.len() <= tail {
        return msgs.to_vec();
    }

    let mut keep: Vec<Message> = msgs[msgs.len() - tail..].to_vec();

    // Don't start with a tool message — extend keep to include the preceding assistant message.
    // Use checked_sub to avoid underflow when keep.len() approaches msgs.len().
    while !keep.is_empty() && keep[0].role == Role::Tool {
        let Some(new_start) = msgs.len().checked_sub(keep.len() + 1) else {
            break;
        };
        let min_start = msgs.len().saturating_sub(tail);
        if new_start < min_start {
            break;
        }
        keep.insert(0, msgs[new_start].clone());
    }

    let old = &msgs[..msgs.len() - keep.len()];
    let mut lines: Vec<String> = Vec::new();

    for m in old {
        let text = m.content.trim();
        match m.role {
            Role::User => {
                if !text.is_empty() {
                    // Truncate to 300 chars
                    let truncated: String = text.chars().take(300).collect();
                    lines.push(format!("USER: {}", truncated));
                }
            }
            Role::Assistant => {
                if let Some(tool_calls) = &m.tool_calls {
                    let names: Vec<String> = tool_calls
                        .iter()
                        .map(|tc| tc.function.name.clone())
                        .collect();
                    lines.push(format!("ASSISTANT[tools: {}]", names.join(", ")));
                }
                if !text.is_empty() {
                    let truncated: String = text.chars().take(200).collect();
                    lines.push(format!("ASSISTANT: {}", truncated));
                }
            }
            _ => {}
        }
    }

    let digest_content = format!(
        "[Earlier conversation digest — older turns summarised to bound the \
         review's cold-write cost on the routed aux model. Recent turns \
         follow verbatim below.]\
         \n{}",
        lines.join("\n")
    );

    let mut result = vec![Message::user(digest_content)];
    result.extend(keep);
    result
}

// ---------------------------------------------------------------------------
// Action summaries
// ---------------------------------------------------------------------------

/// Summary of actions taken by the background review.
///
/// Used by the background review daemon to surface a compact
/// summary of skill/memory changes to the user.
#[derive(Debug, Clone, Default)]
pub struct BackgroundReviewSummary {
    /// Human-readable action descriptions.
    pub actions: Vec<String>,
    /// Whether any skills were created/updated.
    pub skills_changed: bool,
    /// Whether any memory entries were added/updated.
    pub memory_changed: bool,
}

/// Build a compact action summary from background review messages.
///
/// Scans the review agent's messages for successful tool actions and
/// surfaces a compact summary to the user. Matches hermes-agent's
/// `summarize_background_review_actions`.
///
/// `notification_mode` controls display detail:
/// - `Off`: return no actions.
/// - `On`: generic "Memory updated" / tool messages.
/// - `Verbose`: include compact content previews from tool-call arguments.
///
/// NOTE: only exercised by unit tests today; the TUI surfacing hook isn't
/// wired yet (see docs/DEAD_CODE_GAP_ANALYSIS.md). `cfg_attr(not(test), …)`
/// keeps the lib-only build quiet without making `--all-targets` warn about
/// an unfulfilled `#[expect]`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn summarize_review_actions(
    review_messages: &[String],
    prior_messages: &[String],
    notification_mode: NotificationMode,
) -> BackgroundReviewSummary {
    if notification_mode == NotificationMode::Off {
        return BackgroundReviewSummary::default();
    }

    let verbose = notification_mode == NotificationMode::Verbose;
    let mut summary = BackgroundReviewSummary::default();

    // Collect existing tool call IDs from prior messages to avoid re-surfacing
    let prior_tool_ids: std::collections::HashSet<String> = prior_messages
        .iter()
        .filter_map(|m| {
            let data: serde_json::Value = serde_json::from_str(m).ok()?;
            if data.get("role").and_then(|v| v.as_str()) == Some("tool") {
                data.get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            } else {
                None
            }
        })
        .collect();

    // Map review-agent tool results back to the calls that produced them.
    // The result JSON only says "Entry added"; the call arguments contain
    // action, target, and content previews.
    let notify_tools: std::collections::HashSet<&str> = [
        "memory_store",
        "memory_search",
        "memory_recall",
        "skill_manage",
        "skill_view",
    ]
    .iter()
    .copied()
    .collect();

    let mut all_tool_call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut call_details: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();

    // First pass: collect assistant messages with tool calls
    for msg_str in review_messages {
        let data: serde_json::Value = match serde_json::from_str(msg_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if data.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }

        if let Some(tool_calls) = data.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tool_calls {
                let fn_obj = tc.get("function").and_then(|v| v.as_object());
                let fn_name = fn_obj
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tcid = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");

                if !tcid.is_empty() {
                    all_tool_call_ids.insert(tcid.to_string());
                }

                if !notify_tools.contains(fn_name) {
                    continue;
                }

                let args: serde_json::Value = fn_obj
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();

                call_details.insert(
                    tcid.to_string(),
                    serde_json::json!({
                        "tool": fn_name,
                        "action": args.get("action").and_then(|v| v.as_str()).unwrap_or("?"),
                        "target": args.get("target").and_then(|v| v.as_str()).unwrap_or("memory"),
                        "content": args.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                        "name": args.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    }),
                );
            }
        }
    }

    // Second pass: collect tool results
    for msg_str in review_messages {
        let data: serde_json::Value = match serde_json::from_str(msg_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if data.get("role").and_then(|v| v.as_str()) != Some("tool") {
            continue;
        }

        let tcid = data
            .get("tool_call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !tcid.is_empty() && prior_tool_ids.contains(tcid) {
            continue;
        }

        if !tcid.is_empty() && !all_tool_call_ids.is_empty() && !call_details.contains_key(tcid) {
            continue;
        }

        let content_str = data.get("content").and_then(|v| v.as_str()).unwrap_or("{}");

        let result: serde_json::Value = serde_json::from_str(content_str).unwrap_or_default();
        if !result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }

        let message = result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if message.is_empty() {
            continue;
        }

        let detail = call_details.get(tcid).cloned().unwrap_or_default();
        let message_lower = message.to_lowercase();
        let is_skill = detail
            .get("tool")
            .and_then(|v| v.as_str())
            == Some("skill_manage")
            // Fallback: when no assistant tool-call context is available
            // (e.g. standalone tool messages), infer from message content.
            || message_lower.contains("updated skill") || message_lower.contains("skill_manage");

        if !verbose
            && (message_lower.contains("created")
                || message_lower.contains("updated")
                || (is_skill && message_lower.contains("patched")))
        {
            summary.actions.push(message);
            if is_skill {
                summary.skills_changed = true;
            } else {
                summary.memory_changed = true;
            }
            continue;
        }

        // Verbose mode: include content previews
        let label = if is_skill {
            "Skill"
        } else {
            let target = detail
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("memory");
            match target {
                "memory" => "Memory",
                "user" => "User profile",
                _ => "Memory",
            }
        };

        let max_preview = 120;

        if verbose {
            let action = detail.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let content = detail.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let skill_name = detail.get("name").and_then(|v| v.as_str()).unwrap_or("");

            if is_skill {
                if action == "patch" && !content.is_empty() {
                    let preview: String = content.chars().take(80).collect();
                    let suffix = if content.len() > 80 { "…" } else { "" };
                    summary.actions.push(format!(
                        "📝 Skill '{}' patched: \"{}{}\"",
                        skill_name, preview, suffix
                    ));
                } else if action == "create" {
                    summary
                        .actions
                        .push(format!("📝 Skill '{}' created: {}", skill_name, message));
                } else if action == "edit" {
                    summary
                        .actions
                        .push(format!("📝 Skill '{}' rewritten: {}", skill_name, message));
                } else {
                    summary.actions.push(format!("📝 {}", message));
                }
                summary.skills_changed = true;
            } else if !content.is_empty() {
                let preview: String = content.chars().take(max_preview).collect();
                let suffix = if content.len() > max_preview {
                    "…"
                } else {
                    ""
                };
                summary
                    .actions
                    .push(format!("{} ➕ {}{}", label, preview, suffix));
                summary.memory_changed = true;
            } else {
                summary.actions.push(format!("{} updated", label));
                summary.memory_changed = true;
            }
        } else {
            // Non-verbose mode
            if message_lower.contains("added")
                || message_lower.contains("replaced")
                || message_lower.contains("removed")
                || message_lower.contains("applied")
                || message_lower.contains("entry added")
            {
                summary.actions.push(format!("{} updated", label));
                if is_skill {
                    summary.skills_changed = true;
                } else {
                    summary.memory_changed = true;
                }
            }
        }
    }

    summary
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt_combined() {
        let prompt = build_review_prompt(true, true);
        assert!(prompt.contains("Memory"));
        assert!(prompt.contains("Skills"));
    }

    #[test]
    fn test_build_prompt_skills_only() {
        let prompt = build_review_prompt(false, true);
        assert!(prompt.contains("skill library"));
        assert!(!prompt.contains("**Memory**"));
    }

    #[test]
    fn test_build_prompt_memory_only() {
        let prompt = build_review_prompt(true, false);
        assert!(prompt.contains("saving to memory"));
    }

    #[test]
    fn test_notification_mode_from_str() {
        assert_eq!(
            "off".parse::<NotificationMode>().unwrap(),
            NotificationMode::Off
        );
        assert_eq!(
            "on".parse::<NotificationMode>().unwrap(),
            NotificationMode::On
        );
        assert_eq!(
            "verbose".parse::<NotificationMode>().unwrap(),
            NotificationMode::Verbose
        );
        assert!("invalid".parse::<NotificationMode>().is_err());
    }

    #[test]
    fn test_self_evolution_skill_nudge() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 10,
        };
        let mut state = SelfEvolutionState::new(&config);

        for _ in 0..4 {
            state.bump_skill_counter();
        }
        assert!(!state.should_review_skills());

        state.bump_skill_counter();
        assert!(state.should_review_skills());
    }

    #[test]
    fn test_self_evolution_skill_manage_resets() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 5,
        };
        let mut state = SelfEvolutionState::new(&config);

        for _ in 0..10 {
            state.bump_skill_counter();
        }
        assert!(state.should_review_skills());

        state.reset_skill_counter();
        assert!(!state.should_review_skills());
        assert_eq!(state.iters_since_skill, 0);
    }

    #[test]
    fn test_self_evolution_disabled_when_zero() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 0,
            memory_review_interval: 0,
        };
        let state = SelfEvolutionState::new(&config);
        assert!(!state.should_review_skills());
        assert!(!state.should_review_memory());
    }

    #[test]
    fn test_memory_review_counter_bump_and_trigger() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 10,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        state.bump_memory_counter();
        assert!(!state.should_review_memory());

        state.bump_memory_counter();
        assert!(!state.should_review_memory());

        state.bump_memory_counter();
        assert!(state.should_review_memory());
    }

    #[test]
    fn test_memory_review_counter_reset() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 10,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        for _ in 0..3 {
            state.bump_memory_counter();
        }
        assert!(state.should_review_memory());

        state.reset_memory_counter();
        assert!(!state.should_review_memory());
        assert_eq!(state.turns_since_memory_review, 0);
    }

    #[test]
    fn test_digest_history_short_conversation() {
        use crate::client::Message;
        let messages = vec![Message::user("hello"), Message::assistant("hi there")];
        let result = digest_history(&messages, 10);
        // Conversation is shorter than tail — return as-is
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_digest_history_long_conversation() {
        use crate::client::Message;
        let mut messages: Vec<Message> = Vec::new();
        for i in 0..20 {
            messages.push(Message::user(format!("msg {}", i)));
            messages.push(Message::assistant(format!("reply {}", i)));
        }
        let result = digest_history(&messages, 6);
        // Should have 1 digest + 6 recent messages = 7
        assert!(result.len() <= 8);
        // First message should be the digest
        assert!(result[0].content.contains("Earlier conversation digest"));
    }

    #[test]
    fn test_summarize_review_actions_empty() {
        let summary = summarize_review_actions(&[], &[], NotificationMode::On);
        assert!(summary.actions.is_empty());
        assert!(!summary.skills_changed);
        assert!(!summary.memory_changed);
    }

    #[test]
    fn test_summarize_review_actions_off_mode() {
        let review = vec![r#"{"role":"tool","tool_call_id":"tc1","content":"{\"success\":true,\"message\":\"Updated skill web-search\"}"}"#.to_string()];
        let summary = summarize_review_actions(&review, &[], NotificationMode::Off);
        assert!(summary.actions.is_empty());
    }

    #[test]
    fn test_summarize_review_actions_skill() {
        let review = vec![r#"{"role":"tool","tool_call_id":"tc1","content":"{\"success\":true,\"message\":\"Updated skill web-search\"}"}"#.to_string()];
        let summary = summarize_review_actions(&review, &[], NotificationMode::On);
        assert_eq!(summary.actions.len(), 1);
        assert!(summary.skills_changed);
        assert!(!summary.memory_changed);
    }

    #[test]
    fn test_summarize_review_actions_skips_prior() {
        let review = vec![r#"{"role":"tool","tool_call_id":"tc1","content":"{\"success\":true,\"message\":\"Saved memory\"}"}"#.to_string()];
        let prior = vec![r#"{"role":"tool","tool_call_id":"tc1"}"#.to_string()];
        let summary = summarize_review_actions(&review, &prior, NotificationMode::On);
        assert!(summary.actions.is_empty());
    }

    #[test]
    fn test_persist_counters_roundtrip() {
        let config = BackgroundReviewConfig {
            skill_nudge_interval: 5,
            memory_review_interval: 3,
        };
        let mut state = SelfEvolutionState::new(&config);

        // Bump counters
        state.bump_memory_counter();
        state.bump_memory_counter();
        state.bump_skill_counter();

        // Persist
        let pairs = state.persist_counters();
        assert_eq!(pairs.len(), 2);

        // Convert to HashMap (simulates what Database would store/retrieve)
        let metadata: std::collections::HashMap<String, String> =
            pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();

        // Hydrate into a fresh state
        let mut state2 = SelfEvolutionState::new(&config);
        state2.hydrate_from_metadata(&metadata);

        assert_eq!(state2.turns_since_memory_review, 2);
        assert_eq!(state2.iters_since_skill, 1);
    }
}
