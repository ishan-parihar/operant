// Vendored from jcode (crates/operant-side-panel-types/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805 — the whole
// module (102 lines, types only): the session-scoped side-panel snapshot
// contract the `side_panel` tool writes and the TUI side pane renders.
// [port-decision] added at batch-3: TuiState trait dependency
// (tui_state.rs `side_panel` returns &SidePanelSnapshot). operant_app/mod.rs
// must declare `pub mod side_panel;` — integrator step; this batch may not
// touch mod.rs.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidePanelPageFormat {
    #[default]
    Markdown,
    Pdf,
}

impl SidePanelPageFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidePanelPageSource {
    #[default]
    Managed,
    LinkedFile,
    Ephemeral,
}

impl SidePanelPageSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::LinkedFile => "linked_file",
            Self::Ephemeral => "ephemeral",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedSidePanelState {
    #[serde(default)]
    pub focus_revision: u64,
    #[serde(default)]
    pub focused_page_id: Option<String>,
    #[serde(default)]
    pub pages: Vec<PersistedSidePanelPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSidePanelPage {
    pub id: String,
    pub title: String,
    pub file_path: String,
    #[serde(default)]
    pub format: SidePanelPageFormat,
    #[serde(default)]
    pub source: SidePanelPageSource,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SidePanelPage {
    pub id: String,
    pub title: String,
    pub file_path: String,
    #[serde(default)]
    pub format: SidePanelPageFormat,
    #[serde(default)]
    pub source: SidePanelPageSource,
    #[serde(default)]
    pub content: String,
    /// Base64 PDF bytes. Content remains a human-readable Markdown fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_data: Option<String>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SidePanelSnapshot {
    #[serde(default)]
    pub focus_revision: u64,
    #[serde(default)]
    pub focused_page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<SidePanelPage>,
}

impl SidePanelSnapshot {
    pub fn has_pages(&self) -> bool {
        !self.pages.is_empty()
    }

    pub fn focused_page(&self) -> Option<&SidePanelPage> {
        let focused_id = self.focused_page_id.as_deref()?;
        self.pages.iter().find(|page| page.id == focused_id)
    }
}

pub fn snapshot_is_empty(snapshot: &SidePanelSnapshot) -> bool {
    !snapshot.has_pages()
}

// --- refresh machinery (operant-base/src/side_panel.rs:8,93-130,443-485,514-575),
// --- ported at batch-4 sweep tail so `ui_pinned_tests::render_side_panel_markdown_live_syncs_file_content`
// --- has its live-sync backend. The upstream session-persist/append layer above
// --- these fns stays unported (cutover layer); this leaf is the file I/O core.

use anyhow::{Context as _, Result};
use base64::Engine as _;
use std::hash::{Hash as _, Hasher as _};
use std::io::Read as _;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Largest PDF we will load into a side panel (operant-base/src/side_panel.rs:6).
pub const MAX_PDF_BYTES: u64 = 20 * 1024 * 1024;

/// Aggregate budget shared by every PDF page in one session (:8).
pub const MAX_SESSION_PDF_BYTES: u64 = 32 * 1024 * 1024;

pub fn refresh_linked_page_content(
    snapshot: &mut SidePanelSnapshot,
    page_id: Option<&str>,
) -> bool {
    let target_page_id = page_id.or(snapshot.focused_page_id.as_deref());
    let mut changed = false;

    for page in &mut snapshot.pages {
        if page.source != SidePanelPageSource::LinkedFile {
            continue;
        }
        if let Some(target_page_id) = target_page_id
            && page.id != target_page_id
            // PDF pages share a session-wide budget. A nonfocused PDF growing
            // or shrinking can change whether the focused page can be loaded.
            && page.format != SidePanelPageFormat::Pdf
        {
            continue;
        }

        let next_revision = linked_file_revision(Path::new(&page.file_path));
        if next_revision == page.updated_at_ms {
            continue;
        }

        let (content, pdf_data) = hydrated_content(Path::new(&page.file_path), page.format);
        page.content = content;
        page.pdf_data = pdf_data;
        page.updated_at_ms = next_revision;
        changed = true;
    }

    if changed {
        let mut budget = MAX_SESSION_PDF_BYTES;
        for page in &mut snapshot.pages {
            // Reconsider all PDFs when budget changes, including a previously
            // downgraded page whose own file revision has not changed.
            if page.source == SidePanelPageSource::LinkedFile
                && page.format == SidePanelPageFormat::Pdf
            {
                (page.content, page.pdf_data) =
                    hydrated_content(Path::new(&page.file_path), page.format);
                page.updated_at_ms = linked_file_revision(Path::new(&page.file_path));
            }
            enforce_pdf_budget(&mut page.content, &mut page.pdf_data, &mut budget);
        }
    }
    changed
}

fn enforce_pdf_budget(content: &mut String, data: &mut Option<String>, budget: &mut u64) {
    if let Some(encoded) = data {
        let size = pdf_byte_len(encoded);
        if size > *budget {
            *data = None;
            *content =
                "Unable to load PDF: session PDF payload exceeds aggregate 32 MiB limit.".into();
        } else {
            *budget -= size;
        }
    }
}

fn pdf_byte_len(encoded: &str) -> u64 {
    ((encoded.len() / 4) * 3 - encoded.bytes().rev().take_while(|b| *b == b'=').count()) as u64
}

fn read_page_content(path: &Path, format: SidePanelPageFormat) -> Result<(String, Option<String>)> {
    match format {
        SidePanelPageFormat::Markdown => Ok((
            std::fs::read_to_string(path)
                .with_context(|| format!("failed to read {}", path.display()))?,
            None,
        )),
        SidePanelPageFormat::Pdf => {
            let file = std::fs::File::open(path)
                .with_context(|| format!("failed to read PDF {}", path.display()))?;
            anyhow::ensure!(
                file.metadata()?.len() <= MAX_PDF_BYTES,
                "PDF exceeds 20 MiB limit: {}",
                path.display()
            );
            // Bound the actual read too, in case the source grows after metadata.
            let mut bytes = Vec::new();
            file.take(MAX_PDF_BYTES + 1)
                .read_to_end(&mut bytes)
                .with_context(|| format!("failed to read PDF {}", path.display()))?;
            anyhow::ensure!(
                bytes.len() as u64 <= MAX_PDF_BYTES,
                "PDF exceeds 20 MiB limit: {}",
                path.display()
            );
            anyhow::ensure!(
                bytes.starts_with(b"%PDF-"),
                "invalid PDF signature: {}",
                path.display()
            );
            Ok((
                format!(
                    "PDF document: `{}`\n\nOpen this panel in the desktop app to view the PDF.",
                    path.display()
                ),
                Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            ))
        }
    }
}

fn hydrated_content(path: &Path, format: SidePanelPageFormat) -> (String, Option<String>) {
    read_page_content(path, format).unwrap_or_else(|err| {
        (
            format!("Unable to load {} document: {err:#}", format.as_str()),
            None,
        )
    })
}

fn linked_file_revision(path: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);

    match std::fs::metadata(path) {
        Ok(metadata) => {
            metadata.len().hash(&mut hasher);
            metadata.permissions().readonly().hash(&mut hasher);
            metadata
                .modified()
                .ok()
                .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
                .map(|dur| (dur.as_secs(), dur.subsec_nanos()))
                .hash(&mut hasher);
            "present".hash(&mut hasher);
        }
        Err(_) => {
            "missing".hash(&mut hasher);
        }
    }

    hasher.finish()
}
