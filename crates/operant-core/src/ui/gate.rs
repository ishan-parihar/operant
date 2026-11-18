//! M4 — the governance gate.
//!
//! One function, [`validate_surface`], called from the surface EMISSION path
//! in `crate::tools::ui_tool` and from the `ui_request_input` dispatch path —
//! deliberately NOT from the `ToolRegistry` interceptor seam. A pre-execute
//! interceptor runs before the `SurfaceOp` exists; a post-execute interceptor
//! runs after `ui_render_surface` has already emitted its `AgentEvent` and
//! after `ui_request_input` has already suspended the turn and shown its
//! card. Both are too late to refuse anything, which is why the seam call is
//! inside the tool (plan review F8).
//!
//! The checks, in order:
//!
//! 1. **Enabled** — `[ui] generative_enabled = true` (the kill switch, G7).
//! 2. **Well-formed** — the depth and node-count bounds, so a 400-deep tree
//!    never reaches a renderer.
//! 3. **Capability match** — the transport can actually show the node kinds
//!    the op uses.
//! 4. **Secret scrub** — every string leaf through
//!    [`crate::redaction::redact_sensitive_text`], BEFORE layout. The
//!    scrubber is regex MASKING, not fail-closed DLP, and a leaf-wise walk
//!    misses a secret split across two sibling cells; the caller therefore
//!    ALSO scrubs the serialized fallback text (the tool does this), and the
//!    `[v] view raw JSON` view shows the post-scrub op, never the pre-scrub
//!    one (review F8).
//!
//! Fail-closed throughout: a refusal is a [`UiRefusal`] naming the rule, and
//! the tool turns it into an actionable error string for the model.

use super::{
    Provenance, SurfaceCapabilities, SurfaceOp, SurfaceVerdict, UiRefusal, check_capabilities,
    check_depth_and_count,
};
use crate::config::AppConfig;
use crate::redaction::redact_sensitive_text;

/// The `[ui]` section. One boolean, default true (G7).
///
/// Deliberately tiny: the plan's cost line is "1 config boolean". Every other
/// knob the OpenBot digest suggests (per-transport overrides, an image size
/// ladder) is derived from the capability set, not configured, because a
/// configuration surface that nothing reads is the `lifeos_enabled` failure
/// this repo already paid for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UiSettings {
    /// Whether the agent may emit surfaces at all. The operator's kill
    /// switch: when false, both surface tools refuse with
    /// [`UiRefusal::Disabled`] and the renderer path is never reached.
    pub generative_enabled: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            generative_enabled: true,
        }
    }
}

/// Validate one op against the config and the live transport, scrubbing its
/// strings in place.
///
/// Returns the op to RENDER — which is the (possibly rewritten) argument,
/// never the caller's original. A caller that renders the pre-scrub value
/// would undo the scrub.
pub fn validate_surface(
    op: &mut SurfaceOp,
    caps: &SurfaceCapabilities,
    config: &AppConfig,
) -> SurfaceVerdict<()> {
    if !config.ui.generative_enabled {
        return Err(UiRefusal::Disabled);
    }
    check_depth_and_count(op)?;
    check_capabilities(op, caps)?;
    scrub(op);
    Ok(())
}

/// Run every string leaf of `op` through the secret scrubber.
///
/// Public because the tool scrubs the SERIALIZED fallback text as well — the
/// leaf walk alone cannot see a token split across two cells.
pub fn scrub(op: &mut SurfaceOp) {
    for leaf in op.string_leaves_mut() {
        let cleaned = redact_sensitive_text(leaf);
        if cleaned != *leaf {
            *leaf = cleaned;
        }
    }
}

/// Render a refusal as the string the model receives.
///
/// The model needs the RULE, not the enum: "ui_render_surface: surface has
/// 412 widgets; the limit is 200 (split it into several surfaces)" is
/// something it can act on, whereas `OutOfBounds` is not.
pub fn refusal_message(tool: &str, refusal: &UiRefusal) -> String {
    format!("{tool}: {refusal}")
}

