//! The Surface IR — the governed document the agent emits instead of
//! hand-writing Ratatui, Slack or Telegram markup.
//!
//! The plan is `docs/plans/2026-11-18-composable-generative-ui-surface-ir-for-the-tui-and-the-gate.md`
//! (M1). This module holds the types and NOTHING else: the governance gate
//! is [`gate`], the two agent tools are [`crate::tools::ui_tool`], the TUI
//! renderer lives in `operant-cli/src/tui/operant_ui/ui_surface.rs`, and the
//! gateway rasterizer in `operant-core/src/gateway/`. Splitting them keeps
//! this file auditable and keeps `operant-core` free of `ratatui` (which
//! only `operant-cli` links — the crate-direction constraint the plan's
//! review F4 established).
//!
//! Three invariants are enforced here rather than documented and hoped for:
//!
//! 1. [`SemanticTone`] is the ONLY colour surface. No variant carries a
//!    `Color`, a hex string, or any other raw colour field; the TUI maps
//!    tones onto the ANSI palette and the PNG mapper onto high-contrast RGB.
//! 2. A tree is depth-bounded ([`MAX_SURFACE_DEPTH`]) and node-count-bounded
//!    ([`MAX_SURFACE_NODES`]), checked by [`check_depth_and_count`] in the
//!    gate and again by the tool that parses the model's JSON, so the
//!    serialized `WidgetNode` schema cannot blow a provider's JSON-schema
//!    recursion limit.
//! 3. [`Provenance::AgentSynthesis`] renders an `[unverified]` badge. The
//!    renderer appends it from the op's provenance — no node can opt out,
//!    because the badge is added by the consumer and not carried as a
//!    suppressible field.

pub mod gate;
pub mod plain;
pub mod request;

use serde::{Deserialize, Serialize};

/// Maximum nesting depth of a [`WidgetNode`] tree.
///
/// The root counts as depth 1, so `depth <= MAX_SURFACE_DEPTH` is the whole
/// check. Bounded because the type is recursive and its serialized form is
/// handed to providers as part of a tool schema (M1 rule 4).
pub const MAX_SURFACE_DEPTH: usize = 8;

/// Maximum number of [`WidgetNode`]s in one tree.
///
/// Counts every node, not just containers, so a fan-out of 200 sibling
/// `Text` rows is refused exactly like a 200-deep chain.
pub const MAX_SURFACE_NODES: usize = 200;

/// Why a surface was refused. Every variant names the rule that failed so
/// the model receives an actionable string rather than "invalid".
///
/// Fail-closed is the house rule at every governance seam: a refusal is the
/// typed reason, never a silently dropped surface.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UiRefusal {
    /// `[ui] generative_enabled = false` (the operator's kill switch, G7).
    #[error("generative UI is disabled by the operator ([ui] generative_enabled = false)")]
    Disabled,
    /// The transport cannot show this kind of node.
    #[error("this transport cannot render {node} (capabilities: {capabilities})")]
    UnsupportedNode {
        /// The node kind that was asked for.
        node: String,
        /// The capability set of the active transport, rendered.
        capabilities: String,
    },
    /// A bound from [`MAX_SURFACE_DEPTH`] or [`MAX_SURFACE_NODES`] was hit.
    #[error("{detail}")]
    OutOfBounds {
        /// Which bound, and by how much.
        detail: String,
    },
    /// The op could not be deserialized into the IR.
    #[error("surface is not valid Surface IR: {detail}")]
    Malformed {
        /// The deserializer's complaint.
        detail: String,
    },
}

/// One governance verdict. `Ok` carries the (possibly rewritten) op: the
/// gate scrubs every string leaf, so the op it returns is NOT byte-identical
/// to the one it received, and callers must render the returned value.
pub type SurfaceVerdict<T> = Result<T, UiRefusal>;

/// What a transport can actually show.
///
/// Built per turn from the live transport and injected through the tool
/// context — never from a process-global, so two concurrent gateway chats
/// cannot overwrite each other's capability set (the RT-534 turn-riding
/// argument, applied to capabilities).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SurfaceCapabilities {
    /// Widgets may be drawn (TUI true; every messaging adapter false).
    pub inline_widgets: bool,
    /// The transport carries inline buttons that a user can press.
    pub native_buttons: bool,
    /// The transport can open a modal / dialog surface.
    pub modals: bool,
    /// The transport can display an image attachment.
    pub images: bool,
    /// Largest image byte count the transport accepts.
    pub max_image_bytes: usize,
}

