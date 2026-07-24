pub mod output_styles {
    #[derive(Debug, Clone)]
    pub struct StyleInfo {
        pub name: String,
        pub label: String,
        pub description: String,
    }

    pub fn builtin_styles() -> Vec<StyleInfo> {
        vec![StyleInfo {
            name: "default".to_string(),
            label: "Default".to_string(),
            description: "Standard theme".to_string(),
        }]
    }

    pub fn find_style<'a>(styles: &'a [StyleInfo], name: &str) -> Option<&'a StyleInfo> {
        styles.iter().find(|s| s.name == name)
    }
}

/// Rotating completion verbs — shown after a turn completes ("✽ Worked for 2m 5s").
/// Varied per turn so the UI feels alive rather than mechanical.
/// (P2-15 from UX audit — was always "done".)
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

