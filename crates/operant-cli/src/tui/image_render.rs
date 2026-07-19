// image_render.rs — Kitty/Sixel/iTerm2 inline image rendering for TUI.
//
// Provides terminal capability detection and image rendering via:
//   - Kitty Graphics Protocol (most capable, supports RGB, animation)
//   - Sixel (DEC VT340 legacy, wide support)
//   - iTerm2 proprietary protocol (macOS only)

use std::path::PathBuf;
use std::process::Command;

/// Supported terminal graphics protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsProtocol {
    /// Kitty Graphics Protocol — RGB, animation, transparency, Unicode placeholders.
    Kitty,
    /// Sixel (DEC VT340) — 256 colors, broad terminal support.
    Sixel,
    /// iTerm2 inline images — macOS only, proprietary but well-supported.
    ITerm2,
    /// No graphics protocol detected; fall back to text description.
    None,
}

/// Detect the best available graphics protocol for the current terminal.
/// Checks environment variables and terminal capabilities in order of preference.
pub fn detect_graphics_protocol() -> GraphicsProtocol {
    // Kitty: $TERM contains "kitty" or $KITTY_WINDOW_ID is set
    if std::env::var("TERM").unwrap_or_default().contains("kitty")
        || std::env::var("KITTY_WINDOW_ID").is_ok()
    {
        return GraphicsProtocol::Kitty;
    }

    // iTerm2: $TERM_PROGRAM == "iTerm.app" or $ITERM_SESSION_ID set
    if std::env::var("TERM_PROGRAM").as_deref() == Ok("iTerm.app")
        || std::env::var("ITERM_SESSION_ID").is_ok()
    {
        return GraphicsProtocol::ITerm2;
    }

    // Sixel: check for DEC private mode support via $TERM
    // Many modern terminals (mintty, contour, foot, wezterm, etc.) support Sixel
    let term = std::env::var("TERM").unwrap_or_default();
    let sixel_terms = [
        "foot",
        "wezterm",
        "contour",
        "mintty",
        "xterm",
        "vte",
        "alacritty",
        "konsole",
        "gnome",
        "xfce",
        "mate",
        "rxvt",
        "mlterm",
        "st",
        "tmux",
    ];
    if sixel_terms.iter().any(|t| term.contains(t)) {
        // Heuristic: assume Sixel support for known terminals
        // Could be refined with actual capability query (DECRQSS) but that's async
        return GraphicsProtocol::Sixel;
    }

    GraphicsProtocol::None
}

/// Image rendering configuration.
#[derive(Debug, Clone)]
pub struct ImageRenderConfig {
    /// Maximum display width in terminal cells.
    pub max_width_cells: u16,
    /// Maximum display height in terminal cells.
    pub max_height_cells: u16,
    /// Preserve aspect ratio when scaling.
    pub preserve_aspect: bool,
    /// Use Unicode half-block characters for higher density (Kitty only).
    pub use_half_blocks: bool,
    /// Placeholder text when rendering is not supported.
    pub placeholder: String,
}

impl Default for ImageRenderConfig {
    fn default() -> Self {
        Self {
            max_width_cells: 80,
            max_height_cells: 40,
            preserve_aspect: true,
            use_half_blocks: true,
            placeholder: "[image]".to_string(),
        }
    }
}

/// Rendered image result.
#[derive(Debug, Clone)]
pub struct RenderedImage {
    /// The escape sequence(s) to emit for inline display.
    pub escape_sequence: String,
    /// Width in terminal cells.
    pub width_cells: u16,
    /// Height in terminal cells.
    pub height_cells: u16,
    /// Whether rendering succeeded (false = fallback to placeholder).
    pub success: bool,
}

/// Render an image file to terminal escape sequences using the detected protocol.
pub fn render_image(path: &PathBuf, config: &ImageRenderConfig) -> RenderedImage {
    let protocol = detect_graphics_protocol();

    match protocol {
        GraphicsProtocol::Kitty => render_kitty(path, config),
        GraphicsProtocol::Sixel => render_sixel(path, config),
        GraphicsProtocol::ITerm2 => render_iterm2(path, config),
        GraphicsProtocol::None => RenderedImage {
            escape_sequence: config.placeholder.clone(),
            width_cells: 0,
            height_cells: 0,
            success: false,
        },
    }
}