impl SurfaceCapabilities {
    /// The TUI: the full set.
    pub const fn tui() -> Self {
        Self {
            inline_widgets: true,
            native_buttons: true,
            modals: true,
            images: false,
            max_image_bytes: 0,
        }
    }

    /// Telegram, a direct message: images and native buttons, no widgets.
    pub const fn telegram_dm() -> Self {
        Self {
            inline_widgets: false,
            native_buttons: true,
            modals: false,
            images: true,
            max_image_bytes: 10 * 1024 * 1024,
        }
    }

    /// A text-only transport (Discord, Slack, WhatsApp, email, SMS): the
    /// surface degrades to its plain-text fallback.
    pub const fn text_only() -> Self {
        Self {
            inline_widgets: false,
            native_buttons: false,
            modals: false,
            images: false,
            max_image_bytes: 0,
        }
    }

    /// Render the capability set as the short string [`UiRefusal::UnsupportedNode`]
    /// carries.
    pub fn describe(self) -> String {
        let mut parts = Vec::new();
        if self.inline_widgets {
            parts.push("widgets");
        }
        if self.native_buttons {
            parts.push("buttons");
        }
        if self.modals {
            parts.push("modals");
        }
        if self.images {
            parts.push("images");
        }
        if parts.is_empty() {
            "text only".to_string()
        } else {
            parts.join(",")
        }
    }
}

/// Where a value came from. The honesty layer: a number the agent invented
/// is not the same as a number a tool measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    /// Produced by a named tool, optionally from a source document.
    ToolVerified {
        /// The tool that produced the value.
        tool_name: String,
        /// Where the tool read it from, when it can name a source.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_uri: Option<String>,
    },
    /// Typed by the user.
    UserSupplied,
    /// Synthesized by the model. Renders the `[unverified]` badge.
    AgentSynthesis,
}

/// The only colour vocabulary in the IR. Deliberately not a `Color`: a raw
/// colour field would let a model bypass the palette the renderers are
/// tuned for, and the two renderers disagree about the palette by design
/// (TUI ANSI vs. PNG high-contrast RGB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticTone {
    /// The transport's default foreground.
    #[default]
    Default,
    /// De-emphasized: secondary text, footnotes, units.
    Muted,
    /// The transport's accent (brand / highlight).
    Accent,
    /// A good outcome.
    Success,
    /// Something to watch.
    Warning,
    /// Something went wrong.
    Danger,
}

/// The four operations a surface is built from.
///
/// The vocabulary is OpenBot's, collapsed to the four that this tree needs:
/// create the surface, append widgets, replace its data, seal it. There is
/// no `Destroy`: a sealed surface stays in the transcript so a reloaded
/// session re-renders the card instead of dumping raw JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SurfaceOp {
    /// Open a surface.
    CreateSurface {
        /// Stable identity, so later ops address this surface.
        surface_id: String,
        /// What the surface is for, one line, shown as the card title.
        intent: String,
        /// Where the content came from.
        provenance: Provenance,
        /// The widget tree.
        body: WidgetNode,
    },
    /// Append widgets to an open surface.
    UpdateComponents {
        /// The surface to append to.
        surface_id: String,
        /// The widgets to append, in order.
        components: Vec<WidgetNode>,
    },
    /// Replace an open surface's data model (a `Table`'s rows, a chart's
    /// series).
    UpdateDataModel {
        /// The surface to update.
        surface_id: String,
        /// The widget whose data is being replaced.
        body: WidgetNode,
    },
    /// Close a surface: no further updates are accepted.
    SealSurface {
        /// The surface to seal.
        surface_id: String,
    },
}

impl SurfaceOp {
    /// The surface this op addresses.
    pub fn surface_id(&self) -> &str {
        match self {
            Self::CreateSurface { surface_id, .. }
            | Self::UpdateComponents { surface_id, .. }
            | Self::UpdateDataModel { surface_id, .. }
            | Self::SealSurface { surface_id } => surface_id,
        }
    }

