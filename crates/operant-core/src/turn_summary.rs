//! Per-turn accounting line — hermes `agent/turn_summary.py` parity (R5).
//!
//! Two display-only pieces:
//!
//! * [`TurnSummaryCollector`] — a tiny observer that tallies what a turn
//!   actually did from the tool event feed. It holds **no** agent-loop
//!   state: the display layer already sees every tool call, so nothing new
//!   is threaded through the conversation loop.
//! * [`format_turn_summary`] — a pure formatter that turns a tally plus a
//!   wall-clock duration into one dim line, e.g.::
//!
//!     ⋯ 12.4s · edited 2 files +18 -3 · read 4 files · ran 3 commands
//!
//! Ported from Claude Code's post-turn accounting line.
//!
//! Everything in this module is pure/side-effect free apart from the
//! collector's own counters, which makes it directly unit-testable.

/// Leading glyph for the summary line — terminal chrome, not agent speech.
pub const SUMMARY_PREFIX: &str = "⋯";

/// A turn that called no tools and finished this fast has nothing worth
/// reporting (plain chat reply). Below the threshold the formatter returns "".
const MIN_TOOLLESS_SECONDS: f64 = 2.0;

/// Max number of "verb + count" segments rendered before collapsing the rest
/// into a "+N more" tail, so a 12-tool turn cannot blow past one line.
const MAX_SEGMENTS: usize = 4;

/// (verb, singular noun, plural noun) for curated tool groups.
/// Tools not listed fall into a generic "called N tools" bucket.
const VERB_GROUPS: &[(&str, &str, &str, &str)] = &[
    ("file_write", "edited", "file", "files"),
    ("file_edit", "edited", "file", "files"),
    ("patch", "edited", "file", "files"),
    ("aft_write", "edited", "file", "files"),
    ("aft_edit", "edited", "file", "files"),
    ("aft_apply_patch", "edited", "file", "files"),
    ("file_read", "read", "file", "files"),
    ("aft_read", "read", "file", "files"),
    ("web_fetch", "read", "page", "pages"),
    ("web_extract", "read", "page", "pages"),
    ("terminal", "ran", "command", "commands"),
    ("aft_bash", "ran", "command", "commands"),
    ("code_execution", "ran", "script", "scripts"),
    ("file_search", "searched", "path", "paths"),
    ("web_search", "searched the web", "time", "times"),
    ("session_search", "searched sessions", "time", "times"),
    ("browser_navigate", "browsed", "page", "pages"),
    ("skill_view", "read", "skill", "skills"),
    ("skill_manage", "updated", "skill", "skills"),
    ("skills_list", "listed skills", "time", "times"),
    ("todo", "updated", "task list", "task lists"),
    ("delegate_task", "delegated", "task", "tasks"),
    ("memory_store", "updated", "memory", "memories"),
    ("memory_save", "updated", "memory", "memories"),
];

/// Render order: edits first, then reads, then commands. Anything else
/// follows in first-seen order.
const VERB_PRIORITY: &[&str] = &["edited", "read", "ran"];

/// Tools whose results may report a unified diff we can count lines from.
const DIFF_RESULT_TOOLS: &[&str] = &["patch"];

/// A counted noun group: singular label, plural label, count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NounGroup {
    pub singular: String,
    pub plural: String,
    pub count: usize,
}

/// What a single turn did, as observed from the tool event feed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TurnTally {
    /// verb -> noun groups; preserves insertion order.
    pub verbs: Vec<(String, Vec<NounGroup>)>,
    /// Tools with no curated verb, counted together.
    pub other_tools: usize,
    /// Aggregated unified-diff line deltas across edit tools, when reported.
    pub lines_added: usize,
    pub lines_removed: usize,
    /// True once at least one edit tool reported a countable diff, so the
    /// formatter knows the difference between "+0 -0" and "unknown".
    pub has_line_deltas: bool,
}

impl TurnTally {
    pub fn total_tools(&self) -> usize {
        let counted: usize = self
            .verbs
            .iter()
            .map(|(_, nouns)| nouns.iter().map(|n| n.count).sum::<usize>())
            .sum();
        counted + self.other_tools
    }
}

