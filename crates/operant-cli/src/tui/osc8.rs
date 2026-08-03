//! OSC 8 hyperlink overlay for the ratatui-based TUI.
//!
//! ratatui 0.29 has no native hyperlink primitive. We hook the main draw loop,
//! scan the painted buffer for URLs, and re-emit those cells wrapped in
//! `OSC 8 ; ; URL ESC \` ... `OSC 8 ;; ESC \` so terminals that implement
//! the protocol (Windows Terminal, iTerm2, WezTerm, Kitty, etc.) make them
//! Ctrl/Cmd-clickable. Terminals without OSC 8 support silently ignore it.
//!
//! Disable with `OPERANT_NO_HYPERLINKS=1`.

use std::io::{self, Write};

use crossterm::{
    QueueableCommand,
    cursor::{MoveTo, RestorePosition, SavePosition},
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
};
use ratatui::buffer::Buffer;
use regex::Regex;
use std::sync::LazyLock;

const OSC8_OPEN_PREFIX: &str = "\x1b]8;;";
const OSC8_ST: &str = "\x1b\\";
const OSC8_CLOSE: &str = "\x1b]8;;\x1b\\";

#[expect(
    clippy::expect_used,
    reason = "invariant guaranteed by surrounding validation"
)]
static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:https?|ftp)://[A-Za-z0-9\-._~:/?#\[\]@!$&'()*+,;=%]+|www\.[A-Za-z0-9\-]+\.[A-Za-z0-9\-._~:/?#\[\]@!$&'()*+,;=%]+"#,
    )
    .expect("OSC 8 URL regex")
});

#[derive(Debug, Clone)]
pub struct UrlHit {
    /// Visual column of the first cell, absolute (already includes area.x).
    pub col: u16,
    /// Visual row, absolute (already includes area.y).
    pub row: u16,
    /// URL passed to the terminal — normalized (e.g., `www.…` → `https://…`).
    pub url: String,
    /// Original on-screen text — re-printed verbatim so the row looks unchanged.
    pub display: String,
}

fn enabled() -> bool {
    match std::env::var("OPERANT_NO_HYPERLINKS").ok().as_deref() {
        Some(v) => !matches!(v.trim(), "1" | "true" | "yes" | "on"),
        None => true,
    }
}

/// Strip trailing punctuation that is almost certainly *not* part of the URL.
fn trim_url_punct(matched: &str) -> &str {
    let bytes = matched.as_bytes();
    let mut paren_balance: i32 = 0;
    for &b in bytes {
        if b == b'(' {
            paren_balance += 1;
        } else if b == b')' {
            paren_balance -= 1;
        }
    }
    let mut end = bytes.len();
    while end > 0 {
        let last = bytes[end - 1];
        let strip = match last {
            b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'"' | b'>' => true,
            b']' | b'}' => true,
            b')' => paren_balance < 0,
            _ => false,
        };
        if strip {
            if last == b')' {
                paren_balance += 1;
            }
            end -= 1;
        } else {
            break;
        }
    }
    &matched[..end]
}

/// Scan a rendered buffer for URL runs and return their visual positions + normalized targets.
pub fn scan_buffer_for_urls(buf: &Buffer) -> Vec<UrlHit> {
    if !enabled() {
        return Vec::new();
    }
    let area = buf.area();
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    let mut hits = Vec::new();
    let mut row_text = String::new();
    let mut col_of_byte: Vec<u16> = Vec::new();

    for row in 0..area.height {
        row_text.clear();
        col_of_byte.clear();
        for col in 0..area.width {
            let cell = &buf[(area.x + col, area.y + row)];
            let sym = cell.symbol();
            if sym.is_empty() {
                continue;
            }
            let before = row_text.len();
            row_text.push_str(sym);
            for _ in before..row_text.len() {
                col_of_byte.push(col);
            }
        }

        for m in URL_RE.find_iter(&row_text) {
            let matched = &row_text[m.start()..m.end()];
            let cleaned = trim_url_punct(matched);
            if cleaned.is_empty() {
                continue;
            }
            let start_byte = m.start();
            let Some(&start_col) = col_of_byte.get(start_byte) else {
                continue;
            };
            hits.push(UrlHit {
                col: area.x + start_col,
                row: area.y + row,
                url: normalize_url(cleaned),
                display: cleaned.to_string(),
            });
        }
    }
    hits
}

/// Write OSC 8 hyperlink wrappers for the given hits to stdout.
pub fn emit_hits(hits: &[UrlHit]) -> io::Result<()> {
    if !enabled() || hits.is_empty() {
        return Ok(());
    }
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    write_hits(&mut lock, hits)
}

fn normalize_url(s: &str) -> String {
    if s.starts_with("www.") {
        format!("https://{s}")
    } else {
        s.to_string()
    }
}

fn write_hits(writer: &mut impl Write, hits: &[UrlHit]) -> io::Result<()> {
    writer.queue(SavePosition)?;
    for h in hits {
        writer.queue(MoveTo(h.col, h.row))?;
        writer.queue(SetForegroundColor(Color::Cyan))?;
        writer.queue(SetAttribute(Attribute::Underlined))?;
        writer.queue(Print(format!("{OSC8_OPEN_PREFIX}{}{OSC8_ST}", h.url)))?;
        writer.queue(Print(&h.display))?;
        writer.queue(Print(OSC8_CLOSE))?;
        writer.queue(SetAttribute(Attribute::NoUnderline))?;
        writer.queue(ResetColor)?;
    }
    writer.queue(RestorePosition)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    fn buffer_with(lines: &[&str]) -> Buffer {
        let h = lines.len() as u16;
        let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        for (y, line) in lines.iter().enumerate() {
            buf.set_string(0, y as u16, *line, Style::default());
        }
        buf
    }

    #[test]
    fn detects_simple_http_url() {
        let buf = buffer_with(&["Visit https://example.com today"]);
        let hits = scan_buffer_for_urls(&buf);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].display, "https://example.com");
        assert_eq!(hits[0].url, "https://example.com");
    }

    #[test]
    fn detects_www_and_normalizes_to_https() {
        let buf = buffer_with(&["go to www.example.com now"]);
        let hits = scan_buffer_for_urls(&buf);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://www.example.com");
    }

    #[test]
    fn no_urls_no_hits() {
        let buf = buffer_with(&["just some text without urls"]);
        let hits = scan_buffer_for_urls(&buf);
        assert!(hits.is_empty());
    }

    #[test]
    fn two_urls_one_line() {
        let buf = buffer_with(&["a https://one.test and https://two.test x"]);
        let hits = scan_buffer_for_urls(&buf);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn strips_trailing_period() {
        let buf = buffer_with(&["See https://example.com."]);
        let hits = scan_buffer_for_urls(&buf);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].display, "https://example.com");
    }

    #[test]
    fn keeps_balanced_paren_inside_url() {
        let buf = buffer_with(&["see https://en.wikipedia.org/wiki/Foo_(bar) ok"]);
        let hits = scan_buffer_for_urls(&buf);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].display, "https://en.wikipedia.org/wiki/Foo_(bar)");
    }

    #[test]
    fn write_hits_emits_osc8_envelope() {
        let hits = vec![UrlHit {
            col: 6,
            row: 0,
            url: "https://example.com".to_string(),
            display: "https://example.com".to_string(),
        }];
        let mut out: Vec<u8> = Vec::new();
        write_hits(&mut out, &hits).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("\x1b]8;;https://example.com\x1b\\"),
            "missing OSC 8 open"
        );
        assert!(s.contains("\x1b]8;;\x1b\\"), "missing OSC 8 close");
    }
}
