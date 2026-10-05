// Vendored from jcode (crates/jcode-base/src/live_tests.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial:
// CoverageLineStyle (:2250) + is_stage_fraction (:2264) +
// classify_provider_test_coverage_line (:2281) — the surface
// ui_overlays.rs's model_status_line_style calls through
// `crate::live_tests::` (re-rooted here). [port-excision] the rest of
// live_tests.rs (the provider test harness) is not ported.
// See jcode_app/mod.rs for scope.

/// Semantic role of a single line in a provider-test-coverage report. Both the
/// CLI (ANSI) and the TUI overlay map this one classification onto their own
/// color palette, so the two surfaces stay in lock-step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageLineStyle {
    /// Section banner / headline / `Foo:` label.
    Title,
    /// Fully READY / passed.
    Pass,
    /// Hard failure (ran and failed/blocked).
    Fail,
    /// In progress / skipped / needs attention but not a hard failure.
    Warn,
    /// Subdued metadata, table chrome, untested rows.
    Dim,
    /// No special styling.
    Plain,
}

/// True when `token` looks like a pipeline stage count such as `6/11`.
fn is_stage_fraction(token: &str) -> bool {
    let mut parts = token.splitn(2, '/');
    match (parts.next(), parts.next()) {
        (Some(a), Some(b)) => {
            !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

/// Classify one report line for coloring. Rules are ordered most-specific first
/// so per-pair status tokens win over the prose that merely mentions "READY".
pub fn classify_provider_test_coverage_line(line: &str) -> CoverageLineStyle {
    use CoverageLineStyle::{Dim, Fail, Pass, Plain, Title, Warn};
    let t = line.trim_start();
    if t.is_empty() {
        return Plain;
    }

    // Detail-report checkpoint glyphs are unambiguous: color by the leading icon.
    match t.chars().next() {
        Some('✓') => return Pass,
        Some('✗') => return Fail,
        Some('!') => return Warn,
        Some('•') => return Dim,
        _ => {}
    }

    // Per-pair / detail rows that are fully READY lead with the READY token.
    if t == "READY" || t.starts_with("READY ") || t.starts_with("READY\t") {
        return Pass;
    }

    // Per-pair in-progress rows lead with an `N/M` stage count.
    if let Some(first) = t.split_whitespace().next()
        && is_stage_fraction(first)
    {
        return if t.contains("failed at") { Fail } else { Warn };
    }

    // Provider-monitor rows end with a `ready/seen` fraction; color by status
    // word. Checked before generic prose so a sentence that merely mentions
    // "READY" is not miscolored.
    if t.split_whitespace().last().is_some_and(is_stage_fraction) {
        if t.contains("needs native suite") || t.contains("untested") {
            return Dim;
        }
        if t.contains("no key") || t.contains("in progress") {
            return Warn;
        }
        if t.contains("READY") {
            return Pass;
        }
        return Plain;
    }

    // Section banners, the headline count, `#` headings, and `Foo:` labels.
    if t.starts_with("READY:")
        || t.starts_with('#')
        || t == "Live provider/model readiness"
        || (!t.is_empty() && t.chars().all(|c| c == '='))
        || (t.ends_with(':') && !t.starts_with('['))
    {
        return Title;
    }

    // Detail-report status verdicts.
    if t.contains("Fully tested") || t.contains("all seen pairs READY") {
        return Pass;
    }
    if t.contains("Partially tested") {
        return Warn;
    }

    // Per-provider rollup detail ("N not ready (...)").
    if t.contains("not ready") {
        return if t.contains("failed at") { Fail } else { Warn };
    }

    // Issue-tracked targets: "[...] provider / model: <verdict>".
    if t.starts_with('[') {
        if t.ends_with(": READY") {
            return Pass;
        }
        if t.contains("no evidence") {
            return Dim;
        }
        return Warn;
    }

    // Provider-monitor table chrome: header row, rule, legend.
    if t.starts_with("provider ") || t.starts_with("----") || t.starts_with("Legend:") {
        return Dim;
    }

    // Subdued metadata.
    if t.starts_with("last tested") || t.starts_with("Ledger:") || t.starts_with("Evidence source:")
    {
        return Dim;
    }

    Plain
}
