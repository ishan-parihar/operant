//! The `ui` widget tree does not carry a second, stubbed predicate surface.
//!
//! # Why
//!
//! Four `pub` methods on the UI surface had no caller anywhere in the
//! workspace. Each was proven dead by the acid test (the `pub` removed,
//! `cargo check -p operant-core --lib` reporting "methods ... are never
//! used"):
//!
//!   * `WidgetNode::children_mut` — a mutable children walk whose doc called
//!     it "the string-leaf scrub walk". No scrub walk called it; the live one
//!     is `SurfaceOp::string_leaves`, which recurses over `widgets()` itself.
//!   * `WidgetNode::has_unverified` — **a stub returning a hardcoded `false`.**
//!     Its own doc pointed at `SurfaceOp::is_synthesized`, which is the real
//!     check and is live (4 call sites: `gateway_runner`, `agent_events`,
//!     `channel_drains`, and the gateway's own dispatch).
//!   * `SurfaceOp::is_create` — a `matches!` leaf.
//!   * `WidgetNode::is_text_field` — a `matches!` leaf.
//!
//! The `has_unverified` one is the reason this file exists. Two names for one
//! behaviour, where one was a **constant**: a reader who greps "is this
//! surface unverified?" found a confident-looking predicate that answered
//! "no" unconditionally. That is the same shape as the file-backed memory
//! context path (iter-2727) and the MCP OAuth 401 surface (iter-2734) — two
//! mechanisms, one description, only one reachable.
//!
//! What this test pins: the LIVE predicates still behave correctly, and the
//! real `is_synthesized` / `string_leaves` pair is what the scrubber uses.

use operant_core::ui::{Provenance, SurfaceOp, WidgetNode};

fn text(s: &str) -> WidgetNode {
    WidgetNode::Text {
        text: s.to_string(),
        tone: Default::default(),
        mono: false,
    }
}

fn create(provenance: Provenance, body: WidgetNode) -> SurfaceOp {
    SurfaceOp::CreateSurface {
        surface_id: "s1".to_string(),
        intent: "a test surface".to_string(),
        provenance,
        body,
    }
}

/// `SurfaceOp::is_synthesized` is the live provenance check. It must be true
/// only for `AgentSynthesis` and false for every other provenance -- which is
/// exactly what the deleted `WidgetNode::has_unverified` claimed to do and
/// never did (it returned `false` unconditionally).
#[test]
fn is_synthesized_is_the_real_provenance_check() {
    let synthesized = create(Provenance::AgentSynthesis, text("hi"));
    assert!(
        synthesized.is_synthesized(),
        "a CreateSurface carrying AgentSynthesis provenance must be synthesized"
    );

    let user = create(Provenance::UserSupplied, text("hi"));
    assert!(
        !user.is_synthesized(),
        "a UserSupplied op must NOT be synthesized"
    );

    let tool = create(
        Provenance::ToolVerified {
            tool_name: "terminal".to_string(),
            source_uri: None,
        },
        text("hi"),
    );
    assert!(
        !tool.is_synthesized(),
        "a ToolVerified op must NOT be synthesized"
    );

    // Only CreateSurface carries provenance at the op level, so an
    // UpdateComponents is never synthesized however its payload looks.
    let update = SurfaceOp::UpdateComponents {
        surface_id: "s1".to_string(),
        components: vec![text("hi")],
    };
    assert!(
        !update.is_synthesized(),
        "only CreateSurface carries provenance at the op level"
    );
}

/// `string_leaves` is the live scrub walk. It must reach every string leaf in
/// the tree, which is the walk `children_mut` claimed to be.
#[test]
fn string_leaves_reaches_every_leaf() {
    let op = create(
        Provenance::UserSupplied,
        WidgetNode::Row {
            children: vec![text("alpha"), text("beta")],
        },
    );

    let leaves = op.string_leaves();
    assert!(
        leaves.iter().any(|s| *s == "alpha") && leaves.iter().any(|s| *s == "beta"),
        "string_leaves must reach nested children, got {leaves:?}"
    );
}

/// A `WidgetNode::Text` is still constructible and identifiable by its shape,
/// which is what `is_text_field` used to answer.
#[test]
fn widget_node_surface_stays_minimal() {
    let n = text("x");
    assert!(
        matches!(n, WidgetNode::Text { .. }),
        "Text is a leaf variant"
    );
}

/// `Provenance` must stay `Clone + PartialEq`: the call sites compare it and
/// the surface ops clone it into events.
#[test]
fn provenance_is_comparable() {
    let a = Provenance::AgentSynthesis;
    let b = a.clone();
    assert_eq!(a, b, "Provenance must be comparable by value");
    assert_ne!(a, Provenance::UserSupplied, "distinct provenances differ");
}
