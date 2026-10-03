//! P0 of `docs/PERMISSION-SCOPING-PLAN.md` §5: the per-seat policy genome.
//!
//! One policy shape every employee agent inherits and every seat specialises:
//! a [`SeatMode`] plus allow/deny patterns matched with the LCM glob matcher
//! (`crate::context::lcm::glob_match`), so patterns are namespace-addressable
//! (`content.*`, `jcode.*`). New tools and whole future subsystems inherit
//! their namespace's posture without touching this engine — that is the
//! "any number of systems" property.
//!
//! [`decide`] is a **pure function**: data in, verdict out. It deliberately
//! knows nothing about sqlite, channels, or the agent loop — P1 composes it
//! into the run path, P2 stores the data. Nothing here performs I/O.
//!
//! Precedence (plan §4, first match wins):
//!
//! 1. hardline blocklist — absolute, over YOLO and over grants;
//! 2. seat deny list — absolute over everything below (a deny row a CEO
//!    grant cannot override: amplification must be impossible through
//!    negation as well as through the ladder);
//! 3. seat allow list — runs;
//! 4. `Yolo` mode — runs;
//! 5. standing grant — runs (grants extend, they never un-deny);
//! 6. mode defaults: `Lockdown`/`Scoped` escalate, `Standard` escalates
//!    dangerous tools and runs safe ones, no policy reproduces today's
//!    ungoverned behaviour byte-for-byte (migration is opt-in per seat).
//!
//! A seat with **no policy row** is byte-identical to mainline: safe tools
//! run, dangerous tools escalate, the blocklist still bites. Wiring a policy
//! is the opt-in, never the default.

use crate::context::lcm::glob_match;
use std::str::FromStr;

/// A seat's operating mode. Plan §1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatMode {
    /// Every tool open, no prompts. Still under the hardline blocklist.
    Yolo,
    /// Today's behaviour: dangerous tools escalate, safe tools run.
    Standard,
    /// Only allow-listed tools run; the rest escalate (missing ≠ denied).
    Scoped,
    /// Nothing runs without an allow entry or a standing grant.
    Lockdown,
}

impl SeatMode {
    /// Stored/parsed spelling, matching the plan's wire names.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yolo => "yolo",
            Self::Standard => "standard",
            Self::Scoped => "scoped",
            Self::Lockdown => "lockdown",
        }
    }
}

/// Unknown [`SeatMode`] spelling — kept as its own error so a policy row's
/// typo names itself in the audit trail instead of surfacing as a bare string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatModeParseError {
    /// The raw string the policy row carried.
    pub raw: String,
}

impl std::fmt::Display for SeatModeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown seat mode '{}' (expected yolo | standard | scoped | lockdown)",
            self.raw
        )
    }
}

impl std::error::Error for SeatModeParseError {}

impl FromStr for SeatMode {
    type Err = SeatModeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "yolo" => Ok(Self::Yolo),
            "standard" => Ok(Self::Standard),
            "scoped" => Ok(Self::Scoped),
            "lockdown" => Ok(Self::Lockdown),
            _ => Err(SeatModeParseError { raw: s.to_string() }),
        }
    }
}

/// One seat's policy. Patterns are globs (`content.*`, `chat.send`).
///
/// `allow`/`deny` list *tool names or namespace globs*, not capabilities —
/// the tool name is what the run path has in hand at the decision point, and
/// a namespace prefix is how a whole future subsystem is governed in one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatPolicy {
    pub mode: SeatMode,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// The verdict for one tool call, with the sentence that explains it —
/// every Run/Escalate/Deny an audit surface later shows must be able to say
/// *which rule* produced it, so the `why` travels with the verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatDecision {
    Run(String),
    Escalate(String),
    Deny(String),
}

fn listed(patterns: &[String], tool: &str) -> bool {
    patterns.iter().any(|p| glob_match(p, tool))
}

