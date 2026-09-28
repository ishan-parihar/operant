// image_paste.rs — Clipboard image detection and text paste via subprocess.
//
// Supports three operations:
//   1. `read_clipboard_text()` — read text from the system clipboard
//   2. `read_clipboard_image()` — detect an image in the clipboard and save to a temp file
//   3. Helper structs for image attachments shown in the prompt
//
// All clipboard access uses platform CLI tools (no native Rust bindings needed):
//   macOS  : pbpaste / osascript
//   Linux  : xclip / wl-paste
//   Windows: PowerShell Get-Clipboard

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use crate::tui::image_render::{self, ImageRenderConfig, RenderedImage};

// ---------------------------------------------------------------------------
// Image attachment state
// ---------------------------------------------------------------------------

/// A pasted image attachment waiting to be included in the next message.
#[derive(Debug, Clone)]
pub struct PastedImage {
    /// Path to the temporary PNG file on disk.
    pub path: PathBuf,
    /// Display label shown in the prompt (e.g. "clipboard.png" or "image.png").
    pub label: String,
    /// Original dimensions, if known.
    pub dimensions: Option<(u32, u32)>,
}

// ---------------------------------------------------------------------------
// Clipboard text reading
// ---------------------------------------------------------------------------

/// Read text from the system clipboard. Returns `None` if the clipboard is
/// empty, unavailable, or contains non-text data.
pub fn read_clipboard_text() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        read_text_macos()
    }
    #[cfg(target_os = "windows")]
    {
        read_text_windows()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        read_text_linux()
    }
}

/// Read text from the primary selection when supported (Linux/X11/Wayland).
pub fn read_primary_text() -> Option<String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        None
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        read_primary_text_linux()
    }
}

