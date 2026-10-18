// Vendored from jcode (crates/operant-tui/src/tui/theme_detect.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported @ 0a9dc7805 (batch-4 sweep
// tail). Re-roots: operant_tui_style -> crate::tui::vendor::style,
// crate::config::config() -> crate::tui::operant_app::config_shim::config(),
// crate::logging -> crate::tui::operant_app::logging.
//
// [port-decision] terminal_colorsaurus (the OSC 11 background query) is a
// pending dependency decision, like ratatui_image and unicode_properties.
// Degraded arm: detect_terminal_theme performs the same is-terminal and
// OSC-support checks, then returns None (falling back to Dark) instead of
// querying; the silent-terminal cache/identity helpers are gated out with
// the same marker. They return verbatim at the user-call YES. The env
// override (OPERANT_THEME), config (display.theme), palette install and the
// resume handoff path are all live.

/// Identity of the terminal we are talking to, for caching purposes. Keep it
/// coarse: the emulator identity, not the individual window or session.
// [port-decision] colorsaurus companion, gated with the query arm (see above).
#[cfg(any())]
fn terminal_identity() -> String {
    let value = |name: &str| std::env::var(name).unwrap_or_default();
    format!(
        "{}|{}|{}",
        value("TERM"),
        value("TERM_PROGRAM"),
        value("LC_TERMINAL")
    )
}

// [port-decision] colorsaurus companions, gated with the query arm (see above).
#[cfg(any())]
fn silent_terminal_cache_path() -> Option<std::path::PathBuf> {
    Some(
        super::storage::operant_dir()
            .ok()?
            .join("cache")
            .join("osc11-silent-terminals"),
    )
}

/// Longest cache we keep, so an upgraded or reconfigured terminal that gains
/// OSC support is re-probed instead of being written off forever.
#[cfg(any())]
const SILENT_TERMINAL_CACHE_TTL: std::time::Duration =
    std::time::Duration::from_secs(60 * 60 * 24 * 7);

/// Cap on remembered terminal identities. This is a cache, not a record.
#[cfg(any())]
const SILENT_TERMINAL_CACHE_MAX: usize = 32;

#[cfg(any())]
fn silent_terminal_is_cached_at(path: Option<&std::path::Path>, identity: &str) -> bool {
    let Some(path) = path else {
        return false;
    };
    let Ok(modified) = std::fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    if modified
        .elapsed()
        .is_ok_and(|age| age > SILENT_TERMINAL_CACHE_TTL)
    {
        return false;
    }
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    contents.lines().any(|line| line == identity)
}

#[cfg(any())]
fn cache_silent_terminal_at(path: Option<&std::path::Path>, identity: &str) {
    let Some(path) = path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if existing.lines().any(|line| line == identity) {
        return;
    }
    let mut kept: Vec<&str> = existing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(SILENT_TERMINAL_CACHE_MAX - 1)
        .collect();
    kept.reverse();
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(identity);
    out.push('\n');
    let _ = std::fs::write(path, out);
}

/// Reject terminal classes that cannot answer OSC 11 before entering the
/// colorsaurus timeout path. A concrete terminal-program hint wins because
/// launchers and multiplexers occasionally leave a conservative `TERM` value
/// in place even though the outer emulator supports OSC queries.
fn terminal_background_query_supported(
    term: Option<&str>,
    term_program: Option<&str>,
    lc_terminal: Option<&str>,
) -> bool {
    if term_program.is_some_and(|value| !value.trim().is_empty())
        || lc_terminal.is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }

    let term = term.unwrap_or("").trim().to_ascii_lowercase();
    !matches!(term.as_str(), "" | "dumb" | "linux" | "cons25" | "emacs")
}

#[cfg(test)]
mod tests {
    use super::terminal_background_query_supported;

    #[test]
    fn skips_terminals_without_osc_query_support() {
        for term in [None, Some(""), Some("dumb"), Some("linux"), Some("cons25")] {
            assert!(!terminal_background_query_supported(term, None, None));
        }
    }

    #[test]
    fn queries_terminal_emulators_and_honors_program_hints() {
        assert!(terminal_background_query_supported(
            Some("xterm-256color"),
            None,
            None
        ));
        assert!(terminal_background_query_supported(
            Some("linux"),
            Some("kitty"),
            None
        ));
        assert!(terminal_background_query_supported(
            Some("linux"),
            None,
            Some("iTerm2")
        ));
    }

    // [port-decision] the silent-terminal cache tests are gated with the
    // query arm (colorsaurus pending); they return verbatim at the YES.
}