/// Decide one tool call. Pure; no I/O; no clock (pass `has_grant` already
/// resolved — expiry is the ledger's job, and the caller's `now` is the
/// ledger's `now`).
///
/// - `policy` — `None` = ungoverned seat, today's behaviour.
/// - `tool` — the tool name the run path holds.
/// - `blocked` — the hardline blocklist's verdict (approval.rs patterns).
/// - `dangerous` — the existing smart-approval gate's classification
///   (`agent/stream.rs:709`); it stays the classifier, this stays the policy.
/// - `has_grant` — an unexpired, unrevoked grant row covers this tool for
///   this seat (the grant ledger's `list_for_grantee` already refuses lapsed
///   rows, so `true` here means *currently standing*).
pub fn decide(
    policy: Option<&SeatPolicy>,
    tool: &str,
    blocked: bool,
    dangerous: bool,
    has_grant: bool,
) -> SeatDecision {
    // RULE 1 — the blocklist is infrastructure, not policy. Absolute.
    if blocked {
        return SeatDecision::Deny(format!(
            "tool {tool} matches the hardline blocklist; the blocklist is \
             infrastructure and no seat mode, allow entry, or grant overrides it"
        ));
    }

    // RULE 2 — an ungoverned seat is byte-identical to mainline. Wiring a
    // policy is the opt-in; its absence must not change behaviour.
    let Some(policy) = policy else {
        return if dangerous {
            SeatDecision::Escalate(format!(
                "seat holds no policy and tool {tool} is classified dangerous; \
                 this is today's ungoverned behaviour"
            ))
        } else {
            SeatDecision::Run(format!(
                "tool {tool} is safe and the seat holds no policy; \
                 this is today's ungoverned behaviour"
            ))
        };
    };

    // RULE 3 — deny is absolute over allow, YOLO, and grants. A grant can
    // extend authority but never negate a negation.
    if listed(&policy.deny, tool) {
        return SeatDecision::Deny(format!(
            "tool {tool} matches the seat's deny list; deny rows override \
             allow entries, YOLO, and standing grants"
        ));
    }

    // RULE 4 — the allow list runs regardless of mode (an allow row inside
    // lockdown is the point of lockdown's allow list).
    if listed(&policy.allow, tool) {
        return SeatDecision::Run(format!("tool {tool} matches the seat's allow list"));
    }

    // RULE 5 — YOLO opens everything the deny list leaves.
    if policy.mode == SeatMode::Yolo {
        return SeatDecision::Run(format!(
            "seat is in yolo mode and tool {tool} is not denied"
        ));
    }

    // RULE 6 — a standing grant extends authority past the mode default.
    if has_grant {
        return SeatDecision::Run(format!(
            "tool {tool} is covered by a standing grant for this seat"
        ));
    }

    // RULE 7 — mode defaults. Standard keeps today's split; scoped and
    // lockdown escalate everything their allow list does not name — a
    // missing entry is a request for authority, not a denial.
    match policy.mode {
        SeatMode::Standard => {
            if dangerous {
                SeatDecision::Escalate(format!(
                    "tool {tool} is classified dangerous and the seat is in \
                     standard mode; escalate as today"
                ))
            } else {
                SeatDecision::Run(format!(
                    "tool {tool} is safe and the seat is in standard mode"
                ))
            }
        }
        SeatMode::Scoped => SeatDecision::Escalate(format!(
            "tool {tool} is not on the seat's allow list and the seat is in \
             scoped mode; missing is not denied, it is requested"
        )),
        SeatMode::Lockdown => SeatDecision::Escalate(format!(
            "tool {tool} has neither an allow entry nor a standing grant and \
             the seat is in lockdown mode"
        )),
        // Yolo was settled by RULE 5; Standard above. Unreachable, kept
        // exhaustive so adding a mode is a compile error here, not a silent
        // fall-through.
        SeatMode::Yolo => unreachable!("yolo settled by RULE 5"),
    }
}

