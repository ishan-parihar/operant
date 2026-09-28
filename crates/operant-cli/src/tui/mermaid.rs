// mermaid.rs — render a ```mermaid fenced block as a diagram, not as source text.
//
// A mermaid diagram is only drawable by mermaid's own toolchain (a Node CLI
// driving a headless Chrome). This crate does not embed that toolchain, so the
// feature is a *probe* plus a ladder, not a dependency:
//
//   rung 1  terminal graphics protocol AND an external renderer on PATH
//            -> rasterise, then draw inline via `tui::image_render`
//   rung 2  no graphics protocol
//            -> the diagram source in a bordered block, labelled with the reason
//   rung 3  no external renderer installed
//            -> the diagram source, plus the one command that fixes it
//   rung 4  the renderer ran and failed (malformed diagram, timeout, oversize)
//            -> the diagram source *and* the renderer's own error
//
// Rungs 2-4 all end in source text. A diagram that silently becomes a blank
// box is the worst outcome available, so every failure path keeps the source
// on screen.
//
// Two properties are correctness requirements, not optimisations:
//
//   * A subprocess must never block the frame loop. The raster runs on a
//     worker thread; the frame that discovers the diagram shows "rendering…"
//     and moves on.
//   * A source is resolved exactly once per process. The cache key is the
//     diagram source and nothing else — not a line number, not a timestamp, not
//     the terminal width — so an unchanged transcript can never re-spawn `npx`
//     on the next frame.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::tui::image_render::{self, GraphicsProtocol};
use crate::tui::messages::cache::hash_content;
use crate::tui::theme_colors;

/// Raster ceiling asked of the external renderer, in CSS pixels.
///
/// Fixed on purpose. Because the raster does not depend on the terminal width,
/// the cache key can be the diagram source alone and a resize never re-spawns
/// the renderer.
const RASTER_SIZE: &str = "1200";

/// Wall-clock ceiling on one render. A cold `npx` resolves a package (and
/// possibly a browser) on first use, so this is generous; the worker thread
/// means the frame loop waits for nothing either way.
const RENDER_TIMEOUT: Duration = Duration::from_secs(60);

/// Size ceiling on the PNG read back from the renderer. A diagram that
/// rasterises to a gigabyte is a bug or an attack, not a picture.
const MAX_PNG_BYTES: u64 = 8 * 1024 * 1024;

/// Size ceiling on the renderer's stderr, before it is shown to the user.
const MAX_STDERR_BYTES: usize = 16 * 1024;

/// Character ceiling on the error text kept for a failed render.
const MAX_ERROR_CHARS: usize = 400;

/// Ceiling on cached diagrams, so a very long transcript cannot grow the map
/// without bound.
const MAX_CACHED_DIAGRAMS: usize = 256;

/// Matches the code-fence caps in `tui::messages::markdown`, so a diagram
/// block and a code block are the same shape on screen.
const FENCE_WIDTH: usize = 22;

// ---------------------------------------------------------------------------
// External renderer
// ---------------------------------------------------------------------------

/// A mermaid rasteriser found on `PATH`.
///
/// Deliberately a probe and never a dependency: mermaid's toolchain is Node +
/// headless Chrome, which this crate must not pull in. When nothing is
/// installed the ladder degrades to the diagram source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalRenderer {
    /// Executable name, resolved through `PATH`.
    pub program: &'static str,
    /// Arguments that must precede the `-i`/`-o` pair.
    pub args: &'static [&'static str],
}

impl ExternalRenderer {
    /// A globally installed `mmdc` — no resolver, no download, no drift.
    pub const MMDC: Self = Self {
        program: "mmdc",
        args: &[],
    };

    /// `npx` fallback. The package version is pinned for the same supply-chain
    /// reason `@agentmemory` is pinned: `npx -y` would otherwise execute
    /// whatever the registry serves at the moment a transcript is opened.
    pub const NPX: Self = Self {
        program: "npx",
        args: &["-y", "@mermaid-js/mermaid-cli@12.0.0"],
    };
}

impl std::fmt::Display for ExternalRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.program)
    }
}

/// Everything the ladder needs, injected so the decision stays pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MermaidEnv {
    /// Terminal graphics protocol, from `tui::image_render`.
    pub protocol: GraphicsProtocol,
    /// External renderer found on `PATH`, if any.
    pub renderer: Option<ExternalRenderer>,
}

