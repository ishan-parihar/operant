//! Turn-triggered materialisation of deferred (MCP) tools.
//!
//! [`crate::tools::tool_search::assemble_tools`] already hides `mcp_*` tools
//! behind the `tool_search` / `tool_describe` / `tool_call` bridge. That
//! bridge is *model*-driven: a tool the model never thinks to search for
//! stays invisible for the whole session. This module adds a *turn*-driven
//! path — when the current user turn overlaps a deferred tool's name or
//! description, that tool is re-injected into the model-visible tools array
//! for the request.
//!
//! # Exactly what the trigger is
//!
//! A **lexical overlap heuristic**. A deferred tool materialises when at
//! least [`MIN_OVERLAP`] *distinct* content tokens (length >= 3, case
//! folded, alphanumeric runs) of the turn text also appear in the tool's
//! `name` or `description`. No embedding, no model call, no scoring, no
//! ranking. First match wins; ties are irrelevant because a match is
//! binary.
//!
//! # Failure modes
//!
//! 1. **Miss (the damaging one).** A turn that needs
//!    `mcp_github_create_pull_request` but never says "github", "create" or
//!    "pull" leaves the tool hidden. This is worse than merely listing it:
//!    a listed tool the model ignores costs tokens, a hidden one it cannot
//!    know about costs capability. Two things bound the damage: the
//!    heuristic is re-evaluated on *every* request (so a later turn that
//!    does match still materialises it), and a never-matching tool is
//!    still fully discoverable through the bridge — `tool_search` with an
//!    empty query lists every deferred tool, and `operant tools list`
//!    prints the whole registry. A tool that matches nothing is *listed,
//!    not hidden*: the bridge listing is the safety net.
//! 2. **Over-match.** Common words ("file", "search", "list", "create")
//!    materialise more tools than strictly needed. Costs schema tokens,
//!    never causes a wrong call.
//! 3. **Not sticky.** Materialisation is per-request, not per-session. A
//!    tool matched on turn 1 is hidden again on turn 2 unless turn 2 also
//!    matches. Deliberate: cross-turn sticky state is state that can go
//!    stale (a disconnected server's tools would linger), and the bridge
//!    still lists everything, so there is no way to strand a tool.
//!
//! # Deferred vs unavailable
//!
//! These are different states and this module only ever touches the first.
//! *Unavailable* is decided upstream in `ToolRegistry::get_schemas`
//! (registered AND not disabled AND `is_available()`), so an unavailable
//! tool never reaches this catalog at all and cannot be materialised.
//! *Deferred* is a presentation decision made here: the tool is live and
//! callable, just not described to the model. Conflating the two would let
//! a banned tool back into the schema, which is why `materialise` is only
//! ever called with the already-filtered `get_schemas()` output.

use std::collections::HashSet;

use super::is_deferrable;
use crate::schema::ToolSchema;

/// Distinct turn tokens that must appear in a deferred tool's name or
/// description for it to materialise.
pub const MIN_OVERLAP: usize = 1;

/// Tokens shorter than this are ignored. The bound is 4, not 3, and that
/// matters: three-letter English is dominated by function words, and MCP
/// descriptions are saturated with them — "on the connected X server" put
/// `the` in EVERY tool's description, so a length-3 floor made every
/// server tool match every turn. Length 4 drops `the`/`and`/`for`/`not`
/// while keeping the words that carry meaning (`list`, `read`, `issue`).
const MIN_TOKEN_LEN: usize = 4;

/// Upper bound on the characters scanned per tool, so one pathological
/// multi-kilobyte MCP description cannot dominate the per-request scan.
const MAX_SCAN_CHARS: usize = 2048;

/// Cheap token estimate from char count (~4 chars/token), matching
/// `tool_search`'s own estimator so the two modules agree on cost.
const CHARS_PER_TOKEN: f64 = 4.0;

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Distinct case-folded content tokens of length >= [`MIN_TOKEN_LEN`].
fn content_tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| !is_token_char(c))
        .filter(|t| t.chars().count() >= MIN_TOKEN_LEN)
        .map(|t| t.to_lowercase())
        .collect()
}

/// Number of *distinct* turn tokens present in the tool's name or
/// description. Exposed for tests and for callers that want a different
/// cut-off than [`MIN_OVERLAP`].
pub fn relevance(turn: &str, tool: &ToolSchema) -> usize {
    relevance_tokens(turn, tool).len()
}

