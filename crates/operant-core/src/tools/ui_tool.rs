//! The two Surface IR tools — `ui_render_surface` and `ui_request_input`.
//!
//! Plan: `docs/plans/2026-11-18-composable-generative-ui-surface-ir-for-the-tui-and-the-gate.md`
//! (M2). The cost line is fixed: two `OperantTool` impls, zero new crates,
//! zero new DB tables, one config boolean (`[ui] generative_enabled`).
//!
//! Both tools are GOVERNED. Each builds a [`SurfaceOp`], runs
//! [`crate::ui::gate::validate_surface`] (kill switch → depth/count bounds →
//! capability match → secret scrub), and only then acts on the op. A refusal
//! is an `Err` result whose message names the rule — never a silently
//! dropped card. The tool builds the op, the gate validates it, the TUI
//! renders it; that ordering is the whole point of review F8: a registry
//! interceptor would run either before the op exists or after the card is
//! already on screen, and both are too late to refuse anything.
//!
//! `ui_render_surface` NEVER suspends: the op is emitted as an
//! [`AgentEvent::SurfaceRendered`] (the variant every host drains) and the
//! tool returns the op's plain-text fallback for the model. The TUI renders
//! the op as a card row from the event — the transcript keeps a structured
//! card, not a text dump.
//!
//! `ui_request_input` DOES suspend: the op is validated, then handed to the
//! process-global surface-request channel ([`crate::ui::request`]) that a
//! host UI (TUI/gateway) drains. The tool awaits the oneshot reply — the
//! same machinery `clarify` uses — and returns the structured answer
//! (`{ "kind": "choice", "choice_id": ... }` etc.) to the model. In the
//! hostless (one-shot CLI) case, no host is attached: the tool degrades to
//! a result that says so and suggests `clarify`, rather than hanging on a
//! channel nobody reads.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

use crate::agent::AgentEvent;
use crate::tools::{OperantTool, ToolContext, ToolResult};
use crate::ui::gate;
use crate::ui::plain::{self, RenderProvenance};
use crate::ui::request::{self, SurfaceRequest};
use crate::ui::{Provenance, SurfaceCapabilities, SurfaceOp, WidgetNode};

/// How long the request-input tool waits for a host UI answer before
/// degrading.
///
/// There are TWO hosts now, and each bounds a hung prompt in its own way:
///
///   * the TUI drains into `ask_user_dialog`, which carries NO timeout of its
///     own -- so THIS is the only bound on that path, and it is what stops a
///     prompt the operator walked away from wedging the turn forever;
///   * the gateway receiver runs a 120 s timer and answers "dismissed" when it
///     fires, so on that path this 300 s is the outer backstop it was always
///     documented as. It was NOT before iter-2640: the gateway registered no
///     receiver at all, so `try_send_surface_request` returned `None` and the
///     tool failed fast without ever reaching this await.
///
/// `operant run` is hostless by design and still takes the fail-fast path.
/// `surface_host_receivers.rs` pins the host set: a new host must be added
/// there, next to whatever bound it gives its own prompts.
const SURFACE_REPLY_TIMEOUT: Duration = Duration::from_secs(300);

// ─────────────────────────────────────────────────────────────────────────
// Arguments
// ─────────────────────────────────────────────────────────────────────────

/// The widget a request-input card carries. This mirrors
/// [`WidgetNode`]'s interactive variants; it is deserialized from the
/// model's JSON, so it must not be an enum of the full IR — just the shapes
/// the tool accepts.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RequestWidget {
    /// One question, one of a list of choices.
    Choice {
        /// The question text.
        question: String,
        /// The choices. Each has an `id` (returned to the model) and a
        /// `label` (shown to the user).
        choices: Vec<RequestChoice>,
    },
    /// A yes/no confirmation.
    Confirm {
        /// The question text.
        question: String,
        /// Default answer when the user just hits Enter.
        #[serde(default)]
        default: bool,
    },
    /// A free-text field.
    Text {
        /// The field's label.
        label: String,
        /// Default value, when the field should be pre-filled.
        #[serde(default)]
        value: String,
        /// Placeholder shown when empty.
        #[serde(default)]
        placeholder: Option<String>,
        /// Whether the field is masked (a secret).
        #[serde(default)]
        masked: bool,
    },
}

