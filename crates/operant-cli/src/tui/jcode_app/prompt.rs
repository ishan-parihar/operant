// Vendored from jcode (crates/jcode-base/src/prompt.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// see jcode_app/mod.rs for scope.
//! Included: SWARM_EFFORT (:101), SWARM_DEEP_EFFORT (:110), is_swarm_effort
//! (:123), is_deep_swarm_effort (:129), swarm_root_reasoning_effort (:136),
//! ContextInfo (:285, plus the impl members the renderers call).
//! [port-excision] the rest of prompt.rs (directive text, onboarding) is not
//! ported.

/// Reasoning-effort sentinel that enables swarm orchestration. Providers
/// translate this to the configured root effort (maximum by default) when
/// building API requests, while the UI/session keep the literal `swarm` marker
/// so the agent knows to
/// inject [`SWARM_EFFORT_DIRECTIVE`].
pub const SWARM_EFFORT: &str = "swarm";

/// Reasoning-effort sentinel for the **deep task graph** mode: configured root
/// reasoning AND the comprehensive DAG-first swarm workflow (decompose into a
/// validated task graph, critique/verify gates, typed artifact handoffs). Sits
/// one rung above [`SWARM_EFFORT`] on the effort ladder: `... xhigh`, `swarm`
/// (light fan-out), `swarm-deep` (deep task graph). Providers translate this to
/// the configured root effort, while the UI/session keep the literal marker so
/// the agent knows to inject [`SWARM_DEEP_EFFORT_DIRECTIVE`].
pub const SWARM_DEEP_EFFORT: &str = "swarm-deep";

/// System-prompt directive injected when the active reasoning effort is
/// [`SWARM_EFFORT`]. Instructs the agent to lean on the swarm tooling.
pub const SWARM_EFFORT_DIRECTIVE: &str = "# Swarm Effort\n\nSwarm orchestration is enabled. Your root reasoning effort is configured independently from worker effort. For any non-trivial task, decompose the work and use the `swarm` tool to spawn and coordinate parallel agents (spawn workers with concrete prompts, assign tasks, and collect their reports) instead of doing everything yourself in one thread. Prefer parallelizing independent subtasks across swarm members, and use a coordinator/plan when the work has multiple stages. Only skip the swarm for trivial, single-step requests.";

/// System-prompt directive injected when the active reasoning effort is
/// [`SWARM_DEEP_EFFORT`]. Instructs the agent to run the comprehensive DAG-first
/// task-graph workflow.
pub const SWARM_DEEP_EFFORT_DIRECTIVE: &str = "# Deep Task Graph\n\nThe deep task-graph swarm workflow is enabled. Your root reasoning effort is configured independently from worker effort. Treat the task DAG as the primary object, not ad hoc agent chat. Workflow:\n\n1. Seed a graph with `swarm task_graph` using `mode: \"deep\"`: lay out nodes (kind explore|implement|verify|fix|synthesize) and `depends_on` edges instead of answering directly. (At this effort the server already defaults the plan to deep, but pass `mode: \"deep\"` explicitly anyway.) The engine auto-inserts a plan-wide root gate over your seed: the plan cannot finish until a final adversarial audit passes, and that audit can inject new top-level work.\n2. For any node that is too big, `swarm expand_node` to decompose it into a child sub-DAG (you become its planner/integrator). In deep mode a critique/verify gate is auto-inserted before a composite node can close. The graph is EXPECTED to outgrow its seed, often by several times: growth (expansions and gate-injected gaps) is the system working, not scope creep. plan_status reports seeded-vs-grown counts.\n3. Finish each node with `swarm complete_node` and a typed artifact: `findings`, `evidence` (file:line / commit refs), `validation`, `open_questions`, a required `confidence` (low|medium|high; report low honestly, it routes follow-up work to shore up that scope), and an honest `what_i_did_not_check`. Downstream nodes are hydrated with these artifacts automatically. There is no other way to close a deep node: a turn ending without expand_node/complete_node re-queues the node to a fresh worker and fails it on repeat.\n4. When a critique/verify gate finds gaps or failures, use `swarm inject_gap` to add new nodes; the parent cannot close until they drain. A passing gate artifact must account for EVERY node it audited by id (the server rejects rubber stamps), and cannot pass over a low-confidence sibling without addressing it explicitly, so treat low-confidence siblings as priority probe targets.\n5. Use `swarm run_plan` to drive the graph to completion. It returns immediately and drives the plan as a background task (progress card + wake on completion), so keep working or answer the user while it runs; check `swarm plan_status` or `bg` for progress. Deep mode fans out wide automatically (many workers run in parallel, bounded only by the swarm member cap), so prefer decomposing into MANY independent sibling nodes rather than a few serial ones: keep the ready set wide so run_plan can dispatch lots of agents at once. Only add `depends_on` edges for real data dependencies.\n\nComprehensiveness is structural: prefer decomposition + gates over a single thorough answer, so it is very unlikely any nook or cranny is missed.";

