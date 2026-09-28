//! Bounded edit-distance recovery for hallucinated tool names.
//!
//! A model calls `read_files` when the tool is `read_file`, or a tool from
//! an MCP server that has since disconnected. Both cases surface as a bare
//! `ToolNotFound`, and the model's usual response is to call the same wrong
//! name again. This module turns that dead end into one of three honest
//! outcomes: recover onto a single near-miss, refuse because two
//! candidates are equally close, or refuse because nothing is close
//! enough.
//!
//! # The threshold, and why it is 2
//!
//! [`RECOVERY_THRESHOLD`] = 2 edits, compared case-insensitively.
//!
//! * Distance 1 covers the overwhelming majority of real errors: a dropped
//!   or doubled character, a wrong case, `read_file` -> `read_files`.
//! * Distance 2 additionally covers transpositions (`reafd` -> `readf`),
//!   which are two substitutions under plain Levenshtein but one human
//!   keystroke slip.
//! * Distance 3+ is a model inventing a tool rather than mistyping one.
//!   Recovering there means silently running an *unrelated* tool — for
//!   example recovering `read_notes` onto `write_notes` at distance 3
//!   would mutate state the model never asked for. An explicit error is
//!   strictly better than a confident wrong action.
//!
//! Distance alone is not the safety property though; the property is
//! *distance plus uniqueness*. [`recover`] only recovers when exactly one
//! registry name attains the minimum distance. Real registries are dense
//! with sibling names (`read_file` / `read_files` / `list_files`), so ties
//! are common, and an arbitrary pick between two equally close tools is
//! exactly how you get a confidently wrong action. Ties are refused and the
//! candidates are named in the error so the model can choose.
//!
//! Case-insensitivity means a wrong-case call costs 0 edits and recovers
//! with the registry's canonical spelling. Two registry names differing
//! only in case tie at distance 0 and are refused, which is the correct
//! answer for an ambiguous request.
//!
//! # Where the candidates come from
//!
//! [`resolve`] reads `ToolRegistry::get_schemas()`, i.e. the intersection
//! registered AND available AND not-disabled. Recovery therefore can never
//! resurrect a tool the user disabled or an unavailable one — the same
//! filter the schema list uses. A name belonging to a *disconnected* MCP
//! server is simply not in the set, so it returns
//! [`Recovery::NotFound`] and the error says to reconnect the server.
//! That is the honest answer: there is nothing to recover onto.

use std::fmt;

use crate::tools::ToolRegistry;

/// Maximum Levenshtein distance at which a wrong tool name is treated as a
/// near-miss worth recovering. See the module docs for the justification.
pub const RECOVERY_THRESHOLD: usize = 2;

/// Outcome of matching a requested name against the live registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// The name matched exactly. Nothing to do.
    Exact,
    /// Exactly one registry name is within [`RECOVERY_THRESHOLD`] and is
    /// strictly closer than every other candidate.
    Recovered {
        /// The registry's canonical spelling.
        canonical: String,
        /// Levenshtein distance from the requested name.
        distance: usize,
        /// Size of the candidate set that was searched.
        registry_size: usize,
        /// How many of those candidates were within the threshold.
        within_threshold: usize,
    },
    /// Two or more registry names are EQUALLY close at the minimum
    /// distance. Refused rather than guessed.
    Ambiguous {
        /// The tied-at-minimum candidates, sorted.
        candidates: Vec<String>,
        /// The shared minimum distance.
        distance: usize,
        /// Size of the candidate set that was searched.
        registry_size: usize,
        /// How many of those candidates were within the threshold.
        within_threshold: usize,
    },
    /// Nothing in the registry is within the threshold.
    NotFound {
        /// Size of the candidate set that was searched.
        registry_size: usize,
    },
}

impl Recovery {
    /// The name to actually execute, or `None` when recovery was refused.
    pub fn canonical_name(&self) -> Option<&str> {
        match self {
            Recovery::Recovered { canonical, .. } => Some(canonical),
            _ => None,
        }
    }
}