    /// Every widget in this op, in render order.
    pub fn widgets(&self) -> Vec<&WidgetNode> {
        match self {
            Self::CreateSurface { body, .. } | Self::UpdateDataModel { body, .. } => {
                vec![body]
            }
            Self::UpdateComponents { components, .. } => components.iter().collect(),
            Self::SealSurface { .. } => Vec::new(),
        }
    }

    /// Whether this op is the one that creates the surface.
    pub fn is_create(&self) -> bool {
        matches!(self, Self::CreateSurface { .. })
    }
}

/// A labelled pair, the unit of a `KeyValueGrid` and of a metric row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyValue {
    /// The label.
    pub key: String,
    /// The value, already formatted.
    pub value: String,
    /// Optional emphasis on the value.
    #[serde(default)]
    pub tone: SemanticTone,
}

/// One row of a [`WidgetNode::Table`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    /// The row's cells, left to right. Ragged rows are padded by the
    /// renderer rather than refused, because a model that drops a trailing
    /// empty cell is not a security event.
    pub cells: Vec<String>,
}

/// One column of a [`WidgetNode::Table`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableColumn {
    /// The header text.
    pub header: String,
    /// Optional alignment hint for the renderer.
    #[serde(default)]
    pub align: ColumnAlign,
}

/// Horizontal alignment of a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnAlign {
    /// Left.
    #[default]
    Left,
    /// Right — the right choice for numbers.
    Right,
    /// Center.
    Center,
}

/// One bar of a [`WidgetNode::BarChart`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarDatum {
    /// The bar's label.
    pub label: String,
    /// The bar's magnitude. Negative values render left of the axis.
    pub value: f64,
}

/// A chart kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    /// Horizontal bars — the default, because labels stay readable.
    #[default]
    Bar,
    /// A sparkline: one point per value, no axes.
    Sparkline,
}

/// One row of a [`WidgetNode::CodeDiff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    /// Unchanged context.
    Context,
    /// An added line.
    Added,
    /// A removed line.
    Removed,
    /// A hunk header (`@@ ... @@`).
    Hunk,
}

/// A diff line: its kind plus its text (without the leading `+`/`-`/` `).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    /// What kind of line this is.
    pub kind: DiffKind,
    /// The line's text.
    pub text: String,
    /// The old-file line number, when known.
    #[serde(default)]
    pub old_lineno: Option<u32>,
    /// The new-file line number, when known.
    #[serde(default)]
    pub new_lineno: Option<u32>,
}

/// One step of a [`WidgetNode::StatusTimeline`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineStep {
    /// What happened.
    pub label: String,
    /// The outcome.
    pub tone: SemanticTone,
    /// Optional timestamp / duration text.
    #[serde(default)]
    pub detail: Option<String>,
}

/// One choice offered by a [`WidgetNode::ChoicePicker`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    /// Stable id, returned to the agent when chosen.
    pub id: String,
    /// The label the user reads.
    pub label: String,
    /// Optional second line (cost, consequence, file path).
    #[serde(default)]
    pub description: Option<String>,
}

/// One button of a [`WidgetNode::Button`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ButtonSpec {
    /// Stable id, carried back on the callback.
    pub id: String,
    /// The button's label.
    pub label: String,
    /// Emphasis.
    #[serde(default)]
    pub tone: SemanticTone,
}