impl MermaidEnv {
    /// Probe the terminal and `PATH` once per process.
    ///
    /// A `PATH` scan per frame would be a per-frame cost for an answer that
    /// cannot change while the process runs.
    pub fn detect() -> &'static MermaidEnv {
        static ENV: LazyLock<MermaidEnv> = LazyLock::new(|| MermaidEnv {
            protocol: image_render::detect_graphics_protocol(),
            renderer: probe_renderer(),
        });
        &ENV
    }
}

fn probe_renderer() -> Option<ExternalRenderer> {
    // Standalone first: `mmdc` starts in milliseconds, `npx` does not.
    [ExternalRenderer::MMDC, ExternalRenderer::NPX]
        .into_iter()
        .find(|renderer| which::which(renderer.program).is_ok())
}

// ---------------------------------------------------------------------------
// The ladder
// ---------------------------------------------------------------------------

/// Why the ladder chose the source over a rendered diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Rung 2: the terminal cannot draw an inline image.
    NoGraphicsProtocol,
    /// Rung 3: no mermaid toolchain is installed.
    NoExternalRenderer,
}

impl Why {
    /// The line shown to the user in place of a diagram.
    pub fn message(self) -> &'static str {
        match self {
            Self::NoGraphicsProtocol => "diagram not drawn: no terminal graphics protocol",
            Self::NoExternalRenderer => {
                "diagram not drawn: no mermaid renderer on PATH (npm i -g @mermaid-js/mermaid-cli)"
            }
        }
    }
}

/// Which rung a diagram lands on. Pure: spawns nothing, reads no terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MermaidPlan {
    /// Rung 1: rasterise, then draw inline.
    Render,
    /// Rung 2 / rung 3: show the source.
    Source(Why),
}

/// Decide the rung. The order is the contract: a terminal that cannot draw an
/// inline image never gets a subprocess spawned on its behalf, and a terminal
/// that can never gets `npx` for a diagram it cannot display.
pub fn plan(env: &MermaidEnv) -> MermaidPlan {
    match (env.protocol, env.renderer) {
        (GraphicsProtocol::None, _) => MermaidPlan::Source(Why::NoGraphicsProtocol),
        (_, None) => MermaidPlan::Source(Why::NoExternalRenderer),
        _ => MermaidPlan::Render,
    }
}

/// The resolved state of one diagram. Data only — [`lines`] is what the user
/// actually sees, and it is pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MermaidDiagram {
    /// Rung 1: a PNG on disk, waiting to be painted inline.
    Rendered {
        png: PathBuf,
        protocol: GraphicsProtocol,
    },
    /// A raster is in flight on the worker thread.
    Rendering,
    /// Rung 2 / rung 3: the honest fallback, the source itself.
    Source { why: Why },
    /// Rung 4: the render failed; the source stays beside the error.
    Failed { error: String },
}

impl MermaidDiagram {
    /// The rasterised PNG, if the diagram made it that far.
    fn png(&self) -> Option<&Path> {
        match self {
            Self::Rendered { png, .. } => Some(png),
            _ => None,
        }
    }

