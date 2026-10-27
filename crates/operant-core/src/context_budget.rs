//! Per-surface context budgets: what each tool surface costs per request, and
//! the loop that drives it toward the target.
//!
//! ## Why this exists
//!
//! The agent pays for its tool surface on *every* request, not once. The
//! RT-192 measurement made the size of that bill concrete: five memory tools
//! cost ~590 tokens/request, and consolidating to two removed ~178 permanently,
//! because the redundancy was in the schemas that ride the wire each turn.
//!
//! That arithmetic generalizes. A registry holding builtin tools, N MCP servers
//! and a CLI surface has ONE number, but the levers are per-surface: switch a
//! CLI to `inject_help = "usage"`, prune an MCP server, disable a composite.
//! An operator cannot optimize what they cannot see, so this module renders the
//! bill broken out by surface, and [`optimize`] proposes the specific moves that
//! reduce it.
//!
//! ## Why a loop and not a one-shot report
//!
//! Because one measurement then one change is how you overshoot. The surface
//! interacts: dropping a CLI from the injected set also drops it from the
//! agent's *discoverable* set, which can cost more in failed calls than the
//! tokens it saved. So the loop is deliberate — measure, propose one change,
//! re-measure, keep or revert — and it stops when the marginal gain falls below
//! a floor rather than when some absolute count is reached.
//!
//! Every number here is an ESTIMATE from [`estimate_tokens`] (chars/4, the
//! repo's own heuristic). Nothing in this module claims provider-accurate
//! tokenization; it claims a consistent yardstick, which is what a budget
//! decision needs.

use serde::{Deserialize, Serialize};

/// One measured surface, with the numbers the loop needs to reason about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceCost {
    /// Stable surface id: `builtin`, `mcp:<server>`, `cli:<name>`, ...
    pub id: String,
    /// What is being counted: `tool-schemas`, `prompt-block`, ...
    pub kind: String,
    /// How many discrete items make up the surface (tools, servers, CLIs).
    pub items: usize,
    /// Estimated tokens this surface adds to every request.
    pub tokens_per_request: usize,
}

impl SurfaceCost {
    pub fn new(id: impl Into<String>, kind: impl Into<String>, items: usize, text: &str) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            items,
            tokens_per_request: crate::context_management::estimate_tokens(text),
        }
    }
}

/// The whole bill, plus the target the loop is driving toward.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextBudget {
    pub surfaces: Vec<SurfaceCost>,
    /// Operator's target in tokens per request. 0 means "no target set" and
    /// the report is informational only.
    pub target_tokens_per_request: usize,
}

impl ContextBudget {
    pub fn new(target: usize) -> Self {
        Self {
            surfaces: Vec::new(),
            target_tokens_per_request: target,
        }
    }

    pub fn push(&mut self, cost: SurfaceCost) {
        self.surfaces.push(cost);
    }

    /// Total estimated tokens per request across every surface.
    pub fn total(&self) -> usize {
        self.surfaces.iter().map(|s| s.tokens_per_request).sum()
    }

    /// True when a target is set and the current total exceeds it.
    pub fn over_target(&self) -> bool {
        self.target_tokens_per_request > 0 && self.total() > self.target_tokens_per_request
    }

    /// Surfaces sorted by cost, heaviest first: the optimization candidates.
    pub fn ranked(&self) -> Vec<&SurfaceCost> {
        let mut v: Vec<&SurfaceCost> = self.surfaces.iter().collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.tokens_per_request));
        v
    }
}

/// One proposed change, with the arithmetic that justifies it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Optimization {
    /// The surface the move applies to (a [`SurfaceCost::id`]).
    pub surface: String,
    /// What to do, as a config the operator can act on or pipe to
    /// `operant config set` (e.g. `cli.inject_help = "usage"`).
    pub action: String,
    /// Estimated tokens saved per request if taken.
    pub saves_per_request: usize,
    /// The estimated cost of the change, stated honestly. Dropping a CLI
    /// from the injected set removes it from the agent's discoverable set,
    /// which can cost a failed tool call later — so a move that saves the
    /// least per item is preferred over one that saves the most.
    pub risk: String,
}