/// The provenance a caller should stamp when the model supplied the content
/// and no tool did.
///
/// Spelled out here so the tool cannot accidentally default to
/// `ToolVerified` — the whole point of the badge is that "the agent made
/// this up" is distinguishable from "a tool measured this".
pub const MODEL_PROVENANCE: Provenance = Provenance::AgentSynthesis;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::*;

    fn config_with(enabled: bool) -> AppConfig {
        let mut config = AppConfig::default();
        config.ui.generative_enabled = enabled;
        config
    }

    fn table_with(cell: &str) -> SurfaceOp {
        SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "t".into(),
            provenance: Provenance::AgentSynthesis,
            body: WidgetNode::Table {
                columns: vec![TableColumn {
                    header: "k".into(),
                    align: ColumnAlign::Left,
                }],
                rows: vec![TableRow {
                    cells: vec![cell.to_string()],
                }],
            },
        }
    }

    fn leaf(op: &SurfaceOp) -> String {
        op.string_leaves().join("|")
    }

    #[test]
    fn disabled_config_refuses_every_op() {
        let mut op = table_with("hello");
        let err = validate_surface(&mut op, &SurfaceCapabilities::tui(), &config_with(false))
            .unwrap_err();
        assert!(matches!(err, UiRefusal::Disabled));
    }

    #[test]
    fn enabled_config_passes_a_read_only_op() {
        let mut op = table_with("hello");
        assert!(validate_surface(&mut op, &SurfaceCapabilities::tui(), &config_with(true)).is_ok());
    }

    #[test]
    fn openai_key_in_a_cell_is_masked() {
        let mut op = table_with("key sk-abcdefghijklmnopqrstuvwxyz123456");
        validate_surface(&mut op, &SurfaceCapabilities::tui(), &config_with(true)).unwrap();
        let text = leaf(&op);
        assert!(!text.contains("sk-abcdefghijklmnopqrstuvwxyz"), "{text}");
        assert!(text.contains("[REDACTED]"), "{text}");
    }

    #[test]
    fn github_token_in_a_cell_is_masked() {
        let mut op = table_with("token ghp_abcdefghijklmnopqrstuvwxyz0123456789");
        validate_surface(&mut op, &SurfaceCapabilities::tui(), &config_with(true)).unwrap();
        assert!(!leaf(&op).contains("ghp_"), "{}", leaf(&op));
    }

    #[test]
    fn private_key_block_is_masked() {
        let mut op = table_with(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1234\n-----END RSA PRIVATE KEY-----",
        );
        validate_surface(&mut op, &SurfaceCapabilities::tui(), &config_with(true)).unwrap();
        assert!(!leaf(&op).contains("MIIEowIBAAKCAQEA"), "{}", leaf(&op));
    }

    #[test]
    fn postgres_connection_string_password_is_masked() {
        let mut op = table_with("postgres://admin:sup3rs3cret@db.example.com/app");
        validate_surface(&mut op, &SurfaceCapabilities::tui(), &config_with(true)).unwrap();
        let text = leaf(&op);
        assert!(!text.contains("sup3rs3cret"), "{text}");
    }

    #[test]
    fn scrub_reaches_nested_widgets_not_just_the_root() {
        let mut op = SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "t".into(),
            provenance: Provenance::AgentSynthesis,
            body: WidgetNode::Column {
                children: vec![
                    WidgetNode::Card {
                        title: "outer".into(),
                        tone: SemanticTone::Default,
                        child: Box::new(WidgetNode::Table {
                            columns: vec![TableColumn {
                                header: "h".into(),
                                align: ColumnAlign::Left,
                            }],
                            rows: vec![TableRow {
                                cells: vec!["sk-deadbeefdeadbeefdeadbeef".into()],
                            }],
                        }),
                    },
                    WidgetNode::CodeDiff {
                        path: Some("f".into()),
                        lines: vec![DiffLine {
                            kind: DiffKind::Added,
                            text: "ghp_deadbeefdeadbeefdeadbeefdeadbeef".into(),
                            old_lineno: None,
                            new_lineno: Some(1),
                        }],
                        show_markers: true,
                    },
                ],
            },
        };
        validate_surface(&mut op, &SurfaceCapabilities::tui(), &config_with(true)).unwrap();
        let text = leaf(&op);
        assert!(!text.contains("sk-deadbeef"), "{text}");
        assert!(!text.contains("ghp_deadbeef"), "{text}");
    }

    #[test]
    fn out_of_bounds_refuses_before_any_renderer_sees_it() {
        let children: Vec<WidgetNode> = (0..(super::super::MAX_SURFACE_NODES + 1))
            .map(|i| WidgetNode::Text {
                text: format!("r{i}"),
                tone: SemanticTone::Default,
                mono: false,
            })
            .collect();
        let mut op = SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "t".into(),
            provenance: Provenance::AgentSynthesis,
            body: WidgetNode::Column { children },
        };
        let err =
            validate_surface(&mut op, &SurfaceCapabilities::tui(), &config_with(true)).unwrap_err();
        assert!(matches!(err, UiRefusal::OutOfBounds { .. }));
    }

    #[test]
    fn interactive_node_on_a_text_only_transport_is_refused() {
        let mut op = SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "t".into(),
            provenance: Provenance::AgentSynthesis,
            body: WidgetNode::Button {
                buttons: vec![ButtonSpec {
                    id: "ok".into(),
                    label: "OK".into(),
                    tone: SemanticTone::Default,
                }],
            },
        };
        let err = validate_surface(
            &mut op,
            &SurfaceCapabilities::text_only(),
            &config_with(true),
        )
        .unwrap_err();
        assert!(matches!(err, UiRefusal::UnsupportedNode { .. }));
    }

    #[test]
    fn refusal_message_names_the_tool_and_the_rule() {
        let refusal = UiRefusal::OutOfBounds {
            detail: "surface has 412 widgets; the limit is 200".into(),
        };
        let msg = refusal_message("ui_render_surface", &refusal);
        assert!(msg.starts_with("ui_render_surface: "), "{msg}");
        assert!(msg.contains("412 widgets"), "{msg}");
    }

    #[test]
    fn model_provenance_is_always_synthesis() {
        // The tool must never stamp ToolVerified on the model's behalf.
        assert!(matches!(MODEL_PROVENANCE, Provenance::AgentSynthesis));
    }

    #[test]
    fn ui_settings_default_is_enabled() {
        // G7: the section defaults ON, so an existing config that predates
        // `[ui]` keeps working without an edit.
        assert!(UiSettings::default().generative_enabled);
        let parsed: UiSettings = toml::from_str("").unwrap_or_default();
        assert!(parsed.generative_enabled);
    }
}
