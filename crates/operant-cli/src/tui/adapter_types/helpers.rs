// adapter_types/helpers.rs — UI personalization helpers.

pub fn sample_completion_verb(seed: u64) -> &'static str {
    const VERBS: &[&str] = &[
        "done",
        "finished",
        "completed",
        "wrapped up",
        "sorted",
        "nailed it",
        "shipped",
        "landed",
    ];
    VERBS[(seed as usize) % VERBS.len()]
}

/// Rotating spinner verbs — shown while the agent is thinking ("Thinking…").
/// Varied per turn so the status row feels expressive.
/// (P2-15 from UX audit — was always "thinking".)
pub fn sample_spinner_verb(seed: u64) -> &'static str {
    const VERBS: &[&str] = &[
        "thinking",
        "processing",
        "working",
        "pondering",
        "analyzing",
        "computing",
        "reasoning",
        "reflecting",
        "considering",
        "exploring",
        "investigating",
        "composing",
        "searching",
        "crafting",
    ];
    VERBS[(seed as usize) % VERBS.len()]
}

/// Model context-window lookup. (iter-115 — the query module's
/// QueryEvent/StreamEvent/UsageInfo types were deleted with the bridge
/// in iter-114. Only this function survived because it's still used by
/// refresh_context_window_size in app.rs.)
pub fn context_window_for_model(_model: &str) -> usize {
    128000
}

// (iter-211: pub mod compact {} deleted — empty module, zero callers)

// ---------- AuthStore ----------
