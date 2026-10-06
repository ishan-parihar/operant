// Vendored from jcode (crates/jcode-tui/src/tui/theme_detect.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported @ 0a9dc7805 (batch-4 sweep
// tail). Re-roots: jcode_tui_style -> crate::tui::vendor::style,
// crate::config::config() -> crate::tui::jcode_app::config_shim::config(),
// crate::logging -> crate::tui::jcode_app::logging.
//
// [port-decision] terminal_colorsaurus (the OSC 11 background query) is a
// pending dependency decision, like ratatui_image and unicode_properties.
// Degraded arm: detect_terminal_theme performs the same is-terminal and
// OSC-support checks, then returns None (falling back to Dark) instead of
// querying; the silent-terminal cache/identity helpers are gated out with
// the same marker. They return verbatim at the user-call YES. The env
// override (JCODE_THEME), config (display.theme), palette install and the
// resume handoff path are all live.

use crate::tui::jcode_app::config_shim::config;
use crate::tui::jcode_app::logging;
use crate::tui::vendor::style::{Palette, ThemeMode};
use std::sync::{Mutex, OnceLock};

static DETECTED: OnceLock<ThemeMode> = OnceLock::new();

/// In-flight prewarm of the (blocking) terminal background query. See
/// [`prewarm_theme_mode`].
static PREWARM: Mutex<Option<std::thread::JoinHandle<ThemeMode>>> = Mutex::new(None);

/// Start resolving the theme mode on a background thread so the OSC 11 round
/// trip overlaps other startup work (notably spawning/awaiting the server)
/// instead of adding its full latency to the critical path.
///
/// Only safe before the terminal enters raw mode and while nothing else reads
/// stdin, because the query writes an escape sequence and consumes the reply.
/// Idempotent, and a no-op once the mode is already resolved.
pub fn prewarm_theme_mode() {
    if DETECTED.get().is_some() {
        return;
    }
    let Ok(mut slot) = PREWARM.lock() else {
        return;
    };
    if slot.is_some() {
        return;
    }
    if let Ok(handle) = std::thread::Builder::new()
        .name("jcode-theme-detect".to_string())
        .spawn(resolve_theme_mode)
    {
        *slot = Some(handle);
    }
}

/// Join a prewarm started by [`prewarm_theme_mode`], if any.
fn take_prewarmed_theme_mode() -> Option<ThemeMode> {
    let handle = PREWARM.lock().ok()?.take()?;
    handle.join().ok()
}

/// Resolve and install the global theme mode. Idempotent; the first call does
/// the (potentially blocking, sub-second) terminal query and later calls are
/// free. Must be called before entering raw mode / the alternate screen.
pub fn init_theme_mode() -> ThemeMode {
    let mode = match take_prewarmed_theme_mode() {
        Some(prewarmed) => *DETECTED.get_or_init(|| prewarmed),
        None => *DETECTED.get_or_init(resolve_theme_mode),
    };
    crate::tui::vendor::style::set_theme_mode(mode);
    init_palette();
    mode
}

/// Resolve the theme while resuming an already-active TUI after an `exec` handoff.
///
/// The inherited terminal is already in raw mode and may already have a crossterm
/// event reader attached. Sending a fresh OSC 11 query in that state can leave the
/// terminal's color response in stdin, where it is decoded as ordinary composer
/// input. Prefer the theme captured by the previous process and otherwise resolve
/// configuration without querying the terminal.
pub fn init_theme_mode_for_resume(inherited_theme: Option<&str>) -> ThemeMode {
    let inherited_theme = inherited_theme.and_then(|value| match value {
        "dark" => Some(ThemeMode::Dark),
        "light" => Some(ThemeMode::Light),
        _ => None,
    });
    // A prewarm may already have queried the terminal; prefer its answer over a
    // second query, but never start one on an inherited raw-mode terminal.
    let prewarmed = take_prewarmed_theme_mode();
    let mode = *DETECTED.get_or_init(|| {
        inherited_theme
            .or(prewarmed)
            .unwrap_or_else(resolve_theme_mode_without_terminal_query)
    });
    crate::tui::vendor::style::set_theme_mode(mode);
    init_palette();
    mode
}

/// Install the user's configured color palette from `[display.colors]`.
///
/// Invalid entries are logged and skipped rather than failing the palette, so
/// one typo can never leave the TUI unstyled. Safe to call repeatedly; the TUI
/// calls it again after `/colors` edits so changes apply without a restart.
pub fn init_palette() {
    let configured = &config().display.colors;
    let (palette, errors) = Palette::from_pairs(
        configured
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    for error in errors {
        logging::warn(&format!("display.colors: {error}"));
    }
    crate::tui::vendor::style::set_palette(palette);
}

pub fn current_theme_label() -> &'static str {
    match crate::tui::vendor::style::theme_mode() {
        ThemeMode::Dark => "dark",
        ThemeMode::Light => "light",
    }
}

fn resolve_theme_mode() -> ThemeMode {
    resolve_configured_theme(true)
}

fn resolve_theme_mode_without_terminal_query() -> ThemeMode {
    resolve_configured_theme(false)
}

fn resolve_configured_theme(query_terminal: bool) -> ThemeMode {
    let configured = std::env::var("JCODE_THEME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| config().display.theme.clone());

    match configured.trim().to_ascii_lowercase().as_str() {
        "dark" => return ThemeMode::Dark,
        "light" => return ThemeMode::Light,
        "" | "auto" => {}
        other => {
            logging::info(&format!(
                "Unknown theme '{other}' (expected auto/dark/light); using auto detection"
            ));
        }
    }

    if query_terminal {
        detect_terminal_theme().unwrap_or(ThemeMode::Dark)
    } else {
        logging::info(
            "Skipping terminal background query during reload handoff; preserving a safe theme",
        );
        ThemeMode::Dark
    }
}

/// Query the terminal background color and classify it as dark or light.
/// Returns None when the terminal does not support querying or the query
/// fails, in which case the caller falls back to dark.
fn detect_terminal_theme() -> Option<ThemeMode> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    if !terminal_background_query_supported(
        std::env::var("TERM").ok().as_deref(),
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var("LC_TERMINAL").ok().as_deref(),
    ) {
        logging::info(
            "Skipping terminal background query for a terminal without OSC query support",
        );
        return None;
    }
    // [port-decision] terminal_colorsaurus (OSC 11 query): pending dependency
    // decision (with ratatui_image / unicode_properties). Degraded arm: the
    // query never runs, so every OSC-capable terminal resolves to Dark unless
    // JCODE_THEME/display.theme says otherwise. The silent-terminal cache and
    // identity helpers below are gated out with this same marker and return
    // verbatim at the YES decision.
    logging::info(
        "Terminal background query unavailable (terminal-colorsaurus pending); defaulting to dark theme",
    );
    None
}

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
        super::storage::jcode_dir()
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
