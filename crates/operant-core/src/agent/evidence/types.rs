//! The evidence layer's type vocabulary.
//!
//! A tool result is described by two facts: **what produced it** (`Subject`)
//! and **what shape it is** (`ContentFormat`). The pair selects a chain of
//! `Method`s that know how to shrink that shape without destroying the part
//! the model needs.
//!
//! Every method declares a contract in its `MethodInfo`, so the layer can
//! answer three questions about any transformation before it runs: did it
//! lose information, can the lost information be recovered, and is it safe
//! to run twice.

use serde::{Deserialize, Serialize};

/// What kind of tool produced a result.
///
/// The subject is a *policy* input: a shell result and a web result may be
/// the same shape, but they are trusted differently and recovered differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subject {
    /// A file read.
    Read,
    /// A file write or patch result.
    Write,
    /// An edit result.
    Edit,
    /// A shell command's output.
    Shell,
    /// A search or match listing.
    Search,
    /// A fetched web page or API response.
    Web,
    /// A delegated sub-agent's output.
    Subagent,
    /// An MCP server's result.
    Mcp,
    /// Any other tool result.
    ToolResult,
    /// A result whose producer is not known.
    Any,
}

impl Subject {
    /// The stable name used in configuration and in provenance markers.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Subject::Read => "read",
            Subject::Write => "write",
            Subject::Edit => "edit",
            Subject::Shell => "shell",
            Subject::Search => "search",
            Subject::Web => "web",
            Subject::Subagent => "subagent",
            Subject::Mcp => "mcp",
            Subject::ToolResult => "tool_result",
            Subject::Any => "any",
        }
    }
}

/// The shape of a payload, which selects the chain.
///
/// `Code` and `Plain` are operant extensions over the usual five: a source
/// file and a paragraph of prose need different treatment from a JSON API
/// body, and a code read is the single largest uncompressed surface in the
/// stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentFormat {
    /// A JSON document.
    Json,
    /// An HTML page or a markdown document.
    Web,
    /// A list of search matches.
    Matches,
    /// A dependency lockfile.
    Lockfile,
    /// Terminal output.
    Term,
    /// Source code.
    Code,
    /// Unstructured prose.
    Plain,
}

impl ContentFormat {
    /// The stable name used in configuration and in provenance markers.
    pub const fn as_str(&self) -> &'static str {
        match self {
            ContentFormat::Json => "json",
            ContentFormat::Web => "web",
            ContentFormat::Matches => "matches",
            ContentFormat::Lockfile => "lockfile",
            ContentFormat::Term => "term",
            ContentFormat::Code => "code",
            ContentFormat::Plain => "plain",
        }
    }
}

/// Whether a method transforms content or injects a directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodKind {
    /// Rewrites the payload.
    Transform,
    /// Adds guidance without changing the payload.
    Directive,
}

/// How the information a method dropped can be recovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Nothing was lost.
    None,
    /// The full text is retrievable from the recall store.
    Recall,
    /// A tool call retrieves the full text.
    RetrieveTool,
    /// A pointer to the full text is printed with the compressed output.
    Pointer,
}

/// How expensive a method is, ordered cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    /// Pure string work, no parsing.
    Instant,
    /// Parses the payload.
    Fast,
    /// Parses and may spawn work.
    Slow,
}

/// What a method emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Produces {
    /// The same shape as the input.
    Same,
    /// A different shape.
    Format(ContentFormat),
}

/// The contract a method declares.
///
/// A method that loses information must say so (`lossy`) and must say how the
/// information comes back (`recovery`). A method that declares neither is a
/// bug, not a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodInfo {
    /// Transform or directive.
    pub kind: MethodKind,
    /// Whether the method can lose information.
    pub lossy: bool,
    /// How lost information is recovered.
    pub recovery: Recovery,
    /// Whether running the method twice equals running it once.
    pub idempotent: bool,
    /// Whether the output is stable across identical inputs (prompt-cache safe).
    pub cache_stable: bool,
    /// Whether the output depends only on the input.
    pub deterministic: bool,
    /// The method's cost.
    pub cost: CostClass,
    /// The formats the method accepts.
    pub consumes: &'static [ContentFormat],
    /// What the method emits.
    pub produces: Produces,
    /// Whether the method ends the chain.
    pub terminal: bool,
    /// The method's cost in tokens, used when ranking chain candidates.
    pub cost_tokens: usize,
}

impl MethodInfo {
    /// The contract for a transformation that changes nothing meaningful:
    /// lossless, idempotent, cache-stable and deterministic.
    pub const LOSSLESS_TRANSFORM: MethodInfo = MethodInfo {
        kind: MethodKind::Transform,
        lossy: false,
        recovery: Recovery::None,
        idempotent: true,
        cache_stable: true,
        deterministic: true,
        cost: CostClass::Instant,
        consumes: &[],
        produces: Produces::Same,
        terminal: false,
        cost_tokens: 0,
    };

    /// The contract for a transformation that loses information but records
    /// where the full text went.
    pub const LOSSY_RECALLABLE: MethodInfo = MethodInfo {
        kind: MethodKind::Transform,
        lossy: true,
        recovery: Recovery::Recall,
        idempotent: true,
        cache_stable: true,
        deterministic: true,
        cost: CostClass::Fast,
        consumes: &[],
        produces: Produces::Same,
        terminal: false,
        cost_tokens: 0,
    };
}

/// A payload, plus what is known about it.
#[derive(Debug, Clone, Copy)]
pub struct Block<'a> {
    /// The text to transform.
    pub text: &'a str,
    /// What produced it.
    pub subject: Subject,
    /// What shape it is.
    pub format: ContentFormat,
    /// Where it came from.
    pub origin: &'a Origin,
}

impl<'a> Block<'a> {
    /// Build a block over `text`.
    pub fn new(text: &'a str, subject: Subject, format: ContentFormat, origin: &'a Origin) -> Self {
        Block {
            text,
            subject,
            format,
            origin,
        }
    }
}

/// Where a payload came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Origin {
    /// The tool that produced it.
    pub tool: Option<String>,
    /// The file it read or wrote.
    pub path: Option<String>,
    /// The command that produced it.
    pub command: Option<String>,
}

/// What a method did with a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Continue the chain with this text.
    Continue(String),
    /// Stop the chain; this is the final text.
    ShortCircuit(String),
    /// Leave the text unchanged and continue.
    Skip,
}

/// The context a method may read while transforming.
#[derive(Debug, Clone, Copy)]
pub struct MethodCtx<'a> {
    /// A human-readable description of the source, used by methods that need
    /// to know what they are looking at (a URL, a file name).
    pub source: &'a str,
}

impl<'a> MethodCtx<'a> {
    /// Build a context over `source`.
    pub fn new(source: &'a str) -> Self {
        MethodCtx { source }
    }
}

/// A single transformation.
pub trait Method: Sync {
    /// The method's stable name, used in configuration and provenance.
    fn name(&self) -> &'static str;
    /// The method's contract.
    fn info(&self) -> MethodInfo;
    /// Transform the block.
    fn apply(&self, block: &Block, ctx: &MethodCtx) -> Outcome;
}