/// Kitty Graphics Protocol rendering.
/// See: https://sw.kovidgoyal.net/kitty/graphics-protocol/
fn render_kitty(path: &PathBuf, config: &ImageRenderConfig) -> RenderedImage {
    // Read image data
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => {
            return RenderedImage {
                escape_sequence: config.placeholder.clone(),
                width_cells: 0,
                height_cells: 0,
                success: false,
            };
        }
    };

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data);

    // Get image dimensions (simplified - assumes PNG for now)
    let (img_w, img_h) = get_image_dimensions(&data).unwrap_or((800, 600));

    // Calculate cell dimensions
    // Kitty uses character cells; we need to estimate based on font size
    // Typical terminal: ~8x16 pixels per cell
    let cell_w = 8.0;
    let cell_h = 16.0;

    let max_w_px = config.max_width_cells as f32 * cell_w;
    let max_h_px = config.max_height_cells as f32 * cell_h;

    let scale = if config.preserve_aspect {
        (max_w_px / img_w as f32)
            .min(max_h_px / img_h as f32)
            .min(1.0)
    } else {
        1.0
    };

    let disp_w = (img_w as f32 * scale / cell_w).round() as u16;
    let disp_h = (img_h as f32 * scale / cell_h).round() as u16;

    let disp_w = disp_w.min(config.max_width_cells).max(1);
    let disp_h = disp_h.min(config.max_height_cells).max(1);

    // Kitty protocol: ESC_G key=value,...;payload ESC \
    // a=T (transmit), f=100 (PNG), q=1 (compress), c=cols, r=rows
    let cmd = format!(
        "\x1b_Ga=T,f=100,q=1,c={},r={},m=1;{}\x1b\\",
        disp_w, disp_h, b64
    );

    RenderedImage {
        escape_sequence: cmd,
        width_cells: disp_w,
        height_cells: disp_h,
        success: true,
    }
}

/// Sixel rendering via `img2sixel` or built-in encoder.
/// For simplicity, we shell out to `img2sixel` if available.
fn render_sixel(path: &PathBuf, config: &ImageRenderConfig) -> RenderedImage {
    // Try img2sixel first (most reliable)
    if let Ok(output) = Command::new("img2sixel")
        .args(["-w", &config.max_width_cells.to_string()])
        .arg(path)
        .output()
    {
        if output.status.success() {
            let sixel_data = String::from_utf8_lossy(&output.stdout);
            return RenderedImage {
                escape_sequence: sixel_data.to_string(),
                width_cells: config.max_width_cells,
                height_cells: config.max_height_cells,
                success: true,
            };
        }
    }

    // Fallback: try built-in sixel encoder for PNG
    if let Some(sixel) = encode_sixel_png(path, config) {
        return RenderedImage {
            escape_sequence: sixel,
            width_cells: config.max_width_cells,
            height_cells: config.max_height_cells,
            success: true,
        };
    }

    RenderedImage {
        escape_sequence: config.placeholder.clone(),
        width_cells: 0,
        height_cells: 0,
        success: false,
    }
}

/// iTerm2 proprietary inline image protocol.
/// See: https://iterm2.com/documentation-images.html
fn render_iterm2(path: &PathBuf, config: &ImageRenderConfig) -> RenderedImage {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => {
            return RenderedImage {
                escape_sequence: config.placeholder.clone(),
                width_cells: 0,
                height_cells: 0,
                success: false,
            };
        }
    };

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data);
    let (img_w, img_h) = get_image_dimensions(&data).unwrap_or((800, 600));

    let cell_w = 8.0;
    let cell_h = 16.0;
    let max_w_px = config.max_width_cells as f32 * cell_w;
    let max_h_px = config.max_height_cells as f32 * cell_h;

    let scale = if config.preserve_aspect {
        (max_w_px / img_w as f32)
            .min(max_h_px / img_h as f32)
            .min(1.0)
    } else {
        1.0
    };

    let disp_w = (img_w as f32 * scale / cell_w).round() as u16;
    let disp_h = (img_h as f32 * scale / cell_h).round() as u16;
    let disp_w = disp_w.min(config.max_width_cells).max(1);
    let disp_h = disp_h.min(config.max_height_cells).max(1);

    // iTerm2: ESC ] 1337 ; File = [optional params] : base64 ESC \
    // Params: width=auto,height=auto,preserveAspectRatio=1,inline=1
    let cmd = format!(
        "\x1b]1337;File=width={}px;height={}px;preserveAspectRatio=1;inline=1:{}\x1b\\",
        disp_w as u32 * 8,
        disp_h as u32 * 16,
        b64
    );

    RenderedImage {
        escape_sequence: cmd,
        width_cells: disp_w,
        height_cells: disp_h,
        success: true,
    }
}