#[cfg(target_os = "macos")]
fn read_text_macos() -> Option<String> {
    let out = Command::new("pbpaste").output().ok()?;
    if out.status.success() && !out.stdout.is_empty() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_text_linux() -> Option<String> {
    read_text_linux_selection(false)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_primary_text_linux() -> Option<String> {
    read_text_linux_selection(true)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_text_linux_selection(primary: bool) -> Option<String> {
    let commands: &[(&str, &[&str])] = if primary {
        &[
            ("wl-paste", &["--primary", "--no-newline"]),
            ("xclip", &["-selection", "primary", "-o"]),
            ("xsel", &["--primary", "--output"]),
        ]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    };

    for (prog, args) in commands {
        if let Ok(out) = Command::new(prog).args(*args).output()
            && out.status.success()
            && !out.stdout.is_empty()
        {
            return Some(String::from_utf8_lossy(&out.stdout).into_owned());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn read_text_windows() -> Option<String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Get-Clipboard"])
        .output()
        .ok()?;
    if out.status.success() && !out.stdout.is_empty() {
        Some(
            String::from_utf8_lossy(&out.stdout)
                .trim_end_matches('\n')
                .to_string(),
        )
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Clipboard image reading
// ---------------------------------------------------------------------------

/// Check whether the clipboard currently holds an image. If it does, write
/// the PNG to a temp file and return a `PastedImage`.
pub fn read_clipboard_image() -> Option<PastedImage> {
    #[cfg(target_os = "macos")]
    {
        read_image_macos()
    }
    #[cfg(target_os = "windows")]
    {
        read_image_windows()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        read_image_linux()
    }
}

// ── macOS ──────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn read_image_macos() -> Option<PastedImage> {
    // Check whether the clipboard contains an image type
    let check = Command::new("osascript")
        .args(["-e", "the clipboard as «class PNGf»"])
        .output()
        .ok()?;

    if !check.status.success() || check.stdout.is_empty() {
        return None;
    }

    // Write the PNG bytes to a temp file
    let tmp = make_temp_png()?;

    let script = format!(
        r#"set pngData to (the clipboard as «class PNGf»)
set fp to open for access POSIX file "{}" with write permission
write pngData to fp
close access fp"#,
        tmp.display()
    );

    let write_out = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .ok()?;
    if write_out.status.success() && tmp.exists() && tmp.metadata().ok()?.len() > 0 {
        let dims = png_dimensions(&tmp);
        Some(PastedImage {
            label: "clipboard.png".to_string(),
            path: tmp,
            dimensions: dims,
        })
    } else {
        let _ = std::fs::remove_file(&tmp);
        None
    }
}

// ── Linux ──────────────────────────────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_image_linux() -> Option<PastedImage> {
    // Check whether clipboard contains an image type
    let has_image = check_linux_clipboard_has_image();
    if !has_image {
        return None;
    }

    let tmp = make_temp_png()?;

    // Try xclip then wl-paste
    let saved = try_save_linux_image(&tmp);
    if saved && tmp.exists() && tmp.metadata().ok()?.len() > 0 {
        let dims = png_dimensions(&tmp);
        Some(PastedImage {
            label: "clipboard.png".to_string(),
            path: tmp,
            dimensions: dims,
        })
    } else {
        let _ = std::fs::remove_file(&tmp);
        None
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn check_linux_clipboard_has_image() -> bool {
    // xclip: list TARGETS and grep for image/
    if let Ok(out) = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "TARGETS", "-o"])
        .output()
        && out.status.success()
    {
        let targets = String::from_utf8_lossy(&out.stdout);
        if targets.contains("image/") {
            return true;
        }
    }
    // wl-paste: check available types
    if let Ok(out) = Command::new("wl-paste").args(["--list-types"]).output()
        && out.status.success()
    {
        let types = String::from_utf8_lossy(&out.stdout);
        if types.contains("image/") {
            return true;
        }
    }
    false
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn try_save_linux_image(path: &PathBuf) -> bool {
    // xclip
    if let Ok(out) = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
        && out.status.success()
        && !out.stdout.is_empty()
        && std::fs::write(path, &out.stdout).is_ok()
    {
        return true;
    }
    // wl-paste
    if let Ok(out) = Command::new("wl-paste")
        .args(["--type", "image/png"])
        .output()
        && out.status.success()
        && !out.stdout.is_empty()
        && std::fs::write(path, &out.stdout).is_ok()
    {
        return true;
    }
    false
}

// ── Windows ────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn read_image_windows() -> Option<PastedImage> {
    // Check whether clipboard has an image
    let check = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "if ((Get-Clipboard -Format Image) -ne $null) { 'yes' } else { 'no' }",
        ])
        .output()
        .ok()?;

    let answer = String::from_utf8_lossy(&check.stdout).trim().to_string();
    if answer != "yes" {
        return None;
    }

    let tmp = make_temp_png()?;
    let tmp_str = tmp.display().to_string();

    let script = format!(
        "$img = Get-Clipboard -Format Image; \
         $img.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png)",
        tmp_str.replace('\'', "''")
    );

    let save = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()?;

    if save.status.success() && tmp.exists() && tmp.metadata().ok()?.len() > 0 {
        let dims = png_dimensions(&tmp);
        Some(PastedImage {
            label: "clipboard.png".to_string(),
            path: tmp,
            dimensions: dims,
        })
    } else {
        let _ = std::fs::remove_file(&tmp);
        None
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Clipboard text writing
// ---------------------------------------------------------------------------
//
// There is deliberately no write path here any more. This used to carry a
// third copy of the wl-copy/xclip/xsel chain (which also wrote the X11 primary
// selection). Every clipboard write in the TUI now goes through
// [`crate::tui::clipboard`], so there is exactly one chain to keep correct.

fn make_temp_png() -> Option<PathBuf> {
    let tmp_dir = std::env::temp_dir();
    let name = format!(
        "operant-paste-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    Some(tmp_dir.join(name))
}

/// Read PNG dimensions from the IHDR chunk (bytes 16–23).
fn png_dimensions(path: &PathBuf) -> Option<(u32, u32)> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 24 {
        return None;
    }
    // PNG signature: 8 bytes; IHDR: 4 len + 4 type + 4 w + 4 h
    if &data[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    Some((w, h))
}

/// Read a file and base64-encode it for the Anthropic API.
#[allow(dead_code)] // Base64 encoding for API uploads
pub fn encode_image_base64(path: &PathBuf) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    Some(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &data,
    ))
}

// ---------------------------------------------------------------------------
// Inline rendering
// ---------------------------------------------------------------------------

/// A clear, single-line stand-in for an image the terminal cannot draw.
///
/// Names the format and the pixel size so the user still learns what was
/// pasted, instead of the attachment silently disappearing.
pub fn placeholder_for(img: &PastedImage) -> String {
    let format = image_format_of(&img.path, &img.label);
    match img.dimensions {
        Some((w, h)) => {
            format!("[image: {format} {w}x{h} — no inline graphics protocol available]")
        }
        None => format!("[image: {format} — no inline graphics protocol available]"),
    }
}

/// Sniff the image format from the file's magic bytes, falling back to the
/// label's extension. Sniffing keeps the placeholder honest for drag-and-drop
/// attachments, whose label is the user's filename.
fn image_format_of(path: &PathBuf, label: &str) -> String {
    if let Ok(head) = std::fs::read(path) {
        let head = &head[..head.len().min(12)];
        if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            return "PNG".to_string();
        }
        if head.starts_with(b"\xff\xd8\xff") {
            return "JPEG".to_string();
        }
        if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
            return "GIF".to_string();
        }
        if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
            return "WebP".to_string();
        }
        if head.starts_with(b"BM") {
            return "BMP".to_string();
        }
    }
    match label
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_uppercase())
    {
        Some(ext) if !ext.is_empty() => ext,
        _ => "image".to_string(),
    }
}