/// Storage seam for the run path (wave-2 slice E consults this).
/// Sync because the stores are `Arc<Mutex<Connection>>` like `GrantDb`,
/// and the decision point must not await.
pub trait SeatPolicySource: Send + Sync {
    fn policy_for(&self, employee_id: &str) -> Option<SeatPolicy>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(mode: SeatMode, allow: &[&str], deny: &[&str]) -> SeatPolicy {
        SeatPolicy {
            mode,
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn blocklist_beats_yolo_allow_and_grant() {
        let p = policy(SeatMode::Yolo, &["*"], &[]);
        assert!(matches!(
            decide(Some(&p), "shell", true, true, true),
            SeatDecision::Deny(_)
        ));
        assert!(matches!(
            decide(None, "shell", true, false, false),
            SeatDecision::Deny(_)
        ));
    }

    #[test]
    fn deny_list_beats_grant_allow_and_yolo() {
        // Deny beats a standing grant even under YOLO with a wildcard allow.
        let p = policy(SeatMode::Yolo, &["*"], &["shell"]);
        assert!(matches!(
            decide(Some(&p), "shell", false, true, true),
            SeatDecision::Deny(_)
        ));
    }

    #[test]
    fn deny_pattern_beats_allow_pattern() {
        // allow content.* but deny content.write: the hole wins.
        let p = policy(SeatMode::Scoped, &["content.*"], &["content.write"]);
        assert!(matches!(
            decide(Some(&p), "content.read", false, false, false),
            SeatDecision::Run(_)
        ));
        assert!(matches!(
            decide(Some(&p), "content.write", false, false, false),
            SeatDecision::Deny(_)
        ));
    }

    #[test]
    fn lockdown_escalates_without_grant_and_runs_with_one() {
        let p = policy(SeatMode::Lockdown, &[], &[]);
        assert!(matches!(
            decide(Some(&p), "memory_search", false, false, false),
            SeatDecision::Escalate(_)
        ));
        assert!(matches!(
            decide(Some(&p), "memory_search", false, false, true),
            SeatDecision::Run(_)
        ));
    }

    #[test]
    fn lockdown_allow_list_runs() {
        let p = policy(SeatMode::Lockdown, &["memory_search"], &[]);
        assert!(matches!(
            decide(Some(&p), "memory_search", false, false, false),
            SeatDecision::Run(_)
        ));
    }

    #[test]
    fn scoped_unlisted_escalates() {
        let p = policy(SeatMode::Scoped, &["content.*"], &[]);
        assert!(matches!(
            decide(Some(&p), "memory_search", false, false, false),
            SeatDecision::Escalate(_)
        ));
    }

    #[test]
    fn standard_matches_today() {
        let p = policy(SeatMode::Standard, &[], &[]);
        assert!(matches!(
            decide(Some(&p), "memory_search", false, false, false),
            SeatDecision::Run(_)
        ));
        assert!(matches!(
            decide(Some(&p), "shell", false, true, false),
            SeatDecision::Escalate(_)
        ));
        // A grant for a dangerous tool runs without prompting — that is the
        // "does not re-ask every run" acceptance from the plan's P2.
        assert!(matches!(
            decide(Some(&p), "shell", false, true, true),
            SeatDecision::Run(_)
        ));
    }

    fn no_policy() -> Option<&'static SeatPolicy> {
        None
    }

    #[test]
    fn no_policy_is_byte_identical_to_mainline() {
        assert!(matches!(
            decide(no_policy(), "memory_search", false, false, false),
            SeatDecision::Run(_)
        ));
        assert!(matches!(
            decide(no_policy(), "shell", false, true, false),
            SeatDecision::Escalate(_)
        ));
        // Even a standing grant does not change an ungoverned seat: grants
        // only matter under governance, so a stray row cannot silently open
        // a seat nobody has wired a policy for.
        assert!(matches!(
            decide(no_policy(), "shell", false, true, true),
            SeatDecision::Escalate(_)
        ));
    }

    #[test]
    fn namespace_glob_governs_future_subsystems() {
        // A subsystem that does not exist yet is governed by its namespace
        // pattern — this is the scalability acceptance for "any number of
        // systems added in the future".
        let p = policy(SeatMode::Scoped, &["content.*", "jcode.*"], &[]);
        assert!(matches!(
            decide(Some(&p), "jcode.visual.port", false, true, false),
            SeatDecision::Run(_)
        ));
        let d = policy(SeatMode::Yolo, &[], &["jcode.*"]);
        assert!(matches!(
            decide(Some(&d), "jcode.visual.port", false, true, true),
            SeatDecision::Deny(_)
        ));
    }

    #[test]
    fn mode_wire_format_round_trips() {
        use std::str::FromStr;
        for mode in [
            SeatMode::Yolo,
            SeatMode::Standard,
            SeatMode::Scoped,
            SeatMode::Lockdown,
        ] {
            assert_eq!(SeatMode::from_str(mode.as_str()).as_ref(), Ok(&mode));
        }
        // Unknown mode string is an error, never a silent Yolo default.
        assert!(SeatMode::from_str("superuser").is_err());
        // Case and whitespace tolerant, same as AuthorityScope's parse.
        assert_eq!(
            SeatMode::from_str("  LockDown ").as_ref(),
            Ok(&SeatMode::Lockdown)
        );
    }
}