    /// Test hook: `Failed` fixtures in the cache tests carry their source in
    /// the error field, so an assertion can tell two diagrams apart.
    #[cfg(test)]
    fn source_tag(&self) -> Option<&str> {
        match self {
            Self::Failed { error } => Some(error),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// The cache — the per-frame safety property
// ---------------------------------------------------------------------------

/// One cached diagram. `Pending` holds the worker thread's result slot;
/// `Done` is final and is never resolved again.
#[derive(Debug)]
enum Slot {
    Pending(Arc<Mutex<Option<MermaidDiagram>>>),
    Done(MermaidDiagram),
}

thread_local! {
    /// Diagram-source hash -> resolved diagram.
    ///
    /// This map is why an unchanged transcript does not re-spawn `npx` sixty
    /// times a second. The key is `hash_content(source)` and nothing volatile,
    /// so the same diagram resolves once and stays resolved for the life of the
    /// process.
    // Not `const { .. }`: `HashMap::new` is not a const fn, and the map is
    // built once per thread on first access either way.
    static CACHE: RefCell<HashMap<u64, Slot>> = RefCell::new(HashMap::new());
}

/// Cache bookkeeping, with the spawn factored out so it is testable without a
/// subprocess, a terminal or a renderer.
///
/// `start` is called at most once per key: the only transition into `Pending`.
fn cached_resolve(
    cache: &mut HashMap<u64, Slot>,
    key: u64,
    start: impl FnOnce() -> Arc<Mutex<Option<MermaidDiagram>>>,
) -> MermaidDiagram {
    match cache.get_mut(&key) {
        Some(Slot::Done(done)) => done.clone(),
        Some(Slot::Pending(slot)) => slot
            .lock()
            .ok()
            .and_then(|ready| ready.clone())
            .unwrap_or(MermaidDiagram::Rendering),
        None => {
            let slot = start();
            if cache.len() >= MAX_CACHED_DIAGRAMS {
                // ponytail: a plain clear rather than an LRU. A transcript that
                // reaches 256 distinct diagrams is already pathological, and
                // the cost of getting this wrong (evicting the wrong entry) is
                // a re-render, not a wrong picture. Promote an LRU if this
                // ever shows up in a profile.
                cache.clear();
            }
            cache.insert(key, Slot::Pending(slot));
            MermaidDiagram::Rendering
        }
    }
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Does this code-fence info string name a mermaid diagram?
///
/// Only the first whitespace-delimited token is read, so the common
/// ```` ```mermaid title="deploy" ```` form is recognised too.
pub fn is_mermaid_lang(info: &str) -> bool {
    info.split_whitespace()
        .next()
        .is_some_and(|tag| tag.eq_ignore_ascii_case("mermaid"))
}

/// The lines one diagram contributes to the transcript.
pub fn diagram_lines(source: &str, indent: &str) -> Vec<Line<'static>> {
    let diagram = resolve(source);
    lines(&diagram, source, indent)
}

/// Resolve one diagram, starting a render only on its first sighting.
///
/// Never blocks: the raster runs on a worker thread, so the frame that
/// discovers the diagram shows "rendering…" and returns immediately.
pub fn resolve(source: &str) -> MermaidDiagram {
    let env = MermaidEnv::detect();
    match plan(env) {
        MermaidPlan::Source(why) => MermaidDiagram::Source { why },
        MermaidPlan::Render => {
            let Some(renderer) = env.renderer else {
                return MermaidDiagram::Source {
                    why: Why::NoExternalRenderer,
                };
            };
            let key = hash_content(source);
            CACHE.with(|cache| {
                cached_resolve(&mut cache.borrow_mut(), key, || {
                    let slot = Arc::new(Mutex::new(None));
                    let worker = Arc::clone(&slot);
                    let owned = source.to_string();
                    std::thread::spawn(move || {
                        let outcome = rasterise(&owned, &renderer, env.protocol);
                        if let Ok(mut ready) = worker.lock() {
                            *ready = Some(outcome);
                        }
                    });
                    slot
                })
            })
        }
    }
}

// ---------------------------------------------------------------------------
// External renderer invocation
// ---------------------------------------------------------------------------

/// Run the external renderer over `source` and return the PNG it wrote.
///
/// Bounded twice over, because a subprocess that hangs is a TUI freeze and a
/// subprocess that prints without limit is an OOM: a wall-clock timeout, and a
/// size ceiling on both the stderr read back and the PNG.
fn rasterise(
    source: &str,
    renderer: &ExternalRenderer,
    protocol: GraphicsProtocol,
) -> MermaidDiagram {
    // Not a `tempfile::TempDir`: the PNG outlives this call (the frame that
    // discovers the diagram is not the frame that paints it), so the caller
    // owns the file and removes it once the escape sequence is built.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!("operant-mermaid-{stamp}"));
    let input = base.with_extension("mmd");
    let output = base.with_extension("png");

    if let Err(err) = std::fs::write(&input, source) {
        return MermaidDiagram::Failed {
            error: format!("cannot write diagram: {err}"),
        };
    }

    // `--size`, not the old `-w`: mermaid-cli renamed it. Verified against
    // `mmdc --help` at @mermaid-js/mermaid-cli 12.0.0.
    let mut child = match Command::new(renderer.program)
        .args(renderer.args)
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("-b")
        .arg("transparent")
        .arg("--size")
        .arg(RASTER_SIZE)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            let _ = std::fs::remove_file(&input);
            return MermaidDiagram::Failed {
                error: format!("{renderer}: {err}"),
            };
        }
    };

    // stderr goes to its own thread with a bounded read, so a child that
    // prints past the pipe buffer cannot wedge the poll loop below.
    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || stderr.map_or_else(String::new, read_capped));