/// Count added/removed lines in unified-diff text. File headers (`+++`/`---`)
/// are excluded so a one-line edit does not read as three additions.
fn count_diff_lines(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            // Ignore pure hunk separators like "+" (blank added line still
            // counts; "+++" headers already excluded above).
            added += usize::from(!rest.trim().is_empty());
        } else if let Some(rest) = line.strip_prefix('-') {
            removed += usize::from(!rest.trim().is_empty());
        }
    }
    (added, removed)
}

/// Pull (added, removed) from a tool result, or None when unavailable.
fn extract_line_deltas(tool_name: &str, result_content: &str) -> Option<(usize, usize)> {
    if !DIFF_RESULT_TOOLS.contains(&tool_name) {
        return None;
    }
    let text = result_content.trim();
    if !text.starts_with('{') {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_str(text).ok()?;
    let diff = payload.get("diff")?.as_str()?;
    if diff.trim().is_empty() {
        return None;
    }
    let (added, removed) = count_diff_lines(diff);
    if added == 0 && removed == 0 {
        return None;
    }
    Some((added, removed))
}

/// Accumulate per-turn tool tallies from the tool event feed.
#[derive(Debug, Default, Clone)]
pub struct TurnSummaryCollector {
    tally: TurnTally,
}

impl TurnSummaryCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a fresh turn (drops any prior tally).
    pub fn begin(&mut self) {
        self.tally = TurnTally::default();
    }

    /// Record a completed tool call by name + raw result content.
    pub fn record_tool(&mut self, tool_name: &str, result_content: &str) {
        if tool_name.is_empty() {
            return;
        }
        if let Some((_, verb, noun_singular, noun_plural)) = VERB_GROUPS
            .iter()
            .find(|(name, _, _, _)| *name == tool_name)
        {
            let verb = (*verb).to_string();
            let singular = (*noun_singular).to_string();
            let plural = (*noun_plural).to_string();
            let mut found = false;
            for (existing_verb, nouns) in self.tally.verbs.iter_mut() {
                if existing_verb == &verb {
                    if let Some(group) = nouns.iter_mut().find(|n| n.plural == plural) {
                        group.count += 1;
                        found = true;
                    } else {
                        nouns.push(NounGroup {
                            singular: singular.clone(),
                            plural: plural.clone(),
                            count: 1,
                        });
                        found = true;
                    }
                    break;
                }
            }
            if !found {
                self.tally.verbs.push((
                    verb.clone(),
                    vec![NounGroup {
                        singular: singular.clone(),
                        plural: plural.clone(),
                        count: 1,
                    }],
                ));
            }
            if verb == "edited"
                && let Some((added, removed)) = extract_line_deltas(tool_name, result_content)
            {
                self.tally.lines_added += added;
                self.tally.lines_removed += removed;
                self.tally.has_line_deltas = true;
            }
        } else {
            self.tally.other_tools += 1;
        }
    }

    pub fn tally(&self) -> &TurnTally {
        &self.tally
    }
}

/// Format a wall-clock duration as `12.4s`, `2m 05s`, or `1h 02m`.
pub fn format_elapsed(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else if seconds < 3600.0 {
        let m = (seconds / 60.0).floor() as u64;
        let s = (seconds % 60.0) as u64;
        format!("{m}m {s:02}s")
    } else {
        let h = (seconds / 3600.0).floor() as u64;
        let m = ((seconds % 3600.0) / 60.0) as u64;
        format!("{h}h {m:02}m")
    }
}

/// Format a cumulative token count as `1.2k tok`, `890 tok`.
pub fn format_token_flow(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M tok", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k tok", tokens as f64 / 1_000.0)
    } else {
        format!("{tokens} tok")
    }
}