/// Returns true when `effort` is either swarm sentinel (light or deep),
/// case-insensitive. Providers resolve their configured root reasoning level.
pub fn is_swarm_effort(effort: &str) -> bool {
    let trimmed = effort.trim();
    trimmed.eq_ignore_ascii_case(SWARM_EFFORT) || trimmed.eq_ignore_ascii_case(SWARM_DEEP_EFFORT)
}

// [port-decision] dedup: prompt.rs defined the three swarm-effort consts and
// is_swarm_effort twice across concatenated sources; kept the first (identical)
// copies at the top of the file.

/// Returns true when `effort` is specifically the deep task-graph sentinel.
pub fn is_deep_swarm_effort(effort: &str) -> bool {
    effort.trim().eq_ignore_ascii_case(SWARM_DEEP_EFFORT)
}


/// Configured root reasoning level for an orchestration sentinel. Providers
/// translate this real level to their supported range while retaining the
/// sentinel in session state. Ordinary reasoning efforts are left untouched.
pub fn swarm_root_reasoning_effort(effort: &str) -> Option<&'static str> {
    if !is_swarm_effort(effort) {
        return None;
    }
    Some(
        crate::tui::jcode_app::config_shim::config()
            .agents
            .root_effort_for_swarm(is_deep_swarm_effort(effort)),
    )
}


/// Information about what's loaded in the context window
#[derive(Debug, Clone, Default)]
pub struct ContextInfo {
    // === Static (System Prompt) ===
    /// Base system prompt size (chars)
    pub system_prompt_chars: usize,
    /// Immutable session context size (chars), when persisted in transcript history.
    pub session_context_chars: usize,
    /// Whether project AGENTS.md was loaded
    pub has_project_agents_md: bool,
    /// Project AGENTS.md size (chars)
    pub project_agents_md_chars: usize,
    /// Whether global ~/AGENTS.md was loaded
    pub has_global_agents_md: bool,
    /// Global AGENTS.md size (chars)
    pub global_agents_md_chars: usize,
    /// Skills section size (chars)
    pub skills_chars: usize,
    /// Self-dev section size (chars)
    pub selfdev_chars: usize,
    /// Memory section size (chars)
    pub memory_chars: usize,
    /// Prompt overlay section size (chars)
    pub prompt_overlay_chars: usize,
    /// Preferred tools section size (chars)
    pub preferred_tools_chars: usize,
    // === Dynamic (Conversation) ===
    /// Tool definitions sent to API (chars)
    pub tool_defs_chars: usize,
    /// Number of tool definitions
    pub tool_defs_count: usize,
    /// User messages total size (chars)
    pub user_messages_chars: usize,
    /// Number of user messages
    pub user_messages_count: usize,
    /// Assistant messages total size (chars)
    pub assistant_messages_chars: usize,
    /// Number of assistant messages
    pub assistant_messages_count: usize,
    /// Tool calls size (chars)
    pub tool_calls_chars: usize,
    /// Number of tool calls
    pub tool_calls_count: usize,
    /// Tool results size (chars)
    pub tool_results_chars: usize,
    /// Number of tool results
    pub tool_results_count: usize,

    /// Total system prompt size (chars)
    pub total_chars: usize,
}


impl ContextInfo {
    /// Rough estimate of tokens (chars / 4 is a common approximation)
    pub fn estimated_tokens(&self) -> usize {
        self.total_chars / 4
    }

    pub fn prompt_prefix_chars(&self) -> usize {
        self.system_prompt_chars
            + self.session_context_chars
            + self.project_agents_md_chars
            + self.global_agents_md_chars
            + self.skills_chars
            + self.selfdev_chars
            + self.memory_chars
            + self.prompt_overlay_chars
            + self.preferred_tools_chars
            + self.tool_defs_chars
    }

    pub fn prompt_prefix_tokens(&self) -> usize {
        self.prompt_prefix_chars() / 4
    }

    pub fn tool_definition_tokens(&self) -> usize {
        self.tool_defs_chars / 4
    }

    /// Get breakdown as (label, chars, icon) tuples for display
    pub fn breakdown(&self) -> Vec<(&'static str, usize, &'static str)> {
        let mut parts = vec![
            ("sys", self.system_prompt_chars, "⚙"),
            ("session", self.session_context_chars, "🌍"),
        ];
        if self.has_project_agents_md {
            parts.push(("agents", self.project_agents_md_chars, "📋"));
        }
        if self.has_global_agents_md {
            parts.push(("~agents", self.global_agents_md_chars, "📋"));
        }
        if self.skills_chars > 0 {
            parts.push(("skills", self.skills_chars, "🔧"));
        }
        if self.selfdev_chars > 0 {
            parts.push(("dev", self.selfdev_chars, "🛠"));
        }
        if self.memory_chars > 0 {
            parts.push(("mem", self.memory_chars, "🧠"));
        }
        if self.prompt_overlay_chars > 0 {
            parts.push(("overlay", self.prompt_overlay_chars, "🧩"));
        }
        if self.preferred_tools_chars > 0 {
            parts.push(("tools", self.preferred_tools_chars, "🧰"));
        }
        parts
    }
}