    let deadline = Instant::now() + RENDER_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                break None;
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stderr = reader.join().unwrap_or_default();
    let _ = std::fs::remove_file(&input);

    let Some(status) = status else {
        let _ = std::fs::remove_file(&output);
        return MermaidDiagram::Failed {
            error: clamp_error(format!(
                "{renderer} did not finish within {}s{}",
                RENDER_TIMEOUT.as_secs(),
                detail(&stderr)
            )),
        };
    };

    if !status.success() {
        let _ = std::fs::remove_file(&output);
        return MermaidDiagram::Failed {
            error: clamp_error(if stderr.is_empty() {
                format!("{renderer} exited with {status}")
            } else {
                stderr
            }),
        };
    }

    match std::fs::metadata(&output) {
        Ok(meta) if meta.len() > MAX_PNG_BYTES => {
            let _ = std::fs::remove_file(&output);
            MermaidDiagram::Failed {
                error: format!(
                    "rendered PNG is {} KiB, over the {} KiB ceiling",
                    meta.len() / 1024,
                    MAX_PNG_BYTES / 1024
                ),
            }
        }
        Ok(_) => MermaidDiagram::Rendered {
            png: output,
            protocol,
        },
        Err(err) => MermaidDiagram::Failed {
            error: format!("{renderer} wrote no PNG: {err}"),
        },
    }
}

/// Read at most [`MAX_STDERR_BYTES`] from a pipe, then stop.
///
/// Bounded on purpose. Dropping the read end makes a child that keeps writing
/// take `EPIPE` and stop, instead of growing this process without limit.
fn read_capped(mut pipe: impl Read) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    while buf.len() < MAX_STDERR_BYTES
        && let Ok(n) = pipe.read(&mut chunk)
        && n > 0
    {
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&buf).trim().to_string()
}

/// ` — <stderr>` when the renderer said something, empty when it did not.
fn detail(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(" — {stderr}")
    }
}

/// Keep the renderer's own message, bounded: an unbounded error dump floods the
/// transcript, which is the same as showing nothing useful.
fn clamp_error(mut text: String) -> String {
    if text.chars().count() > MAX_ERROR_CHARS {
        text = text.chars().take(MAX_ERROR_CHARS).collect();
        text.push('…');
    }
    text
}

// ---------------------------------------------------------------------------
// Post-paint hand-off
// ---------------------------------------------------------------------------

/// What finished resolving since the last frame.
#[derive(Debug, Default)]
pub struct LandedRasters {
    /// A diagram left the `Pending` slot this frame, so the memoized transcript
    /// lines are stale and the caller must rebuild them. True even when the
    /// ladder landed on source text rather than a raster — the "rendering…"
    /// placeholder was replaced either way.
    pub resolved: bool,
    /// PNG paths to hand to the pinned-graphics registry, oldest first.
    pub pngs: Vec<PathBuf>,
}

/// Hand over every raster that landed since the last frame.
///
/// The `Pending` -> `Done` promotion happens here and nowhere else, so a diagram
/// is handed over exactly once. What the caller does with it is deliberately
/// NOT this module's business: the escape sequence has to be re-emitted on
/// every frame, because a painted image lives outside the cell grid and the next
/// redraw that touches its cells would destroy it. `tui::pinned_images` owns
/// that registry, so the write happens there.
///
/// Returns an empty report when nothing landed, which is the common case.
pub fn drain_ready_rasters() -> LandedRasters {
    let mut landed = LandedRasters::default();
    CACHE.with(|cache| {
        for slot in cache.borrow_mut().values_mut() {
            let Slot::Pending(inner) = slot else {
                continue;
            };
            let Some(done) = inner.lock().ok().and_then(|ready| ready.clone()) else {
                continue;
            };
            if let Some(png) = done.png() {
                landed.pngs.push(png.to_path_buf());
            }
            *slot = Slot::Done(done);
            landed.resolved = true;
        }
    });
    landed
}

