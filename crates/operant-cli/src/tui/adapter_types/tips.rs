/// Select a rotating tip for the welcome screen. Seed-based rotation
/// so the tip changes each session but is deterministic within a session.
/// (iter-106 — was a stub returning None, so the welcome screen always
/// showed "Edit AGENTS.md" as the fallback tip.)
#[allow(dead_code)] // Prepared for welcome screen tip rotation
pub fn select_tip(seed: u64) -> Option<String> {
    const TIPS: &[&str] = &[
        "Type /help to see all commands. Try /skills, /journey, /effort.",
        "Press ? or F1 any time to toggle the help overlay.",
        "Use /model to switch models mid-session — your pick persists.",
        "Type /steer while the agent is working to redirect it in real time.",
        "Press Ctrl+A to open the model picker without typing /model.",
        "Use /skills to browse installed skills, or install one with: operant skills install <url>",
        "The agent remembers across sessions via TDG graph memory — use /journey to see what it knows.",
        "Press Ctrl+T to see active subagent tasks.",
        "Use /context to check how much of your context window is used.",
        "Type ! before a message to run it as a shell command (bash prefix mode).",
        "Use /diff to review what the agent changed in your project.",
        "Press Ctrl+B to branch the current session and explore alternatives.",
        "Use /effort to control reasoning depth: low for speed, max for hard problems.",
        "The /reasoning command toggles whether thinking blocks are expanded by default.",
        "Use /setup to re-run the configuration wizard at any time.",
        "Type /export to save the current session as JSON or Markdown.",
        "Use /voice to enable voice input (requires a microphone).",
        "Press Esc to interrupt the agent mid-stream — it stops gracefully.",
        "Use /stats to see token usage, cost, and model breakdown across sessions.",
        "Type /yolo to toggle auto-approve mode (use with care — skips all permission prompts).",
    ];
    if TIPS.is_empty() {
        return None;
    }
    let idx = (seed as usize) % TIPS.len();
    Some(TIPS[idx].to_string())
}