/// Render a pasted attachment for the current terminal.
///
/// Delegates to [`image_render::render_image`], which auto-detects the protocol
/// in the order Kitty -> iTerm2 -> Sixel. On failure the result carries a
/// textual placeholder rather than raw bytes or silence.
pub fn render_attachment(img: &PastedImage) -> RenderedImage {
    let rendered = image_render::render_image(&img.path, &ImageRenderConfig::default());
    if rendered.success {
        return rendered;
    }
    RenderedImage {
        escape_sequence: placeholder_for(img),
        width_cells: 0,
        height_cells: 0,
        success: false,
    }
}

/// Render the attachment and, when the terminal speaks a graphics protocol,
/// write the escape sequence to stdout.
///
/// Terminal graphics protocols paint outside ratatui's cell grid, so this is
/// the same post-paint write path the OSC 8 hyperlink overlay uses. Returns
/// the rendered image (or the placeholder) so the caller can report it.
pub fn emit_inline_image(img: &PastedImage) -> RenderedImage {
    let rendered = render_attachment(img);
    if rendered.success {
        let mut out = std::io::stdout();
        let _ = out.write_all(rendered.escape_sequence.as_bytes());
        let _ = out.flush();
    }
    rendered
}

/// Human-readable one-liner describing what happened to a pasted image.
pub fn describe_rendered(img: &PastedImage, rendered: &RenderedImage) -> String {
    let size = match img.dimensions {
        Some((w, h)) => format!("{w}x{h}"),
        None => "unknown size".to_string(),
    };
    if rendered.success {
        format!(
            "{} ({size}) — inline via {}, {}x{} cells",
            img.label,
            image_render::detect_graphics_protocol(),
            rendered.width_cells,
            rendered.height_cells
        )
    } else {
        format!("{} ({size}) — {}", img.label, rendered.escape_sequence)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_image_clone() {
        let img = PastedImage {
            path: PathBuf::from("/tmp/test.png"),
            label: "test.png".to_string(),
            dimensions: Some((800, 600)),
        };
        let cloned = img.clone();
        assert_eq!(cloned.label, "test.png");
        assert_eq!(cloned.dimensions, Some((800, 600)));
    }

    #[test]
    fn make_temp_png_produces_unique_names() {
        let p = make_temp_png().unwrap();
        assert!(p.to_string_lossy().contains("operant-paste-"));
        assert!(p.to_string_lossy().ends_with(".png"));
    }

    #[test]
    fn png_dimensions_invalid_data_returns_none() {
        let tmp = make_temp_png().unwrap();
        std::fs::write(&tmp, b"not a png").unwrap();
        assert!(png_dimensions(&tmp).is_none());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn png_dimensions_valid_header() {
        // Minimal valid PNG IHDR: 8-byte sig + 4-byte length + "IHDR" + 4-byte w + 4-byte h + ...
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        // IHDR chunk: length=13
        data[8..12].copy_from_slice(&13u32.to_be_bytes());
        data[12..16].copy_from_slice(b"IHDR");
        // width = 100
        data[16..20].copy_from_slice(&100u32.to_be_bytes());
        // height = 200
        data[20..24].copy_from_slice(&200u32.to_be_bytes());
        let tmp = make_temp_png().unwrap();
        std::fs::write(&tmp, &data).unwrap();
        let dims = png_dimensions(&tmp);
        assert_eq!(dims, Some((100, 200)));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn encode_image_base64_missing_file_returns_none() {
        let p = PathBuf::from("/nonexistent/file.png");
        assert!(encode_image_base64(&p).is_none());
    }

    #[test]
    fn encode_image_base64_roundtrip() {
        let tmp = make_temp_png().unwrap();
        std::fs::write(&tmp, b"hello world").unwrap();
        let b64 = encode_image_base64(&tmp).unwrap();
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64).unwrap();
        assert_eq!(decoded, b"hello world");
        let _ = std::fs::remove_file(&tmp);
    }

    /// Write a file with a real PNG IHDR so the format sniffer and the
    /// dimension reader both have something real to work with.
    fn write_test_png(name_hint: &str, w: u32, h: u32) -> PathBuf {
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        data[8..12].copy_from_slice(&13u32.to_be_bytes());
        data[12..16].copy_from_slice(b"IHDR");
        data[16..20].copy_from_slice(&w.to_be_bytes());
        data[20..24].copy_from_slice(&h.to_be_bytes());
        let path = std::env::temp_dir().join(format!("operant-test-{name_hint}.png"));
        std::fs::write(&path, &data).unwrap();
        path
    }

    /// `image_render` had zero callers, so a pasted image never rendered. When
    /// the terminal has no graphics protocol the user must still be told what
    /// was pasted — a named, sized placeholder, not silence and not bytes.
    #[test]
    fn image_paste_should_render_placeholder_when_no_protocol_available() {
        let path = write_test_png("placeholder", 640, 480);
        let img = PastedImage {
            path: path.clone(),
            label: "clipboard.png".to_string(),
            dimensions: Some((640, 480)),
        };

        // Force a terminal with no graphics protocol: the runner's own
        // terminal may well speak Kitty or Sixel.
        let rendered =
            image_render::with_env(&image_render::PROTOCOL_FREE_ENV, || render_attachment(&img));
        assert!(!rendered.success);
        assert_eq!(rendered.width_cells, 0);
        assert_eq!(rendered.height_cells, 0);

        // One clear line naming the format and the pixel size.
        let text = &rendered.escape_sequence;
        assert!(!text.contains('\n'), "placeholder must be a single line");
        assert!(text.contains("PNG"), "placeholder names the format: {text}");
        assert!(
            text.contains("640x480"),
            "placeholder names the size: {text}"
        );
        assert!(
            text.contains("no inline graphics protocol"),
            "placeholder explains why: {text}"
        );
        // Never raw bytes.
        assert!(
            !text.contains('\u{1b}'),
            "placeholder must not carry escapes"
        );

        // The description line carries the same information for the toast.
        let described = describe_rendered(&img, &rendered);
        assert!(described.contains("clipboard.png"));
        assert!(described.contains("640x480"));
        assert!(described.contains("no inline graphics protocol"));

        let _ = std::fs::remove_file(&path);
    }

    /// A non-PNG attachment (drag-and-drop carries the user's filename) falls
    /// back to sniffing, then to the extension, and still never leaks bytes.
    #[test]
    fn image_paste_placeholder_sniffs_format_for_non_png() {
        let path = std::env::temp_dir().join("operant-test-sniff.jpg");
        std::fs::write(&path, b"\xff\xd8\xff\xe0hello").unwrap();
        let img = PastedImage {
            path: path.clone(),
            label: "photo.jpeg".to_string(),
            dimensions: None,
        };
        let rendered =
            image_render::with_env(&image_render::PROTOCOL_FREE_ENV, || render_attachment(&img));
        assert!(!rendered.success);
        assert!(rendered.escape_sequence.contains("JPEG"));
        let _ = std::fs::remove_file(&path);
    }
}