// ---------------------------------------------------------------------------
// What the user sees
// ---------------------------------------------------------------------------

/// The lines a resolved diagram contributes. Pure.
fn lines(diagram: &MermaidDiagram, source: &str, indent: &str) -> Vec<Line<'static>> {
    let border = Style::default().fg(theme_colors::warning());
    let bar = || Span::styled(format!("{indent}\u{2502} "), border);
    match diagram {
        MermaidDiagram::Rendered { protocol, .. } => vec![Line::from(vec![
            Span::raw(indent.to_string()),
            bar(),
            Span::styled(
                format!("[mermaid] diagram rendered via {protocol}"),
                Style::default().fg(theme_colors::success()),
            ),
        ])],
        MermaidDiagram::Rendering => vec![Line::from(vec![
            Span::raw(indent.to_string()),
            bar(),
            Span::styled(
                "[mermaid] rendering… the first run of the external renderer can take a while",
                Style::default().fg(Color::DarkGray),
            ),
        ])],
        MermaidDiagram::Source { why } => {
            source_block(indent, "mermaid", why.message(), source, border)
        }
        MermaidDiagram::Failed { error } => {
            // Rung 4. The source comes first and the error lands under the box,
            // so the user sees what failed before they read why.
            let mut out = source_block(indent, "mermaid", "render failed", source, border);
            out.push(Line::from(vec![
                Span::raw(indent.to_string()),
                bar(),
                Span::styled(
                    format!("mermaid error: {error}"),
                    Style::default().fg(theme_colors::error()),
                ),
            ]));
            out
        }
    }
}