/// One node of a surface tree.
///
/// Nineteen variants, grouped: nine read-only (rows 1-9), five data-bearing
/// (10-14), five interactive (15-19). The grouping matters because the gate
/// checks interactivity against [`SurfaceCapabilities`] — a read-only node
/// degrades to text everywhere, an interactive node needs `native_buttons`
/// or `inline_widgets`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum WidgetNode {
    // ── Read-only containers ────────────────────────────────────────────
    /// A horizontal row of children.
    Row {
        /// Children, left to right.
        children: Vec<WidgetNode>,
    },
    /// A vertical stack of children.
    Column {
        /// Children, top to bottom.
        children: Vec<WidgetNode>,
    },
    /// A titled box around one child.
    Card {
        /// The card's title.
        title: String,
        /// The card's content.
        child: Box<WidgetNode>,
        /// Emphasis of the border / title.
        #[serde(default)]
        tone: SemanticTone,
    },
    /// A tabbed container: one visible child at a time.
    Tabs {
        /// Tab labels, in order.
        tabs: Vec<String>,
        /// Which tab is active.
        active: usize,
        /// The active tab's content.
        child: Box<WidgetNode>,
    },
    /// A list of items with optional markers.
    List {
        /// The items, in order.
        items: Vec<ListItem>,
        /// Whether the items are ordered (numbered) or bulleted.
        #[serde(default)]
        ordered: bool,
    },
    /// A horizontal rule.
    Divider,
    // ── Read-only leaves ────────────────────────────────────────────────
    /// A run of text.
    Text {
        /// The text.
        text: String,
        /// Emphasis.
        #[serde(default)]
        tone: SemanticTone,
        /// Whether to render in a monospace face.
        #[serde(default)]
        mono: bool,
    },
    /// A small status pill.
    Badge {
        /// The pill's text.
        label: String,
        /// Emphasis.
        #[serde(default)]
        tone: SemanticTone,
    },
    /// A label/value grid — metrics, properties, summary fields.
    KeyValueGrid {
        /// The pairs, in order.
        pairs: Vec<KeyValue>,
        /// Columns in the grid (1 = stacked).
        #[serde(default = "default_columns")]
        columns: usize,
    },
    // ── Data-bearing ────────────────────────────────────────────────────
    /// A table with an optional header row.
    Table {
        /// Column headers.
        columns: Vec<TableColumn>,
        /// Data rows.
        rows: Vec<TableRow>,
    },
    /// A bar chart or sparkline.
    BarChart {
        /// The chart's title.
        title: String,
        /// The chart kind.
        #[serde(default)]
        kind: ChartKind,
        /// The bars / points.
        data: Vec<BarDatum>,
    },
    /// A labelled progress bar.
    Progress {
        /// The bar's label.
        label: String,
        /// Completion in percent, clamped to 0..=100 by the renderer.
        value: f64,
        /// Optional text shown at the right of the bar.
        #[serde(default)]
        detail: Option<String>,
    },
    /// A unified diff.
    CodeDiff {
        /// Optional file label.
        #[serde(default)]
        path: Option<String>,
        /// The diff lines.
        lines: Vec<DiffLine>,
        /// Whether to prepend `+`/`-`/` ` to each line (default true).
        #[serde(default = "default_true")]
        show_markers: bool,
    },
    /// A vertical timeline of steps.
    StatusTimeline {
        /// The steps, in order.
        steps: Vec<TimelineStep>,
    },
    // ── Interactive ─────────────────────────────────────────────────────
    /// A row of buttons.
    Button {
        /// The buttons, left to right.
        buttons: Vec<ButtonSpec>,
    },
    /// A single-choice picker.
    ChoicePicker {
        /// The prompt shown above the choices.
        question: String,
        /// The offered choices.
        choices: Vec<ChoiceOption>,
    },
    /// A free-text field.
    TextField {
        /// The field's label.
        label: String,
        /// Default contents.
        #[serde(default)]
        value: String,
        /// Placeholder shown when empty.
        #[serde(default)]
        placeholder: Option<String>,
        /// Whether the field is masked (secrets).
        #[serde(default)]
        masked: bool,
    },
    /// A yes/no confirmation.
    Confirm {
        /// The question.
        question: String,
        /// Default answer when the user just hits Enter.
        #[serde(default)]
        default: bool,
    },
    /// A modal container: shown instead of its child until dismissed.
    Modal {
        /// The modal's title.
        title: String,
        /// The modal's content.
        child: Box<WidgetNode>,
    },
}

/// One item of a [`WidgetNode::List`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    /// The item's primary text.
    pub text: String,
    /// Optional secondary line.
    #[serde(default)]
    pub detail: Option<String>,
    /// Emphasis.
    #[serde(default)]
    pub tone: SemanticTone,
}

fn default_columns() -> usize {
    2
}

fn default_true() -> bool {
    true
}