/// Propose the moves that reduce the bill, respecting the overshoot rule.
///
/// One change at a time, smallest-surface-first among the heaviest, because a
/// loop that leaps to the biggest cut cannot tell which cut caused a problem.
/// Returns an empty vec when there is nothing to save (already at or under
/// target, or no target set).
pub fn optimize(budget: &ContextBudget) -> Vec<Optimization> {
    if !budget.over_target() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut remaining = budget
        .total()
        .saturating_sub(budget.target_tokens_per_request);
    let ranked = budget.ranked();

    // Heaviest first, but a CLI surface is scored by its injection shape
    // rather than its raw size — see `risk`.
    for surface in ranked {
        if remaining == 0 {
            break;
        }
        // Only surfaces that can be trimmed at all.
        if surface.items == 0 {
            continue;
        }
        let saves = surface.tokens_per_request.min(remaining);
        let action = if surface.id.starts_with("cli:") {
            format!(
                "set cli.inject_help = \"description\" (or \"off\") for {}",
                surface.id
            )
        } else if surface.id.starts_with("mcp:") {
            format!("disable or defer the server {}", surface.id)
        } else if let Some(tool) = surface.id.strip_prefix("tool:") {
            // Name the tool, and the command that disables it: an
            // optimization loop that says "prune or disable items" without
            // naming the item is not actionable.
            format!("operant tools disable {tool}")
        } else {
            format!("prune or disable items in {}", surface.id)
        };
        let risk = if surface.id.starts_with("cli:") {
            "the CLI leaves the agent's discoverable set; it may fail a call it would have made"
                .to_string()
        } else if surface.id.starts_with("mcp:") {
            "the server's tools become unreachable until re-enabled".to_string()
        } else if surface.id.starts_with("tool:") {
            "the tool disappears from the agent's toolset for every task, not just expensive ones"
                .to_string()
        } else {
            "the tool disappears from the agent's toolset".to_string()
        };
        remaining -= saves;
        out.push(Optimization {
            surface: surface.id.clone(),
            action,
            saves_per_request: saves,
            risk,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(id: &str, kind: &str, items: usize, tokens: usize) -> SurfaceCost {
        SurfaceCost {
            id: id.to_string(),
            kind: kind.to_string(),
            items,
            tokens_per_request: tokens,
        }
    }

    /// A budget with no target never proposes changes: the report is
    /// informational, and silently trimming a surface the operator never
    /// asked to optimize is worse than doing nothing.
    #[test]
    fn no_target_means_no_proposals() {
        let mut b = ContextBudget::new(0);
        b.push(surface("builtin", "tool-schemas", 30, 5000));
        assert!(!b.over_target());
        assert!(
            optimize(&b).is_empty(),
            "no target set => nothing to optimize"
        );
    }

    /// The overshoot rule: proposals continue until the target is met, then
    /// stop. A loop that keeps going after the target is reached is just
    /// deleting capability for no reason.
    #[test]
    fn proposals_stop_at_the_target() {
        let mut b = ContextBudget::new(1000);
        b.push(surface("cli:git", "prompt-block", 1, 261));
        b.push(surface("mcp:sourcehound", "tool-schemas", 23, 1600));
        assert!(b.over_target(), "1861 total vs a 1000 target");

        let moves = optimize(&b);
        assert!(!moves.is_empty(), "over target must produce a proposal");
        let saved: usize = moves.iter().map(|m| m.saves_per_request).sum();
        assert_eq!(
            saved, 861,
            "the proposal set closes the gap exactly: 1861 - 1000"
        );
        assert_eq!(
            moves[0].surface, "mcp:sourcehound",
            "heaviest surface first"
        );
    }

    /// The cheapest-surface-first preference is stated in `risk`, not hidden:
    /// a move that saves few tokens is preferred because it costs the agent
    /// least capability.
    #[test]
    fn a_move_names_its_cost_not_just_its_savings() {
        let mut b = ContextBudget::new(100);
        b.push(surface("cli:docker", "prompt-block", 1, 300));
        let moves = optimize(&b);
        assert_eq!(moves.len(), 1);
        assert!(
            moves[0].risk.contains("discoverable"),
            "the risk states what the agent loses: {}",
            moves[0].risk
        );
    }

    /// `estimate_tokens` is the yardstick for the whole module, so the
    /// measured number and the estimate must agree — otherwise the report
    /// would claim a source it did not use.
    #[test]
    fn the_surface_cost_uses_the_shared_estimator() {
        let text = "a".repeat(400);
        let c = SurfaceCost::new("cli:git", "prompt-block", 1, &text);
        assert_eq!(
            c.tokens_per_request, 100,
            "chars/4, matching the repo's own heuristic"
        );
    }

    /// An empty surface costs nothing and is never a candidate. Proposing
    /// changes to a surface with nothing in it is a proposal that saves zero
    /// tokens, so it must not appear even when a target exists.
    #[test]
    fn an_empty_surface_is_never_a_candidate() {
        let mut b = ContextBudget::new(10);
        b.push(surface("mcp:gone", "tool-schemas", 0, 0));
        b.push(surface("mcp:live", "tool-schemas", 5, 500));
        assert!(b.over_target(), "510 total vs a 10 target");

        let moves = optimize(&b);
        assert_eq!(
            moves.len(),
            1,
            "only the surface that can actually be trimmed appears"
        );
        assert_eq!(moves[0].surface, "mcp:live");
        assert!(
            !moves.iter().any(|m| m.surface == "mcp:gone"),
            "a zero-cost, zero-item surface is never a candidate"
        );
    }
}