/// Extract image dimensions from raw bytes (PNG/JPEG/GIF/WebP).
fn get_image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    // PNG: IHDR chunk at offset 16-23
    if data.len() >= 24 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return Some((w, h));
    }
    // JPEG: scan for SOF marker
    if data.len() >= 4 && &data[0..2] == b"\xff\xd8" {
        let mut i = 2;
        while i + 8 < data.len() {
            if data[i] == 0xff
                && data[i + 1] >= 0xc0
                && data[i + 1] <= 0xcf
                && data[i + 1] != 0xc4
                && data[i + 1] != 0xc8
                && data[i + 1] != 0xcc
            {
                let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        }
    }
    // GIF: 6-byte header + 7-byte logical screen descriptor
    if data.len() >= 13 && (&data[0..6] == b"GIF87a" || &data[0..6] == b"GIF89a") {
        let w = u16::from_le_bytes([data[6], data[7]]) as u32;
        let h = u16::from_le_bytes([data[8], data[9]]) as u32;
        return Some((w, h));
    }
    // WebP: RIFF header + VP8/VP8L
    if data.len() >= 30 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        if &data[12..16] == b"VP8 " {
            // VP8: width/height at offset 26
            if data.len() >= 30 {
                let w = u16::from_le_bytes([data[26], data[27]]) as u32 & 0x3FFF;
                let h = u16::from_le_bytes([data[28], data[29]]) as u32 & 0x3FFF;
                return Some((w, h));
            }
        } else if &data[12..16] == b"VP8L" {
            // VP8L: size at offset 21
            if data.len() >= 25 {
                let bits = u32::from_le_bytes([data[21], data[22], data[23], data[24]]);
                let w = (bits & 0x3FFF) + 1;
                let h = ((bits >> 14) & 0x3FFF) + 1;
                return Some((w, h));
            }
        }
    }
    None
}

/// Minimal Sixel encoder for PNG (fallback when img2sixel not available).
/// This is a simplified implementation — production use should prefer img2sixel.
fn encode_sixel_png(_path: &PathBuf, _config: &ImageRenderConfig) -> Option<String> {
    // Full Sixel encoding is complex. For now, return None to trigger placeholder.
    // A complete implementation would:
    // 1. Decode PNG to raw RGB
    // 2. Quantize to 256 colors
    // 3. Encode as Sixel (run-length encoding of color registers)
    // This is ~500 lines of code; recommend using img2sixel binary instead.
    None
}

/// Clear the current Kitty graphics cursor position (move past rendered image).
pub fn kitty_clear_image() -> String {
    "\x1b_Ga=d,d=1\x1b\\".to_string()
}

/// Check if the terminal supports any graphics protocol.
pub fn has_graphics_support() -> bool {
    detect_graphics_protocol() != GraphicsProtocol::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_protocol_kitty_env() {
        unsafe {
            std::env::set_var("TERM", "xterm-kitty");
        }
        assert_eq!(detect_graphics_protocol(), GraphicsProtocol::Kitty);
        unsafe {
            std::env::remove_var("TERM");
        }
    }

    #[test]
    fn test_detect_protocol_iterm_env() {
        unsafe {
            std::env::set_var("TERM_PROGRAM", "iTerm.app");
        }
        assert_eq!(detect_graphics_protocol(), GraphicsProtocol::ITerm2);
        unsafe {
            std::env::remove_var("TERM_PROGRAM");
        }
    }

    #[test]
    fn test_get_png_dimensions() {
        // Minimal valid PNG IHDR: 8-byte sig + 4-byte length + "IHDR" + 4-byte w + 4-byte h
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        data[8..12].copy_from_slice(&13u32.to_be_bytes()); // IHDR length
        data[12..16].copy_from_slice(b"IHDR");
        data[16..20].copy_from_slice(&100u32.to_be_bytes()); // width
        data[20..24].copy_from_slice(&200u32.to_be_bytes()); // height
        assert_eq!(get_image_dimensions(&data), Some((100, 200)));
    }

    #[test]
    fn test_get_jpeg_dimensions() {
        // Minimal JPEG with SOF0 marker
        // JPEG segment length includes the 2 bytes of the length field itself
        let mut data = vec![0xff, 0xd8]; // SOI
        // APP0 marker: length = 16 (2 bytes length + 14 bytes payload = 16)
        data.extend_from_slice(&[0xff, 0xe0, 0x00, 0x10]); // APP0, length 16
        data.extend_from_slice(&[0; 14]); // 14 bytes payload (JFIF header)
        // SOF0 marker: length = 11 (2 bytes length + 1 byte precision + 2 height + 2 width + 1 components + 3*3 component spec = 11)
        data.extend_from_slice(&[0xff, 0xc0, 0x00, 0x0b]); // SOF0, length 11
        data.push(8); // precision
        data.extend_from_slice(&200u16.to_be_bytes()); // height
        data.extend_from_slice(&100u16.to_be_bytes()); // width
        data.push(3); // components
        // Component specs: 3 components * 3 bytes each = 9 bytes
        data.extend_from_slice(&[1, 0x21, 0x00, 2, 0x11, 0x01, 3, 0x11, 0x01]);
        assert_eq!(get_image_dimensions(&data), Some((100, 200)));
    }

    #[test]
    fn test_get_gif_dimensions() {
        let mut data = vec![0; 13];
        data[0..6].copy_from_slice(b"GIF89a");
        data[6..8].copy_from_slice(&100u16.to_le_bytes()); // width
        data[8..10].copy_from_slice(&200u16.to_le_bytes()); // height
        assert_eq!(get_image_dimensions(&data), Some((100, 200)));
    }
}