impl WidgetNode {
    /// The node's kind as the wire spells it (the `node` tag).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Row { .. } => "row",
            Self::Column { .. } => "column",
            Self::Card { .. } => "card",
            Self::Tabs { .. } => "tabs",
            Self::List { .. } => "list",
            Self::Divider => "divider",
            Self::Text { .. } => "text",
            Self::Badge { .. } => "badge",
            Self::KeyValueGrid { .. } => "key_value_grid",
            Self::Table { .. } => "table",
            Self::BarChart { .. } => "bar_chart",
            Self::Progress { .. } => "progress",
            Self::CodeDiff { .. } => "code_diff",
            Self::StatusTimeline { .. } => "status_timeline",
            Self::Button { .. } => "button",
            Self::ChoicePicker { .. } => "choice_picker",
            Self::TextField { .. } => "text_field",
            Self::Confirm { .. } => "confirm",
            Self::Modal { .. } => "modal",
        }
    }

    /// Whether this node needs a user interaction to be useful.
    pub fn is_interactive(&self) -> bool {
        matches!(
            self,
            Self::Button { .. }
                | Self::ChoicePicker { .. }
                | Self::TextField { .. }
                | Self::Confirm { .. }
                | Self::Modal { .. }
        )
    }

    /// Direct children, if any. Used by the depth / count checks and the
    /// string-leaf walk; one traversal shared by three callers.
    pub fn children(&self) -> Vec<&WidgetNode> {
        match self {
            Self::Row { children } | Self::Column { children } => children.iter().collect(),
            Self::Card { child, .. } | Self::Tabs { child, .. } | Self::Modal { child, .. } => {
                vec![child]
            }
            _ => Vec::new(),
        }
    }

    /// Mutable children, if any — the string-leaf scrub walk.
    pub fn children_mut(&mut self) -> Vec<&mut WidgetNode> {
        match self {
            Self::Row { children } | Self::Column { children } => children.iter_mut().collect(),
            Self::Card { child, .. } | Self::Tabs { child, .. } | Self::Modal { child, .. } => {
                vec![child]
            }
            _ => Vec::new(),
        }
    }

    /// Whether any node in this subtree is [`Provenance::AgentSynthesis`].
    ///
    /// Only `CreateSurface` and `UpdateComponents`-style containers carry
    /// provenance at the op level, so this checks the op's declared
    /// provenance plus every node — see [`SurfaceOp::is_synthesized`].
    pub fn has_unverified(&self) -> bool {
        false
    }
}

impl SurfaceOp {
    /// Whether the `[unverified]` badge must be appended.
    ///
    /// True when the op's own provenance is `AgentSynthesis`. Node-level
    /// provenance does not exist: the op carries one provenance for the
    /// whole surface, which is what the renderer badges. Keeping it at the
    /// op is deliberate — a per-node override would be a suppressible
    /// field, and M1 rule 3 says it must not be suppressible.
    pub fn is_synthesized(&self) -> bool {
        match self {
            Self::CreateSurface { provenance, .. } => {
                matches!(provenance, Provenance::AgentSynthesis)
            }
            _ => false,
        }
    }

    /// Every string leaf in this op's widgets, in a stable order.
    ///
    /// The gate scrubs these AND the serialized fallback text, because the
    /// scrubber is regex masking and a secret split across two sibling
    /// cells is invisible to a leaf-wise walk (review F8).
    pub fn string_leaves(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for widget in self.widgets() {
            collect_string_leaves(widget, &mut out);
        }
        out
    }

    /// Every string leaf, mutably — the scrub walk.
    pub fn string_leaves_mut(&mut self) -> Vec<&mut String> {
        let mut out = Vec::new();
        let root = match self {
            Self::CreateSurface { body, .. } => std::slice::from_mut(body),
            Self::UpdateDataModel { body, .. } => std::slice::from_mut(body),
            Self::UpdateComponents { components, .. } => components.as_mut_slice(),
            Self::SealSurface { .. } => &mut [],
        };
        for widget in root {
            collect_string_leaves_mut(widget, &mut out);
        }
        out
    }

    /// Depth and node count of this op's tree, for the bounds check.
    pub fn tree_size(&self) -> (usize, usize) {
        let mut nodes = 0usize;
        let mut depth = 0usize;
        for widget in self.widgets() {
            let (d, n) = subtree_size(widget);
            depth = depth.max(d);
            nodes += n;
        }
        (depth, nodes)
    }
}

/// Depth (root = 1) and node count of one subtree.
pub fn subtree_size(node: &WidgetNode) -> (usize, usize) {
    let children = node.children();
    if children.is_empty() {
        return (1, 1);
    }
    let mut depth = 0usize;
    let mut nodes = 1usize;
    for child in children {
        let (d, n) = subtree_size(child);
        depth = depth.max(d);
        nodes += n;
    }
    (depth + 1, nodes)
}