/// Render the one-line turn summary, or "" when nothing worth reporting.
pub fn format_turn_summary(tally: &TurnTally, elapsed_seconds: f64) -> String {
    if tally.total_tools() == 0 && elapsed_seconds < MIN_TOOLLESS_SECONDS {
        return String::new();
    }

    let mut segments: Vec<String> = Vec::new();

    // Ordered verbs first: edited, read, ran.
    let mut ordered: Vec<&(String, Vec<NounGroup>)> = Vec::new();
    for priority in VERB_PRIORITY {
        if let Some(entry) = tally.verbs.iter().find(|(v, _)| v == priority) {
            ordered.push(entry);
        }
    }
    for entry in &tally.verbs {
        if !VERB_PRIORITY.contains(&entry.0.as_str()) {
            ordered.push(entry);
        }
    }

    for (verb, nouns) in ordered.iter().take(MAX_SEGMENTS) {
        let total: usize = nouns.iter().map(|n| n.count).sum();
        let noun = if total == 1 {
            nouns.first().map(|n| n.singular.as_str()).unwrap_or("item")
        } else {
            // Use the plural of the most common noun group, or the first.
            nouns.first().map(|n| n.plural.as_str()).unwrap_or("items")
        };
        if verb == "edited" && tally.has_line_deltas {
            segments.push(format!(
                "{verb} {total} {noun} +{} -{}",
                tally.lines_added, tally.lines_removed
            ));
        } else {
            segments.push(format!("{verb} {total} {noun}"));
        }
    }

    let mut extra = 0usize;
    let shown: usize = ordered.len();
    if shown < tally.verbs.len() {
        extra = tally.verbs.len() - shown;
    }
    if tally.other_tools > 0 {
        extra += tally.other_tools;
    }
    if extra > 0 {
        segments.push(format!("+{extra} more"));
    }

    if segments.is_empty() {
        return String::new();
    }

    format!(
        "{} {} · {}",
        SUMMARY_PREFIX,
        format_elapsed(elapsed_seconds),
        segments.join(" · ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_counts_curated_and_other() {
        let mut c = TurnSummaryCollector::new();
        c.record_tool("file_read", "{}");
        c.record_tool("file_read", "{}");
        c.record_tool("terminal", "{}");
        c.record_tool("some_mcp_tool", "{}");
        let t = c.tally();
        assert_eq!(t.total_tools(), 4);
        assert_eq!(t.other_tools, 1);
    }

    #[test]
    fn format_line_has_expected_shape() {
        let mut c = TurnSummaryCollector::new();
        // `patch` is the diff-bearing edit tool (hermes _DIFF_RESULT_TOOLS
        // parity) — its result payload carries the unified diff.
        c.record_tool("patch", r#"{"diff":"@@\n+added\n-removed\n"}"#);
        c.record_tool("file_read", "{}");
        c.record_tool("terminal", "{}");
        let line = format_turn_summary(c.tally(), 12.4);
        assert!(line.starts_with("⋯ "), "got: {line}");
        assert!(line.contains("edited 1 file +1 -1"), "got: {line}");
        assert!(line.contains("read 1 file"), "got: {line}");
        assert!(line.contains("ran 1 command"), "got: {line}");
        assert!(line.contains("12.4s"), "got: {line}");
    }

    #[test]
    fn toolless_fast_turn_is_empty() {
        let c = TurnSummaryCollector::new();
        assert_eq!(format_turn_summary(c.tally(), 0.5), "");
    }

    #[test]
    fn diff_headers_not_counted() {
        let (a, r) = count_diff_lines("--- a/x\n+++ b/x\n@@\n+one\n-two\n");
        assert_eq!((a, r), (1, 1));
    }

    #[test]
    fn many_tools_collapse_into_more_tail() {
        let mut c = TurnSummaryCollector::new();
        for i in 0..6 {
            c.record_tool(&format!("tool_{i}"), "{}");
        }
        let line = format_turn_summary(c.tally(), 10.0);
        assert!(line.contains("+6 more"), "got: {line}");
    }

    #[test]
    fn token_flow_formatting() {
        assert_eq!(format_token_flow(890), "890 tok");
        assert_eq!(format_token_flow(1234), "1.2k tok");
    }

    #[test]
    fn elapsed_formatting() {
        assert_eq!(format_elapsed(12.4), "12.4s");
        assert_eq!(format_elapsed(125.0), "2m 05s");
    }
}