/// A bordered source block, shaped like the code-fence box in
/// `tui::messages::markdown` so a diagram and a code block read the same.
fn source_block(
    indent: &str,
    label: &str,
    note: &str,
    source: &str,
    border: Style,
) -> Vec<Line<'static>> {
    let rule = "\u{2500}".repeat(FENCE_WIDTH);
    let bar = || Span::styled(format!("{indent}\u{2502} "), border);
    let mut out = vec![
        Line::from(Span::styled(
            format!("{indent}\u{250C}{rule} {label} "),
            border,
        )),
        Line::from(vec![
            bar(),
            Span::styled(note.to_string(), Style::default().fg(Color::DarkGray)),
        ]),
    ];
    let body = Style::default().fg(theme_colors::text());
    for line in source.trim_end_matches('\n').lines() {
        out.push(Line::from(vec![
            bar(),
            Span::styled(line.to_string(), body),
        ]));
    }
    out.push(Line::from(Span::styled(
        format!("{indent}\u{2514}{rule}"),
        border,
    )));
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::image_render::PROTOCOL_FREE_ENV;

    const SRC: &str = "graph TD;\n  A-->B;";

    fn joined(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn with_renderer(protocol: GraphicsProtocol) -> MermaidEnv {
        MermaidEnv {
            protocol,
            renderer: Some(ExternalRenderer::MMDC),
        }
    }

    /// A ```mermaid fence is a diagram, not code to read. The ladder must
    /// recognise it, and must not leave the raw fence on screen.
    #[test]
    fn mermaid_should_detect_a_fenced_mermaid_block() {
        assert!(is_mermaid_lang("mermaid"));
        assert!(is_mermaid_lang("  Mermaid  "));
        assert!(is_mermaid_lang("mermaid title=\"deploy\""));
        assert!(!is_mermaid_lang("rust"));
        assert!(!is_mermaid_lang(""));
        assert!(!is_mermaid_lang("mermaidish"));

        // End to end through the real markdown renderer. A protocol-free
        // terminal also guarantees no subprocess is spawned by this test.
        let md = "```mermaid\ngraph TD;\n  A-->B;\n```\n";
        let rendered = image_render::with_env(&PROTOCOL_FREE_ENV, || {
            crate::tui::messages::render_markdown(md, 80)
        });
        let text = joined(&rendered);
        assert!(
            text.contains("no terminal graphics protocol"),
            "the diagram is recognised, not read as code: {text}"
        );
        assert!(
            text.contains("graph TD;"),
            "the diagram source stays on screen: {text}"
        );
        assert!(
            !text.contains("```"),
            "the raw fence must not be shown as source text: {text}"
        );
    }

    /// Rung 3: a terminal that can draw, but no mermaid toolchain installed.
    /// The user gets the diagram plus the one command that fixes it.
    #[test]
    fn mermaid_should_fall_back_to_source_when_no_renderer_available() {
        let env = MermaidEnv {
            protocol: GraphicsProtocol::Kitty,
            renderer: None,
        };
        assert_eq!(plan(&env), MermaidPlan::Source(Why::NoExternalRenderer));

        let text = joined(&lines(
            &MermaidDiagram::Source {
                why: Why::NoExternalRenderer,
            },
            SRC,
            "  ",
        ));
        assert!(text.contains("graph TD;"), "source is shown: {text}");
        assert!(
            text.contains("no mermaid renderer"),
            "reason is stated: {text}"
        );
        assert!(
            text.contains("@mermaid-js/mermaid-cli"),
            "the fix is named: {text}"
        );
    }

    /// Rung 4: a malformed diagram shows the source *and* the renderer's own
    /// error. Never a silent blank, and never an unbounded error dump.
    #[test]
    fn mermaid_should_surface_the_renderer_error_for_a_malformed_diagram() {
        let stderr = "Parse error on line 3:\nExpected 'end of statement'";
        let diagram = MermaidDiagram::Failed {
            error: clamp_error(stderr.to_string()),
        };
        let text = joined(&lines(&diagram, SRC, "  "));
        assert!(
            text.contains("Parse error on line 3"),
            "the renderer's own error is surfaced: {text}"
        );
        assert!(
            text.contains("graph TD;"),
            "the source stays visible beside the error: {text}"
        );
        assert!(text.contains("render failed"), "and it is labelled: {text}");

        // A renderer that prints a megabyte of diagnostics must not be able to
        // flood the transcript.
        let clamped = clamp_error("e".repeat(MAX_ERROR_CHARS * 4));
        assert_eq!(clamped.chars().count(), MAX_ERROR_CHARS + 1);
    }

    /// The protocol ladder's order is the contract: a terminal that cannot draw
    /// never gets a subprocess spawned, and the auto-detect order decides which
    /// protocol a diagram would use.
    #[test]
    fn mermaid_should_report_the_protocol_it_would_use() {
        // Detection order — most capable protocol first.
        let kitty = image_render::with_env(
            &[
                ("TERM", Some("xterm-256color")),
                ("KITTY_WINDOW_ID", Some("1")),
                ("TERM_PROGRAM", Some("iTerm.app")),
                ("ITERM_SESSION_ID", Some("w0t0p0")),
            ],
            image_render::detect_graphics_protocol,
        );
        assert_eq!(kitty, GraphicsProtocol::Kitty);

        let iterm = image_render::with_env(
            &[
                ("TERM", Some("xterm-256color")),
                ("KITTY_WINDOW_ID", None),
                ("TERM_PROGRAM", Some("iTerm.app")),
                ("ITERM_SESSION_ID", Some("w0t0p0")),
            ],
            image_render::detect_graphics_protocol,
        );
        assert_eq!(iterm, GraphicsProtocol::ITerm2);

        let sixel = image_render::with_env(
            &[
                ("TERM", Some("xterm-256color")),
                ("KITTY_WINDOW_ID", None),
                ("TERM_PROGRAM", None),
                ("ITERM_SESSION_ID", None),
            ],
            image_render::detect_graphics_protocol,
        );
        assert_eq!(sixel, GraphicsProtocol::Sixel);

        // Each of those reaches rung 1 — the diagram is rasterised and drawn.
        for protocol in [
            GraphicsProtocol::Kitty,
            GraphicsProtocol::ITerm2,
            GraphicsProtocol::Sixel,
        ] {
            assert_eq!(plan(&with_renderer(protocol)), MermaidPlan::Render);
        }

        // Rung 2 is checked before rung 3: a protocol-free terminal never gets
        // a renderer spawned on its behalf, even when one is installed.
        let drawless = MermaidEnv {
            protocol: GraphicsProtocol::None,
            renderer: Some(ExternalRenderer::MMDC),
        };
        assert_eq!(
            plan(&drawless),
            MermaidPlan::Source(Why::NoGraphicsProtocol)
        );

        // The plan reports which protocol the diagram would have used, so the
        // transcript line and the paint agree.
        let rendered = MermaidDiagram::Rendered {
            png: PathBuf::from("/tmp/nope.png"),
            protocol: GraphicsProtocol::Sixel,
        };
        assert!(joined(&lines(&rendered, SRC, "")).contains("sixel"));
    }

    /// The cache key is the diagram source. An unchanged transcript resolves
    /// once and stays resolved — the render is started at most once, no matter
    /// how many frames go by.
    #[test]
    fn mermaid_cache_should_hit_for_unchanged_source() {
        let mut cache: HashMap<u64, Slot> = HashMap::new();
        let key = hash_content(SRC);
        let mut starts = 0u32;
        let done = MermaidDiagram::Rendered {
            png: PathBuf::from("/tmp/diagram.png"),
            protocol: GraphicsProtocol::Kitty,
        };

        // Frame 1 starts the render and does not wait for it.
        let first = cached_resolve(&mut cache, key, || {
            starts += 1;
            Arc::new(Mutex::new(Some(done.clone())))
        });
        assert_eq!(first, MermaidDiagram::Rendering);

        // Frames 2..60 observe the finished diagram and start nothing new.
        for _ in 0..59 {
            let again = cached_resolve(&mut cache, key, || {
                starts += 1;
                Arc::new(Mutex::new(Some(done.clone())))
            });
            assert_eq!(again, done, "an unchanged source serves the cached render");
        }

        assert_eq!(
            starts, 1,
            "an unchanged diagram must start at most one render"
        );
        assert_eq!(cache.len(), 1);
    }

    /// Two different sources are two different diagrams. Editing one must not
    /// serve the other's render, and revisiting either must not re-spawn.
    #[test]
    fn mermaid_cache_should_miss_when_source_changes() {
        let mut cache: HashMap<u64, Slot> = HashMap::new();
        let other = "graph LR;\n  C-->D;";
        let mut starts = 0u32;

        // The rendered value carries its own source, so serving a neighbour's
        // render is visible in the assertion rather than invisible in a count.
        let step = |cache: &mut HashMap<u64, Slot>, source: &str, starts: &mut u32| {
            let key = hash_content(source);
            cached_resolve(cache, key, || {
                *starts += 1;
                Arc::new(Mutex::new(Some(MermaidDiagram::Failed {
                    error: source.to_string(),
                })))
            })
        };

        let seen: Vec<MermaidDiagram> = [SRC, SRC, other, SRC, other]
            .iter()
            .map(|source| step(&mut cache, source, &mut starts))
            .collect();

        assert_eq!(
            starts, 2,
            "one render per distinct source, not one per frame"
        );
        assert_eq!(cache.len(), 2);
        // Positions 0 and 2 are first sightings, so they start a render and do
        // not wait for it. Positions 1, 3 and 4 are revisits, and each reads
        // its own source's render back out of the cache.
        assert_eq!(seen[0], MermaidDiagram::Rendering);
        assert_eq!(seen[2], MermaidDiagram::Rendering);
        assert_eq!(
            seen[1], seen[3],
            "revisiting a source serves its own render"
        );
        assert_eq!(
            seen[1].source_tag(),
            Some(SRC),
            "the cache answered with SRC"
        );
        assert_eq!(
            seen[4].source_tag(),
            Some(other),
            "and with `other`, never the neighbour's"
        );
    }

    /// A slow renderer must not stall the frame that discovered the diagram.
    #[test]
    fn mermaid_should_report_pending_while_the_worker_runs() {
        let mut cache: HashMap<u64, Slot> = HashMap::new();
        let key = hash_content(SRC);

        let slow = cached_resolve(&mut cache, key, || Arc::new(Mutex::new(None)));
        assert_eq!(slow, MermaidDiagram::Rendering, "no blocking on the worker");

        // The worker fills the slot; the next frame picks it up.
        let slot = match cache.get(&key) {
            Some(Slot::Pending(slot)) => Arc::clone(slot),
            other => panic!("expected a pending slot, got {other:?}"),
        };
        *slot.lock().unwrap_or_else(|err| err.into_inner()) = Some(MermaidDiagram::Source {
            why: Why::NoGraphicsProtocol,
        });

        let settled = cached_resolve(&mut cache, key, || panic!("must not re-spawn"));
        assert_eq!(
            settled,
            MermaidDiagram::Source {
                why: Why::NoGraphicsProtocol
            }
        );
    }
}