fn collect_string_leaves<'a>(node: &'a WidgetNode, out: &mut Vec<&'a str>) {
    match node {
        WidgetNode::Row { children } | WidgetNode::Column { children } => {
            for child in children {
                collect_string_leaves(child, out);
            }
        }
        WidgetNode::Card {
            title, child, tone, ..
        } => {
            let _ = tone;
            out.push(title);
            collect_string_leaves(child, out);
        }
        WidgetNode::Tabs {
            tabs,
            active,
            child,
        } => {
            let _ = active;
            out.extend(tabs.iter().map(String::as_str));
            collect_string_leaves(child, out);
        }
        WidgetNode::List { items, .. } => {
            for item in items {
                out.push(&item.text);
                if let Some(detail) = &item.detail {
                    out.push(detail);
                }
            }
        }
        WidgetNode::Divider => {}
        WidgetNode::Text { text, .. } => out.push(text),
        WidgetNode::Badge { label, .. } => out.push(label),
        WidgetNode::KeyValueGrid { pairs, .. } => {
            for pair in pairs {
                out.push(&pair.key);
                out.push(&pair.value);
            }
        }
        WidgetNode::Table { columns, rows } => {
            for column in columns {
                out.push(&column.header);
            }
            for row in rows {
                out.extend(row.cells.iter().map(String::as_str));
            }
        }
        WidgetNode::BarChart { title, data, .. } => {
            out.push(title);
            for datum in data {
                out.push(&datum.label);
            }
        }
        WidgetNode::Progress { label, detail, .. } => {
            out.push(label);
            if let Some(detail) = detail {
                out.push(detail);
            }
        }
        WidgetNode::CodeDiff { path, lines, .. } => {
            if let Some(path) = path {
                out.push(path);
            }
            for line in lines {
                out.push(&line.text);
            }
        }
        WidgetNode::StatusTimeline { steps } => {
            for step in steps {
                out.push(&step.label);
                if let Some(detail) = &step.detail {
                    out.push(detail);
                }
            }
        }
        WidgetNode::Button { buttons } => {
            for button in buttons {
                out.push(&button.label);
            }
        }
        WidgetNode::ChoicePicker { question, choices } => {
            out.push(question);
            for choice in choices {
                out.push(&choice.label);
                if let Some(description) = &choice.description {
                    out.push(description);
                }
            }
        }
        WidgetNode::TextField {
            label,
            value,
            placeholder,
            ..
        } => {
            out.push(label);
            out.push(value);
            if let Some(placeholder) = placeholder {
                out.push(placeholder);
            }
        }
        WidgetNode::Confirm { question, .. } => out.push(question),
        WidgetNode::Modal { title, child } => {
            out.push(title);
            collect_string_leaves(child, out);
        }
    }
}

fn collect_string_leaves_mut<'a>(node: &'a mut WidgetNode, out: &mut Vec<&'a mut String>) {
    match node {
        WidgetNode::Row { children } | WidgetNode::Column { children } => {
            for child in children {
                collect_string_leaves_mut(child, out);
            }
        }
        WidgetNode::Card { title, child, .. } => {
            out.push(title);
            collect_string_leaves_mut(child, out);
        }
        WidgetNode::Tabs { tabs, child, .. } => {
            out.extend(tabs.iter_mut());
            collect_string_leaves_mut(child, out);
        }
        WidgetNode::List { items, .. } => {
            for item in items {
                out.push(&mut item.text);
                if let Some(detail) = &mut item.detail {
                    out.push(detail);
                }
            }
        }
        WidgetNode::Divider => {}
        WidgetNode::Text { text, .. } => out.push(text),
        WidgetNode::Badge { label, .. } => out.push(label),
        WidgetNode::KeyValueGrid { pairs, .. } => {
            for pair in pairs {
                out.push(&mut pair.key);
                out.push(&mut pair.value);
            }
        }
        WidgetNode::Table { columns, rows } => {
            for column in columns {
                out.push(&mut column.header);
            }
            for row in rows {
                out.extend(row.cells.iter_mut());
            }
        }
        WidgetNode::BarChart { title, data, .. } => {
            out.push(title);
            for datum in data {
                out.push(&mut datum.label);
            }
        }
        WidgetNode::Progress { label, detail, .. } => {
            out.push(label);
            if let Some(detail) = detail {
                out.push(detail);
            }
        }
        WidgetNode::CodeDiff { path, lines, .. } => {
            if let Some(path) = path {
                out.push(path);
            }
            for line in lines {
                out.push(&mut line.text);
            }
        }
        WidgetNode::StatusTimeline { steps } => {
            for step in steps {
                out.push(&mut step.label);
                if let Some(detail) = &mut step.detail {
                    out.push(detail);
                }
            }
        }
        WidgetNode::Button { buttons } => {
            for button in buttons {
                out.push(&mut button.label);
            }
        }
        WidgetNode::ChoicePicker { question, choices } => {
            out.push(question);
            for choice in choices {
                out.push(&mut choice.label);
                if let Some(description) = &mut choice.description {
                    out.push(description);
                }
            }
        }
        WidgetNode::TextField {
            label,
            value,
            placeholder,
            ..
        } => {
            out.push(label);
            out.push(value);
            if let Some(placeholder) = placeholder {
                out.push(placeholder);
            }
        }
        WidgetNode::Confirm { question, .. } => out.push(question),
        WidgetNode::Modal { title, child } => {
            out.push(title);
            collect_string_leaves_mut(child, out);
        }
    }
}