/// One choice of a [`RequestWidget::Choice`].
#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RequestChoice {
    /// Stable id, returned to the model when chosen.
    id: String,
    /// The label the user reads.
    label: String,
    /// Optional second line (cost, consequence, file path).
    #[serde(default)]
    description: Option<String>,
}

impl RequestWidget {
    /// Turn the request payload into the widget tree the op carries.
    fn into_node(self) -> WidgetNode {
        match self {
            Self::Choice { question, choices } => WidgetNode::ChoicePicker {
                question,
                choices: choices
                    .into_iter()
                    .map(|c| crate::ui::ChoiceOption {
                        id: c.id,
                        label: c.label,
                        description: c.description,
                    })
                    .collect(),
            },
            Self::Confirm { question, default } => WidgetNode::Confirm { question, default },
            Self::Text {
                label,
                value,
                placeholder,
                masked,
            } => WidgetNode::TextField {
                label,
                value,
                placeholder,
                masked,
            },
        }
    }

    /// One line of intent for the card's title / the request header.
    fn intent(&self) -> String {
        match self {
            Self::Choice { question, .. } => question.clone(),
            Self::Confirm { question, .. } => question.clone(),
            Self::Text { label, .. } => label.clone(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// `ui_render_surface`
// ─────────────────────────────────────────────────────────────────────────

/// Arguments for [`UiRenderSurfaceTool`].
#[derive(Debug, Deserialize, JsonSchema)]
struct RenderSurfaceArgs {
    /// Stable identity, so later ops (`ui_render_surface` again) can address
    /// this surface. Required on create.
    surface_id: String,
    /// One line, shown as the card title.
    intent: String,
    /// Where the content came from. One of
    /// `{"kind": "agent_synthesis"}` (the model invented the content — the
    /// card renders `[unverified]`, and this is the default),
    /// `{"kind": "user_supplied"}`, or
    /// `{"kind": "tool_verified", "tool_name": "web_search"}` (a tool
    /// measured it).
    #[serde(default)]
    provenance: RequestProvenance,
    /// The widget tree. Either a single widget object (a card, a table, …)
    /// or, for a table, an array of row objects (each row is `{"cell": ...}`
    /// keyed by the `columns` entries) — the TOON rows the plan allows.
    body: WidgetBody,
}

/// How a render op is stamped. Written by the model as the IR's own tagged
/// provenance shape (`{"kind": "tool_verified", "tool_name": ...}`), kept
/// as raw JSON here and parsed through [`Provenance`]'s real deserializer.
///
/// Written this way because `Provenance` carries no `JsonSchema` derive, and
/// hand-writing a schema for it would be the second copy of the vocabulary
/// M1's rules forbid. The default is `agent_synthesis`, the honest one: the
/// model invented the content.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(transparent)]
struct RequestProvenance(Value);

impl Default for RequestProvenance {
    fn default() -> Self {
        Self(serde_json::json!({ "kind": "agent_synthesis" }))
    }
}

impl RequestProvenance {
    /// Parse into the IR's own provenance type. A malformed value falls back
    /// to `agent_synthesis` rather than failing the call: an unstamped surface
    /// is honestly unverified, which is the safe direction.
    fn into_provenance(self) -> Provenance {
        serde_json::from_value(self.0).unwrap_or(Provenance::AgentSynthesis)
    }
}

/// What a render body parsed into: a widget tree, or the table shorthand the
/// plan's TOON rule allows (column headers + row objects).
#[derive(Debug)]
enum BodyForm {
    /// A single widget (the tree's root).
    Node(WidgetNode),
    /// A table: its headers and its rows (each row already flattened to
    /// cells, in the columns' order).
    Table {
        /// Column headers, in order.
        columns: Vec<String>,
        /// Rows, each one already a cell vector in `columns` order.
        rows: Vec<Vec<String>>,
    },
}

/// Parse a render body into the IR's own widget vocabulary.
///
/// The IR is the ONE authority on what a widget is: the tree is deserialized
/// by [`WidgetNode`]'s own `Deserialize`, so a variant the IR gains is a
/// variant this tool accepts, with no second list to update. The table
/// shorthand is handled first because it is an AGGREGATE (headers + rows),
/// not a widget — the model writes it as data, not as a nested node.
///
/// `Err` is the model-facing reason, not a typed error: the caller is a tool
/// whose whole failure mode is "tell the model what to fix".
fn parse_widget_body(body: Value) -> Result<BodyForm, ToolResult> {
    let object = match body.as_object() {
        Some(object) => object,
        None => {
            return Err(ToolResult::error(
                "ui_render_surface",
                "body must be a widget object (for example {\"node\": \"table\", \
                 \"columns\": [...], \"rows\": [[...]]}) or a table's \
                 {\"columns\": [...], \"rows\": [{...}]} data form",
            ));
        }
    };

    // The table data form: headers plus row OBJECTS keyed by those headers.
    // This is the TOON shape the plan allows, and it is recognised by the
    // shape rather than by a tag, because a table written as a real `Table`
    // widget carries its columns as objects and no `rows` of raw strings.
    if let (Some(columns), Some(rows)) = (
        object.get("columns").and_then(Value::as_array),
        object.get("rows").and_then(Value::as_array),
    ) && !columns.is_empty()
        && columns.iter().all(|c| c.is_string())
        && rows.iter().all(|r| r.is_object() || r.is_array())
        && object.get("node").is_none()
    {
        let headers: Vec<String> = columns
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect();
        let mut out_rows = Vec::with_capacity(rows.len());
        for row in rows {
            let cells: Vec<String> = match row {
                // An object row: cells come out in the HEADERS' order, so a
                // model that writes the keys in any order still lines up.
                Value::Object(map) => headers
                    .iter()
                    .map(|header| map.get(header).map(cell_to_string).unwrap_or_default())
                    .collect(),
                // An array row: already positional, and a ragged row is
                // padded by the renderer rather than refused (M1's rule).
                Value::Array(cells) => cells.iter().map(cell_to_string).collect(),
                _ => unreachable!("the shape guard above admitted objects and arrays only"),
            };
            out_rows.push(cells);
        }
        return Ok(BodyForm::Table {
            columns: headers,
            rows: out_rows,
        });
    }

    // Everything else is a widget in the IR's own vocabulary.
    let node: WidgetNode = serde_json::from_value(Value::Object(object.clone())).map_err(|e| {
        ToolResult::error(
            "ui_render_surface",
            format!(
                "body is not a valid widget: {e}. The node kinds are row, column, \
                 card, tabs, list, divider, text, badge, key_value_grid, table, \
                 bar_chart, progress, code_diff, status_timeline, button, \
                 choice_picker, text_field, confirm, modal — each tagged with \
                 \"node\" and carrying the fields its kind documents."
            ),
        )
    })?;
    Ok(BodyForm::Node(node))
}

/// One cell of a table row, as the string the IR stores.
///
/// A number or a bool is rendered, not rejected: a model handing over metrics
/// writes `3`, not `"3"`, and refusing that would fail the common case on a
/// technicality the IR does not care about.
fn cell_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The capability set for the transport that will DRAW the card.
///
/// Read from the turn's `ToolContext` metadata, which `tool_context` fills from
/// the agent's transport. Both surface tools used to call
/// `SurfaceCapabilities::tui()` directly, so a gateway turn was validated
/// against the TUI's set and the capability gate was decorative: it answered
/// "can this transport draw a table?" with "yes" on a transport that draws
/// plain text. An absent key is a headless run and gets `text_only`.
fn capabilities_for(context: &ToolContext) -> SurfaceCapabilities {
    SurfaceCapabilities::for_transport(context.get("transport"))
}

/// The body of a render op: the widget tree, as the IR spells it.
///
/// Deserialized as a [`serde_json::Value`] and parsed through the real
/// [`WidgetNode`] deserializer inside `execute`. Two reasons, both the
/// plan's own:
///
/// 1. `WidgetNode` has no `JsonSchema` derive, and hand-writing a schema for
///    a 19-variant recursive type would be a second copy of the vocabulary
///    M1 rules must not drift from — the gate's `check_depth_and_count`
///    already bounds the tree, so the schema does not need to.
/// 2. A tool argument struct that restates every variant would reject a
///    widget the IR supports the moment a variant is added; parsing the JSON
///    through the IR itself means the tool supports exactly what the IR
///    supports, by construction.
///
/// The model is told the widget vocabulary in the tool's DESCRIPTION (the
/// node kinds and their fields), which is where the model reads it anyway;
/// the schema advertises the envelope.
type WidgetBody = Value;

/// Render a surface (table / chart / diff / key-value grid / …) into
/// the transcript as a governed card.
///
/// The model hands over the op; the tool validates it against the gate, then
/// emits it. The TUI (and any host draining `AgentEvent::SurfaceRendered`)
/// draws the card; the tool's result carries the plain-text fallback for the
/// model's own context.
pub struct UiRenderSurfaceTool {
    /// The agent's event channel, when one is wired (the registry passes it
    /// through, exactly as it does for `delegate_task`). `None` in a
    /// hostless process: the tool still returns the fallback, it just has
    /// nobody to hand the card to.
    event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
}

impl UiRenderSurfaceTool {
    /// Build with the agent's event channel (the registration path).
    pub fn new(event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>) -> Self {
        Self { event_tx }
    }
}

#[async_trait]
impl OperantTool for UiRenderSurfaceTool {
    fn name(&self) -> &str {
        "ui_render_surface"
    }

    fn description(&self) -> &str {
        "Render a surface (table, chart, diff, key-value grid, timeline, ...). \
        The content is shown to the user as a structured card. Prefer this over \
        hand-building markdown tables; it is governed (depth/count bounds, \
        secret scrub) and the card stays interactive across turns."
    }

    fn schema(&self) -> crate::schema::ToolSchema {
        crate::schema::ToolSchema::from_type::<RenderSurfaceArgs>(
            "ui_render_surface",
            "Render a governed surface card (table/chart/diff/...) into the transcript",
        )
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: RenderSurfaceArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("ui_render_surface", format!("Invalid arguments: {e}"));
            }
        };

        // Parse the body. A table body (`columns` + `rows`, the TOON rows the
        // plan allows) becomes a Table node; anything else must already BE a
        // widget in the IR's own vocabulary — the IR's parser is the single
        // authority on what a widget is.
        let body: WidgetNode = match parse_widget_body(args.body) {
            Ok(form) => match form {
                BodyForm::Node(node) => node,
                BodyForm::Table { columns, rows } => WidgetNode::Table {
                    columns: columns
                        .into_iter()
                        .map(|header| crate::ui::TableColumn {
                            header,
                            align: crate::ui::ColumnAlign::default(),
                        })
                        .collect(),
                    rows: rows
                        .into_iter()
                        .map(|cells| crate::ui::TableRow { cells })
                        .collect(),
                },
            },
            // The parser's error IS the model-facing ToolResult; return it
            // unchanged rather than rebuilding the message here.
            Err(result) => return result,
        };

        let provenance = args.provenance.into_provenance();
        let mut op = SurfaceOp::CreateSurface {
            surface_id: args.surface_id,
            intent: args.intent,
            provenance,
            body,
        };

        // The gate. Fail-closed: every refusal is a typed rule the model can
        // act on, never a silent drop. The capabilities are the DRAWING
        // transport's, read off the turn, not an assumed TUI.
        let caps = capabilities_for(&context);
        let config = crate::config::runtime_config();
        if let Err(refusal) = gate::validate_surface(&mut op, &caps, &config) {
            return ToolResult::error(
                "ui_render_surface",
                gate::refusal_message("ui_render_surface", &refusal),
            );
        }

        // Render the plain-text fallback (what the model sees in its own
        // context). `render_plain` badges agent-synthesized surfaces with
        // `[unverified]` itself — the caller does not add it.
        let provenance_flag = RenderProvenance::from_synthesized(op.is_synthesized());
        let fallback = plain::render_plain(
            &op.widgets_column(),
            plain::FALLBACK_MODEL_WIDTH,
            provenance_flag,
        );

        // Emit for every host draining the event channel. The TUI renders a
        // card row; the gateway can carry it as an attachment. A hostless
        // process simply has no receiver (the event is a no-op there) — the
        // fallback below is what keeps the model informed.
        emit_surface_rendered(&self.event_tx, &op);

        // The result: the op (structured, for a host or a reloaded session to
        // re-render from) plus the plain fallback (what the model reads).
        //
        // The op is included because the emitted `AgentEvent` is a LIVE signal:
        // a session reloaded after this card renders does not replay it, so
        // without the op here the reloaded transcript has the plain text and no
        // way back to the card (plan review F18). Scrubbing the SERIALIZED
        // fallback too catches a secret split across two table cells, which the
        // leaf walk in `gate::validate_surface` cannot see (gate.rs M4).
        let fallback = crate::redaction::redact_sensitive_text(&fallback);
        ToolResult::success(
            "ui_render_surface",
            serde_json::json!({
                "surface_id": op.surface_id(),
                "fallback": fallback,
                "op": &op,
            }),
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────
// `ui_request_input`
// ─────────────────────────────────────────────────────────────────────────

/// Arguments for [`UiRequestInputTool`].
#[derive(Debug, Deserialize, JsonSchema)]
struct RequestInputArgs {
    /// Stable identity for the surface this card belongs to.
    surface_id: String,
    /// The interactive card: `choice` (a question + choices), `confirm`
    /// (yes/no), or `text` (a field).
    #[serde(flatten)]
    widget: RequestWidget,
}

/// Tool: ask the user for input through a governed interactive card.
///
/// The op is validated, then handed to the host UI over the
/// surface-request channel; the tool awaits the answer and returns it as
/// structured JSON. In a hostless process it degrades with a hint to use
/// `clarify` instead — never a hang.
pub struct UiRequestInputTool;

#[async_trait]
impl OperantTool for UiRequestInputTool {
    fn name(&self) -> &str {
        "ui_request_input"
    }

    fn description(&self) -> &str {
        "Ask the user for input through an interactive card (choice / confirm / \
        text field). Use for a structured choice when the answer is one of a \
        known set, or a yes/no gate. The card suspends the turn until the user \
        answers."
    }

    fn schema(&self) -> crate::schema::ToolSchema {
        crate::schema::ToolSchema::from_type::<RequestInputArgs>(
            "ui_request_input",
            "Ask the user for input via a governed choice/confirm/text card",
        )
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: RequestInputArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("ui_request_input", format!("Invalid arguments: {e}"));
            }
        };

        let intent = args.widget.intent();
        let body = args.widget.into_node();

        let mut op = SurfaceOp::CreateSurface {
            surface_id: args.surface_id,
            intent,
            provenance: Provenance::UserSupplied,
            body,
        };

        // The gate runs BEFORE any host sees the card: a refusal is returned
        // to the model and the host is never disturbed.
        let caps = capabilities_for(&context);
        let config = crate::config::runtime_config();
        if let Err(refusal) = gate::validate_surface(&mut op, &caps, &config) {
            return ToolResult::error(
                "ui_request_input",
                gate::refusal_message("ui_request_input", &refusal),
            );
        }

        // Unattended runs (cron) have no interactive user: fail fast with the
        // same rule `clarify` uses, so a scheduled job does not wait on a
        // human that is not there.
        if context.metadata.get("unattended").map(String::as_str) == Some("true") {
            return ToolResult::error(
                "ui_request_input",
                "unattended run: no interactive user can answer — proceed with your \
                 best judgment and do not wait",
            );
        }

        // Hand the op to the host UI. The question is the intent line; the
        // body is the widget the host renders.
        let question = match &op {
            SurfaceOp::CreateSurface { intent, .. } => intent.clone(),
            _ => unreachable!("ui_request_input builds a CreateSurface op"),
        };
        let request = SurfaceRequest {
            surface_id: op.surface_id().to_string(),
            // The host needs to know WHICH chat asked: the gateway serves many
            // sessions from one process, and a card delivered to the wrong one
            // leaves the asking turn blocked on an answer nobody was shown
            // (the iter-2528 H-3 failure, for `clarify`).
            session_id: context.resolved_session_id().map(str::to_string),
            question,
            body: match &op {
                SurfaceOp::CreateSurface { body, .. } => body.clone(),
                _ => unreachable!("ui_request_input builds a CreateSurface op"),
            },
            op: op.clone(),
            reply_tx: tokio::sync::oneshot::channel().0,
        };

        let Some(reply_rx) = request::try_send_surface_request(request) else {
            // Hostless (one-shot CLI): degrade honestly. The model must not
            // be told an answer it cannot get; it must be told to use the
            // text channel instead.
            return ToolResult::error(
                "ui_request_input",
                "no interactive surface is attached in this session — ask the user \
                 in plain text (or use the `clarify` tool) instead",
            );
        };

        // Await the answer. The 300s timeout is the only bound: the only
        // host that reaches this await is the TUI, whose `ask_user_dialog`
        // carries no timeout of its own, so a lost oneshot would otherwise
        // wedge the turn forever. It is NOT a backstop for some longer
        // dialog timeout -- see `SURFACE_REPLY_TIMEOUT`, which
        // `tests/surface_host_receivers.rs` guards.
        let answer = match tokio::time::timeout(SURFACE_REPLY_TIMEOUT, reply_rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => {
                return ToolResult::error("ui_request_input", "the user dismissed the prompt");
            }
            Err(_) => {
                return ToolResult::error(
                    "ui_request_input",
                    "timed out waiting for the user's answer",
                );
            }
        };

        // Structure the reply so the model can branch on it without parsing
        // free text. The host answers with `{ "kind": ..., ... }`; the tool
        // passes it through with the surface id attached.
        let mut payload = match answer {
            Value::Object(map) => Value::Object(map),
            other => serde_json::json!({ "kind": "text", "value": other }),
        };
        if let Some(map) = payload.as_object_mut() {
            map.insert(
                "surface_id".to_string(),
                Value::String(op.surface_id().to_string()),
            );
        }
        ToolResult::success("ui_request_input", payload)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Event emission
// ─────────────────────────────────────────────────────────────────────────

/// Hand the op to every host draining the agent event channel.
///
/// Best-effort and non-blocking (`try_send`, the same rule `delegate_task`
/// uses): a full or slow host channel must never stall the turn, and a
/// hostless process has no sender to begin with.
fn emit_surface_rendered(event_tx: &Option<tokio::sync::mpsc::Sender<AgentEvent>>, op: &SurfaceOp) {
    let Some(tx) = event_tx else {
        return;
    };
    let _ = tx.try_send(AgentEvent::SurfaceRendered {
        surface_id: op.surface_id().to_string(),
        op: op.clone(),
    });
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;
    use serde_json::json;

    /// Set `[ui] generative_enabled` for one test and RESTORE the previous
    /// value on drop, holding the crate's runtime-config test lock for the
    /// whole body.
    ///
    /// The flag lives in the process-global runtime config, so a test that
    /// flips it and does not restore it poisons every test that runs after it
    /// in the same process — which is exactly how `render_accepts_toon_rows`
    /// started failing on a gate it never touched. Restoring is not enough on
    /// its own: `config_tool`'s tests install a fresh default config and
    /// `skill_bundle`'s install a crafted one, so a reader between our set and
    /// our restore would still see the wrong value. The lock is what makes the
    /// window closed rather than merely narrow.
    struct GenEnabledGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        was: bool,
    }

    impl GenEnabledGuard {
        fn set(enabled: bool) -> Self {
            let lock = crate::config::runtime_config_test_lock();
            let was = crate::config::runtime_config().ui.generative_enabled;
            let mut config = crate::config::runtime_config();
            config.ui.generative_enabled = enabled;
            crate::config::install_runtime_config(config);
            Self { _lock: lock, was }
        }
    }

    impl Drop for GenEnabledGuard {
        fn drop(&mut self) {
            let mut config = crate::config::runtime_config();
            config.ui.generative_enabled = self.was;
            crate::config::install_runtime_config(config);
            // `_lock` releases after this, so the restore is inside the
            // critical section it protects.
        }
    }

    fn render_args(body: Value) -> Value {
        json!({
            "surface_id": "s1",
            "intent": "a table",
            "body": body,
        })
    }

    #[tokio::test]
    async fn render_accepts_a_single_node() {
        let _guard = GenEnabledGuard::set(true);
        let tool = UiRenderSurfaceTool::new(None);
        let result = tool
            .execute(
                render_args(json!({
                    "node": "table",
                    "columns": [{"header": "k"}],
                    "rows": [{"cells": ["v"]}],
                })),
                ToolContext::default().with_metadata("transport", "tui"),
            )
            .await;
        assert!(result.success, "{}", result.error.unwrap_or_default());
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["surface_id"], "s1");
        // AgentSynthesis (the default) badges the fallback.
        assert!(
            parsed["fallback"]
                .as_str()
                .unwrap()
                .contains("[unverified]")
        );
    }

    #[tokio::test]
    async fn render_accepts_toon_rows() {
        let _guard = GenEnabledGuard::set(true);
        let tool = UiRenderSurfaceTool::new(None);
        let result = tool
            .execute(
                render_args(json!({
                    "columns": ["name", "qty"],
                    "rows": [
                        {"name": "apple", "qty": "3"},
                        {"name": "pear", "qty": "7"},
                    ],
                })),
                ToolContext::default().with_metadata("transport", "tui"),
            )
            .await;
        assert!(result.success, "{}", result.error.unwrap_or_default());
    }

    #[tokio::test]
    async fn render_refuses_when_disabled() {
        // Flip the kill switch, then hand the SAME tool a legal op. The guard
        // restores the flag when this test ends.
        let _guard = GenEnabledGuard::set(false);
        let tool = UiRenderSurfaceTool::new(None);
        let result = tool
            .execute(
                render_args(json!({"node": "divider"})),
                ToolContext::default().with_metadata("transport", "tui"),
            )
            .await;
        assert!(!result.success, "the gate must refuse a disabled surface");
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("disabled"),
            "the refusal names the rule"
        );
    }

    #[tokio::test]
    async fn render_refuses_an_oversized_tree() {
        let _guard = GenEnabledGuard::set(true);
        // MAX_SURFACE_NODES+1 siblings in a column — the bound the gate
        // enforces so the schema cannot blow a provider's recursion limit.
        let children: Vec<Value> = (0..(crate::ui::MAX_SURFACE_NODES + 1))
            .map(|i| json!({"node": "text", "text": format!("row {i}")}))
            .collect();
        let tool = UiRenderSurfaceTool::new(None);
        let result = tool
            .execute(
                render_args(json!({
                    "node": "column",
                    "children": children,
                })),
                ToolContext::default().with_metadata("transport", "tui"),
            )
            .await;
        assert!(!result.success, "an over-wide tree must be refused");
        let err = result.error.unwrap_or_default();
        assert!(
            err.contains("limit") || err.contains("200") || err.contains("bound"),
            "the refusal names the bound: {err}"
        );
    }

    #[tokio::test]
    async fn request_choices_reach_the_host_and_the_answer_returns() {
        // Drive the tool through the process-global channel the way a host
        // would: register a receiver, execute, answer.
        //
        // The channel is a process-global `OnceLock`, and `ui/request.rs` has
        // a test that registers one too — both live in this crate's test
        // binary, so they must share the module's lock or each steals the
        // other's request (measured: this test read `s1` where it wrote `s2`).
        let _lock = crate::ui::request::tests::SURFACE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        // The channel is a process-global set-once `OnceLock`, so if a host
        // was already registered (the sibling `ui::request` test) this test
        // cannot observe its own request and must not fail the binary: the
        // strict registration assertion lives there.
        if request::set_surface_request_sender(tx).is_err() {
            return;
        }
        let tool = UiRequestInputTool;
        let handle = tokio::spawn(async move {
            tool.execute(
                json!({
                    "surface_id": "s2",
                    "kind": "choice",
                    "question": "Pick one",
                    "choices": [
                        {"id": "a", "label": "A"},
                        {"id": "b", "label": "B"},
                    ],
                }),
                ToolContext::default().with_metadata("transport", "tui"),
            )
            .await
        });
        let received = rx.recv().await.expect("the host receives the request");
        assert_eq!(received.surface_id, "s2");
        assert_eq!(received.question, "Pick one");
        received
            .reply_tx
            .send(json!({"kind": "choice", "choice_id": "b"}))
            .expect("the reply reaches the tool");
        let result = handle.await.expect("the tool task joins");
        assert!(result.success, "{}", result.error.unwrap_or_default());
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["choice_id"], "b");
        assert_eq!(parsed["surface_id"], "s2");
    }

    #[tokio::test]
    async fn request_refuses_unattended() {
        let tool = UiRequestInputTool;
        let context = ToolContext::default()
            .with_metadata("transport", "tui")
            .with_metadata("unattended", "true");
        let result = tool
            .execute(
                json!({
                    "surface_id": "s3",
                    "kind": "confirm",
                    "question": "Continue?",
                }),
                context,
            )
            .await;
        assert!(!result.success, "unattended runs must fail fast");
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("unattended")
        );
    }
}