/// Levenshtein distance between `a` and `b`, compared case-insensitively,
/// returning `None` as soon as it is provably greater than `max`.
///
/// Two-row DP with a per-row lower bound: a row whose every cell exceeds
/// `max` cannot be brought back under it by later rows, so the scan stops.
/// Combined with the length-difference check this makes the scan
/// `O(len * threshold)` for near misses instead of `O(len²)`.
pub fn levenshtein_bounded(a: &str, b: &str, max: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().flat_map(char::to_lowercase).collect();
    let b: Vec<char> = b.chars().flat_map(char::to_lowercase).collect();
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    if a.is_empty() {
        return (b.len() <= max).then_some(b.len());
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
            row_min = row_min.min(cur[j + 1]);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    (prev[b.len()] <= max).then_some(prev[b.len()])
}

/// Match `attempted` against the registry's tool names.
///
/// The candidate set is the entire real registry — every available,
/// non-disabled tool — and *all* of it is considered; the distance scan
/// just discards anything beyond [`RECOVERY_THRESHOLD`].
pub fn recover(attempted: &str, candidates: &[String]) -> Recovery {
    if candidates.iter().any(|c| c == attempted) {
        return Recovery::Exact;
    }
    if attempted.is_empty() {
        return Recovery::NotFound {
            registry_size: candidates.len(),
        };
    }

    let registry_size = candidates.len();
    let mut best: Option<usize> = None;
    let mut tied: Vec<String> = Vec::new();
    let mut within_threshold = 0usize;

    for candidate in candidates {
        let Some(distance) = levenshtein_bounded(attempted, candidate, RECOVERY_THRESHOLD) else {
            continue;
        };
        within_threshold += 1;
        match best {
            Some(b) if distance > b => {}
            Some(b) if distance == b => tied.push(candidate.clone()),
            Some(_) => {
                tied.clear();
                tied.push(candidate.clone());
                best = Some(distance);
            }
            None => {
                tied.push(candidate.clone());
                best = Some(distance);
            }
        }
    }

    match best {
        None => Recovery::NotFound { registry_size },
        Some(distance) if tied.len() > 1 => {
            tied.sort();
            Recovery::Ambiguous {
                candidates: tied,
                distance,
                registry_size,
                within_threshold,
            }
        }
        Some(distance) => Recovery::Recovered {
            canonical: tied.first().cloned().unwrap_or_default(),
            distance,
            registry_size,
            within_threshold,
        },
    }
}

/// Resolve a requested name against the live registry.
///
/// Reads `ToolRegistry::get_schemas()` — registered AND available AND not
/// disabled — so recovery can never resurrect a banned or unavailable tool.
pub async fn resolve(registry: &ToolRegistry, attempted: &str) -> Recovery {
    if attempted.is_empty() {
        return Recovery::NotFound { registry_size: 0 };
    }
    let candidates: Vec<String> = registry
        .get_schemas()
        .await
        .into_iter()
        .map(|s| s.name)
        .collect();
    recover(attempted, &candidates)
}

/// The text prepended to a tool result when a recovery happened, so the
/// model learns its name was wrong instead of building on a false premise.
pub fn recovery_notice(recovery: &Recovery) -> Option<String> {
    let Recovery::Recovered {
        canonical,
        distance,
        registry_size,
        within_threshold,
    } = recovery
    else {
        return None;
    };
    Some(format!(
        "[tool name recovered] No registered tool is named the name you called. \
         Executed '{canonical}' instead (edit distance {distance}; {within_threshold} of \
         {registry_size} registered tools were within the threshold of {RECOVERY_THRESHOLD}). \
         Use the exact name '{canonical}' from now on — do not repeat the incorrect name."
    ))
}

/// The error body used when recovery was refused. Names the candidates when
/// there are any, so the model has something concrete to call next instead
/// of retrying the same wrong name.
pub fn refusal_message(attempted: &str, recovery: &Recovery) -> String {
    match recovery {
        Recovery::Exact => format!("Tool '{attempted}' is registered."),
        Recovery::Recovered { canonical, .. } => {
            format!("Tool '{attempted}' resolved to '{canonical}'.")
        }
        Recovery::Ambiguous {
            candidates,
            distance,
            registry_size,
            within_threshold,
        } => format!(
            "[tool name not recovered] No registered tool is named '{attempted}'. {n} tools are \
             EQUALLY close to it (edit distance {distance}; {within_threshold} of {registry_size} \
             registered tools were within the threshold of {RECOVERY_THRESHOLD}): {list}. \
             Recovery is refused rather than guessed — call one of those exact names.",
            n = candidates.len(),
            list = candidates.join(", "),
        ),
        Recovery::NotFound { registry_size } => format!(
            "[tool name not recovered] No registered tool is named '{attempted}', and none of the \
             {registry_size} registered tools is within edit distance {RECOVERY_THRESHOLD} of it. \
             If it belongs to a disconnected MCP server, reconnect that server; otherwise call a \
             tool that appears in your tool list."
        ),
    }
}

impl fmt::Display for Recovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Recovery::Exact => write!(f, "exact match"),
            Recovery::Recovered {
                canonical,
                distance,
                ..
            } => write!(f, "recovered to '{canonical}' at distance {distance}"),
            Recovery::Ambiguous {
                candidates,
                distance,
                ..
            } => write!(
                f,
                "ambiguous: {} candidates at distance {distance}",
                candidates.len()
            ),
            Recovery::NotFound { registry_size } => {
                write!(
                    f,
                    "no candidate within threshold ({registry_size} searched)"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// A registry with deliberately dense sibling names — the realistic
    /// case where ties are common.
    const REGISTRY: &[&str] = &[
        "read_file",
        "read_files",
        "write_file",
        "list_files",
        "terminal",
        "file_read",
        "mcp_github_create_pull_request",
    ];

    #[test]
    fn tool_name_recovery_should_fix_a_near_miss() {
        let c = names(REGISTRY);

        // Dropped trailing 'e'. "read_files" is ALSO within the threshold
        // (distance 2) but strictly further away, so the unique minimum wins.
        let r = recover("read_fil", &c);
        assert_eq!(
            r,
            Recovery::Recovered {
                canonical: "read_file".to_string(),
                distance: 1,
                registry_size: 7,
                within_threshold: 2,
            }
        );

        // Wrong case costs zero edits and comes back canonically spelled.
        let r = recover("Read_File", &c);
        assert_eq!(r.canonical_name(), Some("read_file"));

        // Transposed pair is 1 edit under plain Levenshtein.
        assert_eq!(recover("reed_file", &c).canonical_name(), Some("read_file"));

        // MCP-prefixed near miss.
        let r = recover("mcp_github_create_pull_reque", &c);
        assert_eq!(r.canonical_name(), Some("mcp_github_create_pull_request"));

        // Beyond threshold: no recovery, no canonical name.
        assert_eq!(recover("read_files_typo", &c).canonical_name(), None);
    }

    #[test]
    fn tool_name_recovery_should_refuse_on_an_equidistant_tie() {
        // "rad_file" is exactly 1 edit from each of these — an arbitrary pick
        // would be a coin flip between two real tools.
        let c = names(&["read_file", "raid_file"]);
        let r = recover("rad_file", &c);
        assert_eq!(
            r,
            Recovery::Ambiguous {
                // Sorted, so the report is stable regardless of hash order.
                candidates: names(&["raid_file", "read_file"]),
                distance: 1,
                registry_size: 2,
                within_threshold: 2,
            }
        );
        assert_eq!(r.canonical_name(), None, "must not pick a winner");
        let msg = refusal_message("rad_file", &r);
        assert!(msg.contains("EQUALLY close"), "{msg}");
        assert!(
            msg.contains("read_file") && msg.contains("raid_file"),
            "{msg}"
        );
    }

    #[test]
    fn tool_name_recovery_should_refuse_when_only_case_differs() {
        // No exact match, but both spellings are 0 case-folded edits away.
        let c = names(&["read_file", "READ_FILE"]);
        let r = recover("Read_File", &c);
        assert!(
            matches!(r, Recovery::Ambiguous { distance: 0, .. }),
            "{r:?}"
        );
        assert_eq!(r.canonical_name(), None);

        // An EXACT match still wins outright — there is no tie to resolve.
        assert_eq!(recover("read_file", &c), Recovery::Exact);
    }

    #[test]
    fn tool_name_recovery_should_not_recover_beyond_the_threshold() {
        let c = names(&["read_file"]);

        // "rde_file" is 3 edits from "read_file" — refused.
        assert_eq!(levenshtein_bounded("rde_file", "read_file", 8), Some(3));
        assert_eq!(
            recover("rde_file", &c),
            Recovery::NotFound { registry_size: 1 }
        );
        assert_eq!(recover("rde_file", &c).canonical_name(), None);

        // "reed_file" is 1 edit — recovered. The threshold is the boundary.
        assert_eq!(recover("reed_file", &c).canonical_name(), Some("read_file"));

        // Nothing at all in range, over a two-tool registry.
        let c2 = names(&["read_file", "write_file"]);
        let r = recover("read_notes", &c2);
        assert_eq!(r, Recovery::NotFound { registry_size: 2 });
        let msg = refusal_message("read_notes", &r);
        assert!(msg.contains("within edit distance 2"), "{msg}");
        assert!(msg.contains("disconnected MCP server"), "{msg}");

        // The bound is real, not a coincidence of the fixtures.
        assert_eq!(levenshtein_bounded("read_file", "read_files", 2), Some(1));
        assert_eq!(levenshtein_bounded("read_file", "read_notes", 2), None);
        assert_eq!(levenshtein_bounded("abc", "", 2), None);
        assert_eq!(levenshtein_bounded("abc", "abc", 0), Some(0));
    }

    #[test]
    fn tool_name_recovery_should_report_that_it_recovered() {
        let c = names(&["read_file", "terminal"]);
        let r = recover("read_fil", &c);
        let notice = recovery_notice(&r).expect("recovered calls must produce a notice");
        assert!(
            notice.contains("read_fil"),
            "names the wrong call: {notice}"
        );
        assert!(notice.contains("read_file"), "names the fix: {notice}");
        assert!(notice.contains("edit distance 1"), "{notice}");
        assert!(
            notice.contains("do not repeat"),
            "must tell the model to stop retrying: {notice}"
        );

        // No notice when nothing was recovered — the model must never be told
        // about a recovery that did not happen.
        for refused in [
            recover("zebra_nonexistent", &c),
            Recovery::Exact,
            recover("rad_file", &names(&["read_file", "raid_file"])),
        ] {
            assert_eq!(recovery_notice(&refused), None, "{refused:?}");
        }
    }

    #[tokio::test]
    async fn resolve_never_recovers_onto_a_disabled_tool() {
        use crate::tools::tool_search::DeferredDummy;
        let registry = ToolRegistry::new(std::time::Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("read_file", "Read a file"))
            .await
            .expect("register read_file");
        registry
            .register(DeferredDummy::new("read_files", "Read many files"))
            .await
            .expect("register read_files");
        registry.disable_tool("read_files").await;
        // "read_fil" was within threshold of both registered names; the
        // disabled one leaves a single candidate, so recovery is safe.
        assert_eq!(
            resolve(&registry, "read_fil").await.canonical_name(),
            Some("read_file")
        );
        registry.disable_tool("read_file").await;
        // Both gone — nothing to recover onto, and the message must point at
        // the state rather than inventing a tool.
        assert_eq!(
            resolve(&registry, "read_fil").await,
            Recovery::NotFound { registry_size: 0 }
        );
    }

    #[test]
    fn exact_match_is_a_no_op() {
        let c = names(REGISTRY);
        assert_eq!(recover("terminal", &c), Recovery::Exact);
        assert_eq!(recovery_notice(&Recovery::Exact), None);
    }

    #[test]
    fn bounded_distance_agrees_with_a_reference_implementation() {
        // Exhaustive check against a plain full-matrix Levenshtein.
        fn reference(a: &str, b: &str) -> usize {
            let a: Vec<char> = a.to_lowercase().chars().collect();
            let b: Vec<char> = b.to_lowercase().chars().collect();
            let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
            for (i, row) in d.iter_mut().enumerate() {
                row[0] = i;
            }
            for (j, cell) in d[0].iter_mut().enumerate() {
                *cell = j;
            }
            for i in 1..=a.len() {
                for j in 1..=b.len() {
                    let cost = usize::from(a[i - 1] != b[j - 1]);
                    d[i][j] = (d[i - 1][j] + 1)
                        .min(d[i][j - 1] + 1)
                        .min(d[i - 1][j - 1] + cost);
                }
            }
            d[a.len()][b.len()]
        }
        let words = [
            "ab",
            "abc",
            "abcd",
            "read",
            "read_file",
            "read_fil",
            "reed_file",
            "",
            "x",
        ];
        for a in words {
            for b in words {
                let want = reference(a, b);
                assert_eq!(
                    levenshtein_bounded(a, b, want),
                    Some(want),
                    "a={a:?} b={b:?} d={want}"
                );
                if want > 0 {
                    assert_eq!(
                        levenshtein_bounded(a, b, want - 1),
                        None,
                        "a={a:?} b={b:?} must exceed bound {}",
                        want - 1
                    );
                }
            }
        }
    }
}