/// Depth and node-count check shared by the deserializer and the gate.
///
/// One implementation, two call sites: the deserializer refuses early (so a
/// 200-deep tree never reaches the gate) and the gate refuses again (because
/// a caller can construct a `SurfaceOp` in code without going through JSON).
pub fn check_depth_and_count(op: &SurfaceOp) -> SurfaceVerdict<()> {
    let (depth, nodes) = op.tree_size();
    if depth > MAX_SURFACE_DEPTH {
        return Err(UiRefusal::OutOfBounds {
            detail: format!(
                "surface nests {depth} levels; the limit is {MAX_SURFACE_DEPTH} \
                 (lower the tree, or split it into several surfaces)"
            ),
        });
    }
    if nodes > MAX_SURFACE_NODES {
        return Err(UiRefusal::OutOfBounds {
            detail: format!(
                "surface has {nodes} widgets; the limit is {MAX_SURFACE_NODES} \
                 (split it into several surfaces)"
            ),
        });
    }
    Ok(())
}

/// Whether an op fits a transport's capabilities.
///
/// Read-only nodes are fine everywhere (they degrade to the text fallback).
/// An interactive node needs `inline_widgets` (TUI) or `native_buttons`
/// (Telegram); `Modal` additionally needs `modals`.
pub fn check_capabilities(op: &SurfaceOp, caps: &SurfaceCapabilities) -> SurfaceVerdict<()> {
    for widget in op.widgets() {
        if !widget.is_interactive() {
            continue;
        }
        if caps.inline_widgets {
            continue;
        }
        if !caps.native_buttons {
            return Err(UiRefusal::UnsupportedNode {
                node: widget.kind().to_string(),
                capabilities: caps.describe(),
            });
        }
        if matches!(widget, WidgetNode::Modal { .. }) && !caps.modals {
            return Err(UiRefusal::UnsupportedNode {
                node: widget.kind().to_string(),
                capabilities: caps.describe(),
            });
        }
    }
    Ok(())
}

impl WidgetNode {
    /// Whether this widget is a [`WidgetNode::TextField`].
    pub fn is_text_field(&self) -> bool {
        matches!(self, Self::TextField { .. })
    }
}