fn relevance_tokens(turn: &str, tool: &ToolSchema) -> HashSet<String> {
    let turn_tokens = content_tokens(turn);
    if turn_tokens.is_empty() {
        return HashSet::new();
    }
    let hay: String = format!("{} {}", tool.name, tool.description)
        .chars()
        .take(MAX_SCAN_CHARS)
        .collect::<String>()
        .to_lowercase();
    hay.split(|c: char| !is_token_char(c))
        .filter(|t| t.chars().count() >= MIN_TOKEN_LEN)
        .filter(|t| turn_tokens.contains(*t))
        .map(|t| t.to_string())
        .collect()
}

/// Whether this deferred tool should be materialised for this turn.
pub fn is_relevant(turn: &str, tool: &ToolSchema) -> bool {
    relevance(turn, tool) >= MIN_OVERLAP
}

/// The deferred tools in `catalog` that this turn makes relevant.
///
/// `catalog` must be the output of `ToolRegistry::get_schemas()` (i.e.
/// already filtered to registered AND available AND not-disabled); the
/// `is_deferrable` filter is applied again here so the function is safe to
/// call with a raw catalog.
pub fn materialise_for_turn<'a>(turn: &str, catalog: &'a [ToolSchema]) -> Vec<&'a ToolSchema> {
    if turn.trim().is_empty() {
        return Vec::new();
    }
    catalog
        .iter()
        .filter(|t| is_deferrable(&t.name) && is_relevant(turn, t))
        .collect()
}

/// Inject every deferred tool this turn makes relevant into `visible`.
///
/// Idempotent: a tool whose name is already present in `visible` is never
/// added again, so `materialise(turn, materialise(turn, base, cat), cat)`
/// equals `materialise(turn, base, cat)` — both in content and order.
/// `visible` is returned untouched when the turn is empty or nothing
/// matches.
pub fn materialise(
    turn: &str,
    mut visible: Vec<ToolSchema>,
    catalog: &[ToolSchema],
) -> Vec<ToolSchema> {
    for tool in materialise_for_turn(turn, catalog) {
        if visible.iter().any(|s| s.name == tool.name) {
            continue;
        }
        visible.push(tool.clone());
    }
    visible
}

