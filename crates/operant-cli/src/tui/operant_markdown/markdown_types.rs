// Vendored from jcode (crates/operant-tui-markdown/src/markdown_types.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805, no
// changes. See operant_markdown/mod.rs for scope.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub enum DiagramDisplayMode {
    #[default]
    None,
    Margin,
    Pinned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub enum MarkdownSpacingMode {
    #[default]
    Compact,
    Document,
}

/// How inline/display LaTeX is rendered. `None` and `Unicode` are the two
/// non-raster modes the config vocabulary names; only `Image` is produced
/// today, so the other two are dead in this build.
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "jcode-parity port: config-named render modes the current build never constructs"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LatexRenderingMode {
    None,
    Unicode,
    #[default]
    Image,
}

/// Which kind of region a copy target is. RT-168 records that a second,
/// near-identical `CopyTargetKind` lives in `copy_targets.rs` (pre-W2 interim
/// spelling, referenced only by its own tests); this is the one the production
/// render path imports.
///
/// `Math` carries the display/inline distinction the copy_targets.rs spelling
/// lacks, which is why it survives there; nothing constructs THIS variant yet
/// (RT-168 records the split).
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "RT-168: the display/inline math distinction survives in copy_targets.rs's tests, not in a live construction of this spelling"
    )
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyTargetKind {
    CodeBlock { language: Option<String> },
    Blockquote,
    Math { display: bool },
    Error,
    ToolOutput,
}

impl CopyTargetKind {
    /// The region's badge label. No PRODUCTION caller reads it — RT-168's
    /// duplication means the surviving callers of this behaviour are the
    /// `copy_targets.rs` tests. The method stays with its explicit match so the
    /// first live construction is a visible decision point.
    #[cfg_attr(
        test,
        allow(
            dead_code,
            reason = "RT-168: the surviving callers of this behaviour are copy_targets.rs's tests, until the duplication is collapsed"
        )
    )]
    pub fn label(&self) -> String {
        match self {
            Self::CodeBlock { language } => language
                .as_deref()
                .filter(|lang| !lang.is_empty())
                .unwrap_or("code")
                .to_string(),
            Self::Blockquote => "quote".to_string(),
            Self::Math { .. } => "math".to_string(),
            Self::Error => "error".to_string(),
            Self::ToolOutput => "output".to_string(),
        }
    }

    /// The notice shown after a copy. Same status as [`Self::label`] — no
    /// production caller until RT-168 is collapsed.
    #[cfg_attr(
        test,
        allow(
            dead_code,
            reason = "RT-168: paired with label(); the surviving callers are copy_targets.rs's tests"
        )
    )]
    pub fn copied_notice(&self) -> String {
        match self {
            Self::CodeBlock { language } => {
                let label = language
                    .as_deref()
                    .filter(|lang| !lang.is_empty())
                    .unwrap_or("code block");
                format!("Copied {}", label)
            }
            Self::Blockquote => "Copied quote".to_string(),
            Self::Math { .. } => "Copied math".to_string(),
            Self::Error => "Copied error".to_string(),
            Self::ToolOutput => "Copied output".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RawCopyTarget {
    pub kind: CopyTargetKind,
    pub content: String,
    pub start_raw_line: usize,
    pub end_raw_line: usize,
    pub badge_raw_line: usize,
}