impl SurfaceOp {
    /// The op's root widget, for a renderer that draws one tree.
    ///
    /// An op with no widgets (a `SealSurface`) renders as a [`WidgetNode::Divider`]:
    /// a visible rule rather than an empty string, so a host that draws "the
    /// surface" still draws SOMETHING and the model's fallback is never blank.
    /// One implementation because two hosts (the TUI's event arm and the
    /// gateway's text fallback) both need the same answer, and a second copy is
    /// how they would come to disagree.
    pub fn root_widget(&self) -> WidgetNode {
        self.widgets()
            .into_iter()
            .next()
            .cloned()
            .unwrap_or(WidgetNode::Divider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_op(body: WidgetNode) -> SurfaceOp {
        SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "test".into(),
            provenance: Provenance::AgentSynthesis,
            body,
        }
    }

    #[test]
    fn tone_is_the_only_colour_surface() {
        // Compile-time-ish: SemanticTone has no `Color`/`hex` field and no
        // variant carries one. This test pins the ENUM SET so a future
        // `Custom { rgb: (u8,u8,u8) }` variant fails the build here.
        let tones = [
            SemanticTone::Default,
            SemanticTone::Muted,
            SemanticTone::Accent,
            SemanticTone::Success,
            SemanticTone::Warning,
            SemanticTone::Danger,
        ];
        assert_eq!(tones.len(), 6, "a new tone needs a renderer in BOTH sinks");
    }

    #[test]
    fn agent_synthesis_always_badges() {
        let op = flat_op(WidgetNode::Divider);
        assert!(
            op.is_synthesized(),
            "agent-synthesized surfaces must be marked unverified"
        );
        let verified = SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "test".into(),
            provenance: Provenance::ToolVerified {
                tool_name: "web_search".into(),
                source_uri: Some("https://example.com".into()),
            },
            body: WidgetNode::Divider,
        };
        assert!(!verified.is_synthesized());
    }

    #[test]
    fn depth_bound_refuses_a_deep_tree() {
        // 9 nested cards = depth 9 > MAX_SURFACE_DEPTH (8).
        let mut node = WidgetNode::Divider;
        for _ in 0..8 {
            node = WidgetNode::Card {
                title: "t".into(),
                child: Box::new(node),
                tone: SemanticTone::Default,
            };
        }
        let op = flat_op(node);
        assert_eq!(op.tree_size().0, 9);
        let err = check_depth_and_count(&op).unwrap_err();
        assert!(matches!(err, UiRefusal::OutOfBounds { .. }));
    }

    #[test]
    fn node_count_bound_refuses_a_wide_tree() {
        let children: Vec<WidgetNode> = (0..(MAX_SURFACE_NODES + 1))
            .map(|i| WidgetNode::Text {
                text: format!("row {i}"),
                tone: SemanticTone::Default,
                mono: false,
            })
            .collect();
        let op = flat_op(WidgetNode::Column { children });
        let err = check_depth_and_count(&op).unwrap_err();
        assert!(matches!(err, UiRefusal::OutOfBounds { .. }));
    }

    #[test]
    fn capabilities_refuse_an_interactive_node_on_a_text_only_transport() {
        let op = flat_op(WidgetNode::Button {
            buttons: vec![ButtonSpec {
                id: "ok".into(),
                label: "OK".into(),
                tone: SemanticTone::Accent,
            }],
        });
        assert!(check_capabilities(&op, &SurfaceCapabilities::text_only()).is_err());
        assert!(check_capabilities(&op, &SurfaceCapabilities::telegram_dm()).is_ok());
        assert!(check_capabilities(&op, &SurfaceCapabilities::tui()).is_ok());
    }

    #[test]
    fn read_only_nodes_are_fine_everywhere() {
        let op = flat_op(WidgetNode::Table {
            columns: vec![TableColumn {
                header: "k".into(),
                align: ColumnAlign::Left,
            }],
            rows: vec![TableRow {
                cells: vec!["v".into()],
            }],
        });
        assert!(check_capabilities(&op, &SurfaceCapabilities::text_only()).is_ok());
    }

    #[test]
    fn string_leaves_reach_every_variant() {
        let node = WidgetNode::Card {
            title: "card-title".into(),
            tone: SemanticTone::Default,
            child: Box::new(WidgetNode::Tabs {
                tabs: vec!["one".into(), "two".into()],
                active: 0,
                child: Box::new(WidgetNode::KeyValueGrid {
                    pairs: vec![KeyValue {
                        key: "answer".into(),
                        value: "42".into(),
                        tone: SemanticTone::Default,
                    }],
                    columns: 2,
                }),
            }),
        };
        let op = flat_op(node);
        let leaves = op.string_leaves();
        assert!(leaves.contains(&"card-title"));
        assert!(leaves.contains(&"one"));
        assert!(leaves.contains(&"answer"));
        assert!(leaves.contains(&"42"));
    }

    #[test]
    fn seal_surface_has_no_widgets() {
        let op = SurfaceOp::SealSurface {
            surface_id: "s1".into(),
        };
        assert!(op.widgets().is_empty());
        assert_eq!(op.surface_id(), "s1");
    }
}