/// Rough token cost of a schema list (name + description + serialised
/// parameters), for reporting the size effect of deferral. Same estimator
/// as the bridge listing.
pub fn estimate_schema_tokens(schemas: &[ToolSchema]) -> usize {
    let chars: usize = schemas
        .iter()
        .map(|s| {
            s.name.chars().count()
                + s.description.chars().count()
                + s.parameters.to_string().chars().count()
        })
        .sum();
    (chars as f64 / CHARS_PER_TOKEN).ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(name: &str, description: &str) -> ToolSchema {
        ToolSchema::new(name, description, json!({ "type": "object" }))
    }

    fn names(schemas: &[ToolSchema]) -> Vec<&str> {
        schemas.iter().map(|s| s.name.as_str()).collect()
    }

    /// The bridge's own output for a catalog of one native + two MCP tools:
    /// native tool plus the three bridge tools, MCP tools hidden.
    fn assembled() -> Vec<ToolSchema> {
        vec![
            schema("terminal", "Run a shell command"),
            schema("tool_search", "Search deferred tools"),
            schema("tool_describe", "Describe a deferred tool"),
            schema("tool_call", "Call a deferred tool"),
        ]
    }

    fn catalog() -> Vec<ToolSchema> {
        vec![
            schema("terminal", "Run a shell command"),
            schema(
                "mcp_github_create_pull_request",
                "Open a new pull request on a GitHub repository",
            ),
            schema(
                "mcp_linear_search_issues",
                "Search Linear issues by text, project or assignee",
            ),
        ]
    }

    #[test]
    fn deferred_tools_should_not_appear_in_schema_until_materialised() {
        let cat = catalog();
        let base = assembled();

        // Unrelated turn: nothing materialises, so the model-visible list is
        // exactly the bridge output and no mcp_* name is present.
        let out = materialise("what is the capital of France?", base.clone(), &cat);
        assert!(names(&out).iter().all(|n| !n.starts_with("mcp_")));
        assert_eq!(names(&out), names(&base));

        // Relevant turn: the matching deferred tool appears.
        let out = materialise("please open a pull request on the repo", base, &cat);
        assert!(names(&out).contains(&"mcp_github_create_pull_request"));
        // The non-matching deferred tool is still hidden.
        assert!(!names(&out).contains(&"mcp_linear_search_issues"));
    }

    #[test]
    fn deferred_materialisation_should_be_idempotent() {
        let cat = catalog();
        let turn = "search linear issues for the flaky test";
        let once = materialise(turn, assembled(), &cat);
        let twice = materialise(turn, once.clone(), &cat);
        let thrice = materialise(turn, twice.clone(), &cat);

        assert_eq!(names(&once), names(&twice));
        assert_eq!(names(&once), names(&thrice));
        // No name appears twice.
        let mut seen = HashSet::new();
        for n in names(&once) {
            assert!(seen.insert(n), "duplicate tool name in schema: {n}");
        }
        assert!(names(&once).contains(&"mcp_linear_search_issues"));
    }

    #[test]
    fn deferred_tool_should_materialise_on_a_relevant_turn() {
        let cat = catalog();
        let tool = cat
            .iter()
            .find(|s| s.name == "mcp_linear_search_issues")
            .expect("fixture tool");
        assert!(is_relevant("search my linear issues please", tool));
        // Overlap on a description word counts too.
        assert!(is_relevant("find the assignee for this ticket", tool));
        // No overlap at all.
        assert!(!is_relevant("format the disk", tool));
        // An empty turn never materialises anything.
        assert!(!is_relevant("", tool));
        assert_eq!(
            relevance("search my linear issues please", tool),
            3, // "search" + "linear" + "issues"
            "relevance must count DISTINCT overlapping tokens"
        );
    }

    #[test]
    fn native_tools_are_never_materialised() {
        let cat = catalog();
        // "terminal" matches the turn hard but is not deferrable.
        let out = materialise("run terminal", assembled(), &cat);
        assert_eq!(
            out.iter().filter(|s| s.name == "terminal").count(),
            1,
            "native tool must not be injected a second time"
        );
        assert!(!is_deferrable("terminal"));
    }

    /// Honest measurement of the size effect, and an honest record of the
    /// heuristic's weakness.
    ///
    /// A realistic 40-tool MCP catalog (4 servers x 6 operations) plus 20
    /// native tools. The deferred-vs-eager gap is large — the bridge alone
    /// cuts the model-visible list to 4 entries — but a `MIN_OVERLAP` of 1 is
    /// COARSE: every operation of every server shares the words "issues" and
    /// "search", so a short generic turn matches all 24 server tools. That is
    /// failure mode #2 (over-match) at its worst, and it is why the measured
    /// benefit of *materialisation* on a short generic turn is close to zero:
    /// it hands back most of what deferral hid.
    ///
    /// What materialisation does buy is correctness on turns carrying a
    /// DISTINCTIVE token. Both numbers are asserted so the tradeoff is
    /// recorded in the repo, not only in a changelog.
    #[test]
    fn schema_size_reduction_is_measurable() {
        let mut cat: Vec<ToolSchema> = (0..20)
            .map(|i| {
                schema(
                    &format!("file_tools_read_{i}"),
                    &format!(
                        "Read file number {i} from the workspace and return its contents verbatim"
                    ),
                )
            })
            .collect();
        for (server, domain) in [
            ("github", "repositories, pull requests and issues"),
            ("linear", "issues, projects and cycles"),
            ("sentry", "errors, releases and performance transactions"),
            ("postgres", "tables, rows and query results"),
        ] {
            for op in ["list", "create", "update", "delete", "search", "get"] {
                cat.push(schema(
                    &format!("mcp_{server}_{op}"),
                    &format!("{op} {domain} on the connected {server} server"),
                ));
            }
        }
        let base = assembled();
        assert_eq!(base.len(), 4, "baseline is the bridge only");

        // Eager — no deferral at all: every tool described to the model.
        let eager_tokens = estimate_schema_tokens(&cat);

        // A DISTINCTIVE turn: only the sentry server shares "sentry".
        let distinctive = materialise("check the sentry releases", base.clone(), &cat);
        let matched = materialise_for_turn("check the sentry releases", &cat);
        assert_eq!(matched.len(), 6, "one server, six operations");
        assert!(
            estimate_schema_tokens(&distinctive) < eager_tokens,
            "materialisation must always be smaller than describing everything"
        );

        // A GENERIC turn: over-match, recorded rather than wished away.
        let generic = materialise("list issues on github", base.clone(), &cat);
        let generic_matched = materialise_for_turn("list issues on github", &cat);
        assert!(
            generic_matched.len() > 12,
            "MIN_OVERLAP=1 over-matches short generic turns: measured {} of {}",
            generic_matched.len(),
            cat.len()
        );
        assert!(
            estimate_schema_tokens(&generic) < eager_tokens,
            "even the worst case stays below describing everything"
        );
        // A short turn still never re-adds the whole catalog.
        assert!(generic.len() < cat.len() + base.len());
    }
}
