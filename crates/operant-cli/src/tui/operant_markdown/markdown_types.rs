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

/// Which kind of region a copy target is. This is the only spelling:
/// RT-168's second one (`tui/copy_targets.rs`, the pre-W2 interim) was deleted
/// in iter-1184 after its behaviour contract was ported onto the live detector
/// in `markdown_tests/cases/copy_target_contract.rs`.
///
/// `Math` carries the display/inline distinction. Nothing constructs THIS
/// variant yet — a `math` frame renders with a `math` label and the detector
/// reports it as `CodeBlock { language: Some("math") }` (measured in
/// `markdown_tests/cases/copy_target_contract.rs::test_a_math_block_is_a_copy_target_and_its_label_is_preserved`).
/// RT-168's duplication is collapsed: the interim spelling is gone and this is
/// the only enum, so promoting math to this variant is now a one-site change.
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "RT-168 collapsed: no live construction of this spelling; a `math` frame reports as CodeBlock today"
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
    /// The region's badge label. No PRODUCTION caller reads it yet — the badge
    /// renders a key, not a label. RT-168's duplication is collapsed, so this is
    /// the only spelling and the copy path reads it directly rather than through
    /// a per-frame cached copy. The explicit match stays so the first live
    /// caller is a visible decision point.
    #[cfg_attr(
        test,
        allow(
            dead_code,
            reason = "RT-168 collapsed: the badge renders a key today, not this label"
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
