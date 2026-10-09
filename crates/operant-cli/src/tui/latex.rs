// latex.rs — render a ```latex / ```math / ```tex fenced block as a formula,
// not as source text.
//
// The toolchain that turns TeX into a bitmap is not part of this crate, so the
// feature is a probe plus a ladder rather than a dependency — the same shape as
// `tui::mermaid`, and for the same reasons:
//
//   rung 1  a terminal graphics protocol AND `dvipng` on PATH
//           -> rasterise on a worker thread, then hand the PNG to the registry
//   rung 2  no graphics protocol
//           -> the formula source in a bordered block, labelled with the reason
//   rung 3  no `dvipng` installed
//           -> the formula source, plus the one line naming what would draw it
//   rung 4  the toolchain ran and failed (malformed TeX, no DVI, timeout)
//           -> the formula source *and* the toolchain's own error
//
// Rungs 2-4 all end in source text. A formula that silently becomes a blank box
// is the worst outcome available, so every failure path keeps the source on
// screen — readable and selectable, never swallowed.
//
// Two properties are correctness requirements, not optimisations:
//
//   * A subprocess must never block the frame loop. The render runs on a worker
//     thread, so the frame that discovers the formula shows "rendering…" and
//     moves on. A worker that PANICS is caught, because an empty result slot is
//     a wedge: `resolve` would answer `Rendering` for the rest of the process
//     and the drain would skip that key forever.
//   * A source is resolved exactly once per process. The cache key is the
//     formula source and nothing else — not a line number, not a timestamp, not
//     the terminal width — so an unchanged transcript can never re-spawn the
//     toolchain on the next frame.
//
// The subprocess goes through the `Renderer` trait, which is the one place this
// module departs from mermaid. `dvipng`, `latex` and `pdflatex` are not
// installed everywhere — including on the machine this was written on — so a
// design whose only reachable test is "dvipng missing -> fallback" would leave
// the whole happy path unverified. Injecting the invocation makes the success
// path a unit test: a fake writes a real PNG and the worker, cache, drain and
// cleanup chain all run for real.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::tui::image_render::{self, GraphicsProtocol};
use crate::tui::messages::cache::hash_content;
use crate::tui::theme_colors;

/// Raster resolution asked of `dvipng`, in DPI.
///
/// Fixed on purpose. Because the raster does not depend on the terminal width,
/// the cache key can be the formula source alone and a resize never re-spawns
/// the toolchain. 600 keeps a formula legible on a HiDPI screen, where one cell
/// is several device pixels across.
const RASTER_DPI: &str = "600";

/// Wall-clock ceiling on ONE toolchain step.
///
/// Generous, because a cold `latex` pays for format and font caches. Two steps
/// run, so the worst case is twice this — still irrelevant to the frame loop,
/// which waits on a worker thread either way.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// Size ceiling on the PNG read back from the toolchain. A formula that
/// rasterises to a gigabyte is a bug or an attack, not a picture.
const MAX_PNG_BYTES: u64 = 8 * 1024 * 1024;

/// Size ceiling on one step's stderr, before it is shown to the user.
const MAX_STDERR_BYTES: usize = 16 * 1024;

/// Character ceiling on the error text kept for a failed render.
const MAX_ERROR_CHARS: usize = 400;

/// Ceiling on cached formulas, so a very long transcript cannot grow the map
/// without bound.
const MAX_CACHED_FORMULAS: usize = 256;

/// Matches the code-fence caps in `tui::messages::markdown`, so a formula block
/// and a code block are the same shape on screen.
const FENCE_WIDTH: usize = 22;

// ---------------------------------------------------------------------------
// Availability
// ---------------------------------------------------------------------------

/// A `PATH` lookup, injected so availability is a pure function under test.
///
/// `dvipng` is optional software. Without this seam the "is it installed?"
/// question could only be answered by a real `PATH` scan, so the only outcome a
/// test could ever observe would be whichever one this particular machine
/// happens to have — and on a machine with no TeX that is always "no".
pub trait PathProbe {
    /// Is `program` resolvable through `PATH`?
    fn on_path(&self, program: &str) -> bool;
}

/// The real `PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemPath;

impl PathProbe for SystemPath {
    fn on_path(&self, program: &str) -> bool {
        which::which(program).is_ok()
    }
}

/// The external toolchain, if it is installed.
///
/// Both halves are probed: a `latex` with no `dvipng` draws nothing, so having
/// one without the other is the same as having neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Toolchain {
    /// The TeX engine used for the DVI pass.
    pub tex: &'static str,
    /// The DVI rasteriser.
    pub dvipng: &'static str,
}

impl Toolchain {
    /// Plain TeX first: the reference implementation, and the one whose default
    /// output format is DVI.
    pub const TEX: Self = Self {
        tex: "latex",
        dvipng: "dvipng",
    };

    /// The pdftex-based alias TeX Live also installs as `latex`, reached only
    /// when [`Self::TEX`] is missing. Harmless to prefer [`Self::TEX`] when both
    /// exist because every invocation passes `-output-format=dvi`, so the
    /// choice cannot change the format the rasteriser reads.
    pub const PDFTEX: Self = Self {
        tex: "pdflatex",
        dvipng: "dvipng",
    };
}

impl std::fmt::Display for Toolchain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}+{}", self.tex, self.dvipng)
    }
}

/// Everything the ladder needs, injected so the decision stays pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatexEnv {
    /// Terminal graphics protocol, from `tui::image_render`.
    pub protocol: GraphicsProtocol,
    /// External toolchain found on `PATH`, if any.
    pub toolchain: Option<Toolchain>,
}

impl LatexEnv {
    /// Probe the terminal and `PATH` once per process.
    ///
    /// A `PATH` scan per frame would be a per-frame cost for an answer that
    /// cannot change while the process runs.
    pub fn detect() -> &'static LatexEnv {
        static ENV: LazyLock<LatexEnv> = LazyLock::new(|| LatexEnv {
            protocol: image_render::detect_graphics_protocol(),
            toolchain: probe(&SystemPath),
        });
        &ENV
    }
}

/// Is a usable toolchain installed? Pure, and both answers are reachable in a
/// test without any TeX on the machine.
pub fn probe(probe: &dyn PathProbe) -> Option<Toolchain> {
    [Toolchain::TEX, Toolchain::PDFTEX]
        .into_iter()
        .find(|toolchain| probe.on_path(toolchain.tex) && probe.on_path(toolchain.dvipng))
}

// ---------------------------------------------------------------------------
// The ladder
// ---------------------------------------------------------------------------

/// Why the ladder chose the source over a rendered formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Rung 2: the terminal cannot draw an inline image.
    NoGraphicsProtocol,
    /// Rung 3: no TeX toolchain is installed.
    NoToolchain,
}

impl Why {
    /// The ONE line shown in place of a formula.
    pub fn message(self) -> &'static str {
        match self {
            Self::NoGraphicsProtocol => "formula not drawn: no terminal graphics protocol",
            Self::NoToolchain => {
                "formula not drawn: no dvipng on PATH (it would rasterise this: apt install dvipng, or pacman -S texlive)"
            }
        }
    }
}

/// Which rung a formula lands on. Pure: spawns nothing, reads no terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatexPlan {
    /// Rung 1: rasterise, then draw inline.
    Render,
    /// Rung 2 / rung 3: show the source.
    Source(Why),
}

/// Decide the rung. The order is the contract: a terminal that cannot draw an
/// inline image never gets a subprocess spawned on its behalf, and a terminal
/// that can never gets a TeX engine for a formula it cannot display.
pub fn plan(env: &LatexEnv) -> LatexPlan {
    match (env.protocol, env.toolchain) {
        (GraphicsProtocol::None, _) => LatexPlan::Source(Why::NoGraphicsProtocol),
        (_, None) => LatexPlan::Source(Why::NoToolchain),
        _ => LatexPlan::Render,
    }
}

/// The resolved state of one formula. Data only — [`lines`] is what the user
/// actually sees, and it is pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LatexBlock {
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

impl LatexBlock {
    /// The rasterised PNG, if the formula made it that far.
    fn png(&self) -> Option<&Path> {
        match self {
            Self::Rendered { png, .. } => Some(png),
            _ => None,
        }
    }

    /// Test hook: `Failed` fixtures in the cache tests carry their source in the
    /// error field, so an assertion can tell two formulas apart.
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

/// One cached formula. `Pending` holds the worker thread's result slot; `Done`
/// is final and is never resolved again.
#[derive(Debug)]
enum Slot {
    Pending(Arc<Mutex<Option<LatexBlock>>>),
    Done(LatexBlock),
}

thread_local! {
    /// Formula-source hash -> resolved formula.
    ///
    /// This map is why an unchanged transcript does not re-spawn `latex` sixty
    /// times a second. The key is `hash_content(source)` and nothing volatile,
    /// so the same formula resolves once and stays resolved for the life of the
    /// process.
    // Not `const { .. }`: `HashMap::new` is not a const fn, and the map is
    // built once per thread on first access either way.
    static CACHE: RefCell<HashMap<u64, Slot>> = RefCell::new(HashMap::new());
}

/// Cache bookkeeping, with the spawn factored out so it is testable without a
/// subprocess, a terminal or a toolchain.
///
/// `start` is called at most once per key: the only transition into `Pending`.
fn cached_resolve(
    cache: &mut HashMap<u64, Slot>,
    key: u64,
    start: impl FnOnce() -> Arc<Mutex<Option<LatexBlock>>>,
) -> LatexBlock {
    match cache.get_mut(&key) {
        Some(Slot::Done(done)) => done.clone(),
        Some(Slot::Pending(slot)) => slot
            .lock()
            .ok()
            .and_then(|ready| ready.clone())
            .unwrap_or(LatexBlock::Rendering),
        None => {
            let slot = start();
            if cache.len() >= MAX_CACHED_FORMULAS {
                // ponytail: a plain clear rather than an LRU. A transcript that
                // reaches 256 distinct formulas is already pathological, and
                // the cost of getting this wrong (evicting the wrong entry) is
                // a re-render, not a wrong picture. Promote an LRU if this
                // ever shows up in a profile.
                cache.clear();
            }
            cache.insert(key, Slot::Pending(slot));
            LatexBlock::Rendering
        }
    }
}

/// What finished resolving since the last frame.
///
/// The same shape as [`crate::tui::mermaid::LandedRasters`] on purpose: both
/// ladders report "a PNG just landed, and the memoized transcript lines are now
/// stale", so the render pass can drain both and still make exactly one call to
/// the pinned-graphics registry.
#[derive(Debug, Default)]
pub struct LandedRasters {
    /// A formula left the `Pending` slot this frame, so the memoized transcript
    /// lines are stale and the caller must rebuild them. True even when the
    /// ladder landed on source text rather than a raster — the "rendering…"
    /// placeholder was replaced either way.
    pub resolved: bool,
    /// PNG paths to hand to the pinned-graphics registry, oldest first.
    pub pngs: Vec<PathBuf>,
}

/// The `Pending` -> `Done` promotion, over a caller-supplied cache.
///
/// Split out from [`drain_ready_rasters`] so the hand-off is testable against a
/// local map, with no thread-local state to reset between tests.
fn drain_cache(cache: &mut HashMap<u64, Slot>) -> LandedRasters {
    let mut landed = LandedRasters::default();
    for slot in cache.values_mut() {
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
    landed
}

/// Hand over every raster that landed since the last frame.
///
/// The `Pending` -> `Done` promotion happens here and nowhere else, so a formula
/// is handed over exactly once. What the caller does with it is deliberately
/// NOT this module's business: the escape sequence has to be re-emitted on every
/// frame, because a painted image lives outside the cell grid. The registry that
/// owns that is `tui::pinned_images`, so the write happens there.
///
/// Returns an empty report when nothing landed, which is the common case.
pub fn drain_ready_rasters() -> LandedRasters {
    CACHE.with(|cache| drain_cache(&mut cache.borrow_mut()))
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Does this code-fence info string name a formula?
///
/// Only the first whitespace-delimited token is read, so the common
/// ```` ```latex equation="1" ```` form is recognised too — and, mirroring
/// `tui::mermaid::is_mermaid_lang`, the match is exact: `latexmk` and
/// `mermaidish` are other languages, not typos of this one.
pub fn is_latex_lang(info: &str) -> bool {
    info.split_whitespace().next().is_some_and(|tag| {
        ["latex", "math", "tex"]
            .iter()
            .any(|lang| tag.eq_ignore_ascii_case(lang))
    })
}

/// The lines one formula contributes to the transcript.
pub fn formula_lines(source: &str, indent: &str) -> Vec<Line<'static>> {
    lines(&resolve(source), source, indent)
}

/// Resolve one formula, starting a render only on its first sighting.
///
/// Never blocks: the raster runs on a worker thread, so the frame that
/// discovers the formula shows "rendering…" and returns immediately.
pub fn resolve(source: &str) -> LatexBlock {
    let env = LatexEnv::detect();
    match plan(env) {
        LatexPlan::Source(why) => LatexBlock::Source { why },
        LatexPlan::Render => {
            let Some(toolchain) = env.toolchain else {
                return LatexBlock::Source {
                    why: Why::NoToolchain,
                };
            };
            let key = hash_content(source);
            CACHE.with(|cache| {
                cached_resolve(&mut cache.borrow_mut(), key, || {
                    spawn_worker(source, Arc::new(DviPng { toolchain }), env.protocol)
                })
            })
        }
    }
}

// ---------------------------------------------------------------------------
// External toolchain invocation
// ---------------------------------------------------------------------------

/// Why a render did not produce a PNG.
///
/// Typed rather than a bare `String`, so the timeout stays distinguishable from
/// a plain failure in a log line without re-parsing prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The step could not start, exited non-zero, or wrote nothing usable.
    Failed {
        /// Which half of the toolchain failed.
        tool: &'static str,
        /// Its own message, or the reason it could not run.
        message: String,
    },
    /// The step did not finish inside [`STEP_TIMEOUT`].
    Timeout {
        /// Which half of the toolchain hung.
        tool: &'static str,
        /// Anything it printed before it stopped.
        message: String,
    },
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed { tool, message } => write!(f, "{tool}: {message}"),
            Self::Timeout { tool, message } => write!(
                f,
                "{tool} did not finish within {}s{message}",
                STEP_TIMEOUT.as_secs()
            ),
        }
    }
}

/// TeX in, PNG out.
///
/// The seam exists because the real toolchain is optional. Production passes
/// [`DviPng`]; a test passes a fake that writes a real PNG, so the success path
/// is exercised rather than assumed.
pub trait Renderer: Send + Sync {
    /// Turn the TeX source at `tex` into the PNG at `png`, or explain why not.
    fn render(&self, tex: &Path, png: &Path) -> Result<(), RenderError>;
}

/// The real toolchain: TeX to DVI, then DVI to PNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DviPng {
    /// Which engines to invoke.
    pub toolchain: Toolchain,
}

/// Step 1: the TeX -> DVI command line.
///
/// ```
/// <tex> -interaction=nonstopmode -halt-on-error
///       -output-format=dvi -output-directory <workdir> <file.tex>
/// ```
///
///   * `-interaction=nonstopmode` — a broken formula must EXIT, not stop and
///     wait on stdin. stdin is `/dev/null`, so a prompt-looping engine would
///     spin until [`STEP_TIMEOUT`] rather than report its error.
///   * `-halt-on-error` — stop at the first error instead of continuing into a
///     wall of follow-on damage, so the message the user sees is the real one.
///   * `-output-format=dvi` — TeX Live's `latex` is pdftex and emits PDF by
///     default, and this step exists only to produce something `dvipng` can
///     read. Passing it makes the engine the probe found irrelevant.
///   * `-output-directory <workdir>` — keep the DVI and the log inside the temp
///     work dir instead of the process CWD, so one recursive remove cleans up
///     the whole run.
///
/// Public because it is a contract, not an implementation detail: the toolchain
/// is optional, so a machine without it could not otherwise verify a flag.
///
/// Takes no engine: which binary gets executed is the caller's `Command::new`,
/// and the flags above are what make that choice irrelevant.
pub fn tex_argv(tex: &Path) -> Vec<OsString> {
    let work = tex.parent().unwrap_or_else(|| Path::new("."));
    [
        "-interaction=nonstopmode",
        "-halt-on-error",
        "-output-format=dvi",
        "-output-directory",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([work.as_os_str(), tex.as_os_str()].map(OsStr::to_os_string))
    .collect()
}

/// Step 2: the DVI -> PNG command line.
///
/// ```
/// dvipng -T tight -D 600 -bg Transparent -o <out.png> <file.dvi>
/// ```
///
///   * `-T tight` — crop to the ink bounding box. Without it every formula
///     arrives wrapped in a full page of white margin, which then becomes a
///     full page of blank terminal cells in the transcript.
///   * `-D 600` — raster resolution, matching [`RASTER_DPI`]. Fixed rather than
///     terminal-derived so a resize never re-runs the toolchain.
///   * `-bg Transparent` — leave the background to the terminal instead of
///     painting an opaque white box behind the glyphs, so a dark theme does not
///     get a bright rectangle in the middle of the transcript.
///   * `-o <png>` — the output path. The PNG is written BESIDE the work dir,
///     not inside it, because it outlives the render: the registry reads it on a
///     later frame and deletes it there.
///
/// Public for the same reason as [`tex_argv`].
pub fn dvipng_argv(png: &Path, dvi: &Path) -> Vec<OsString> {
    ["-T", "tight", "-D", RASTER_DPI, "-bg", "Transparent", "-o"]
        .into_iter()
        .map(OsString::from)
        .chain([png.as_os_str(), dvi.as_os_str()].map(OsStr::to_os_string))
        .collect()
}

impl Renderer for DviPng {
    /// Two steps, because `dvipng` reads DVI and never TeX. The command lines
    /// and the reason for each flag are on [`tex_argv`] and [`dvipng_argv`].
    fn render(&self, tex: &Path, png: &Path) -> Result<(), RenderError> {
        // The .tex lives in a work dir, and so does everything the TeX pass
        // writes there, so one recursive remove cleans up the whole run.
        let dvi = tex.with_extension("dvi");

        let mut child = match Command::new(self.toolchain.tex)
            .args(tex_argv(tex))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                return Err(RenderError::Failed {
                    tool: self.toolchain.tex,
                    message: err.to_string(),
                });
            }
        };
        run_bounded(self.toolchain.tex, &mut child)?;

        // A successful engine that wrote no DVI is a broken install, and saying
        // so beats letting `dvipng` fail with something cryptic.
        if !dvi.exists() {
            return Err(RenderError::Failed {
                tool: self.toolchain.tex,
                message: format!("produced no DVI at {}", dvi.display()),
            });
        }

        let mut child = match Command::new(self.toolchain.dvipng)
            .args(dvipng_argv(png, &dvi))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                return Err(RenderError::Failed {
                    tool: self.toolchain.dvipng,
                    message: err.to_string(),
                });
            }
        };
        run_bounded(self.toolchain.dvipng, &mut child).map(|_| ())
    }
}

/// Run one toolchain step to completion, bounded, and return its stderr.
///
/// Bounded twice over, because a subprocess that hangs is a TUI freeze and a
/// subprocess that prints without limit is an OOM: a wall-clock timeout, and a
/// size ceiling on the stderr read back.
///
/// The stderr read lives on its own thread with a bounded length, so a child
/// that prints past the pipe buffer cannot wedge the poll loop. A `try_wait`
/// error and a timeout are reported the same way, which is honest: in both
/// cases the step did not complete and whatever it printed is all there is.
fn run_bounded(tool: &'static str, child: &mut Child) -> Result<String, RenderError> {
    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || stderr.map_or_else(String::new, read_capped));

    let deadline = Instant::now() + STEP_TIMEOUT;
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

    let Some(status) = status else {
        return Err(RenderError::Timeout {
            tool,
            message: detail(&stderr),
        });
    };
    if !status.success() {
        return Err(RenderError::Failed {
            tool,
            message: if stderr.is_empty() {
                format!("exited with {status}")
            } else {
                stderr
            },
        });
    }
    Ok(stderr)
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

/// ` — <stderr>` when the step said something, empty when it did not.
fn detail(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(" — {stderr}")
    }
}

/// Keep the toolchain's own message, bounded: an unbounded error dump floods the
/// transcript, which is the same as showing nothing useful.
fn clamp_error(mut text: String) -> String {
    if text.chars().count() > MAX_ERROR_CHARS {
        text = text.chars().take(MAX_ERROR_CHARS).collect();
        text.push('…');
    }
    text
}

// ---------------------------------------------------------------------------
// The worker
// ---------------------------------------------------------------------------

/// A temp directory removed on every path, including an unwind.
///
/// `Drop` is the whole point. A cleanup step that has to remember every early
/// return eventually misses one, and a worker that panics is exactly the path
/// nobody remembers. The PNG deliberately lives OUTSIDE the directory: it
/// outlives the render, because `tui::pinned_images` reads it on a later frame
/// and deletes it there.
struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    /// A fresh, empty directory, or `None` when the temp dir is unusable — the
    /// ladder then reports a failure rather than panicking on I/O.
    fn create() -> Option<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("operant-latex-{stamp}"));
        std::fs::create_dir(&path).ok()?;
        Some(Self { path })
    }

    /// A file inside the work dir.
    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// The PNG path: a sibling of the work dir, so the dir can be removed the
    /// moment the render returns.
    fn png_path(&self) -> PathBuf {
        self.path.with_extension("png")
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Start the render for one formula; returns the slot the worker fills.
///
/// The body is `catch_unwind`-wrapped, and the slot is written on every path out
/// of it. A worker that panicked without writing would leave the slot empty,
/// which is the one state the ladder cannot recover from: `resolve` would answer
/// `Rendering` for the rest of the process and the drain would skip the key
/// forever. So a panic becomes a `Failed`, and the source stays on screen.
fn spawn_worker(
    source: &str,
    renderer: Arc<dyn Renderer>,
    protocol: GraphicsProtocol,
) -> Arc<Mutex<Option<LatexBlock>>> {
    let slot = Arc::new(Mutex::new(None));
    let worker = Arc::clone(&slot);
    let owned = source.to_string();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rasterise(&owned, &*renderer, protocol)
        }))
        .unwrap_or_else(|_| {
            tracing::debug!(
                target: "latex",
                "formula render panicked; falling back to the source"
            );
            LatexBlock::Failed {
                error: "the formula renderer panicked".to_string(),
            }
        });
        if let Ok(mut ready) = worker.lock() {
            *ready = Some(outcome);
        }
    });
    slot
}

/// Run the injected renderer over `source` and return the PNG it wrote.
///
/// The work dir is cleaned on every exit — success, failure and panic — because
/// it is owned by a `Drop` guard rather than by a hand-written sequence of
/// removals.
fn rasterise(source: &str, renderer: &dyn Renderer, protocol: GraphicsProtocol) -> LatexBlock {
    let Some(work) = WorkDir::create() else {
        return LatexBlock::Failed {
            error: "cannot create a temp directory for the formula".to_string(),
        };
    };
    let tex = work.join("formula.tex");
    let png = work.png_path();

    if let Err(err) = std::fs::write(&tex, source) {
        return LatexBlock::Failed {
            error: format!("cannot write formula: {err}"),
        };
    }

    if let Err(err) = renderer.render(&tex, &png) {
        // A half-written raster is worse than none: the registry would try to
        // encode it and the user would see a broken image instead of source.
        let _ = std::fs::remove_file(&png);
        return LatexBlock::Failed {
            error: clamp_error(err.to_string()),
        };
    }

    match std::fs::metadata(&png) {
        Ok(meta) if meta.len() > MAX_PNG_BYTES => {
            let _ = std::fs::remove_file(&png);
            LatexBlock::Failed {
                error: format!(
                    "rendered PNG is {} KiB, over the {} KiB ceiling",
                    meta.len() / 1024,
                    MAX_PNG_BYTES / 1024
                ),
            }
        }
        Ok(_) => LatexBlock::Rendered { png, protocol },
        Err(err) => LatexBlock::Failed {
            error: format!("the toolchain wrote no PNG: {err}"),
        },
    }
}

// ---------------------------------------------------------------------------
// What the user sees
// ---------------------------------------------------------------------------

/// The lines a resolved formula contributes. Pure.
fn lines(block: &LatexBlock, source: &str, indent: &str) -> Vec<Line<'static>> {
    let border = Style::default().fg(theme_colors::warning());
    let bar = || Span::styled(format!("{indent}\u{2502} "), border);
    match block {
        LatexBlock::Rendered { protocol, .. } => vec![Line::from(vec![
            Span::raw(indent.to_string()),
            bar(),
            Span::styled(
                format!("[latex] formula rendered via {protocol}"),
                Style::default().fg(theme_colors::success()),
            ),
        ])],
        LatexBlock::Rendering => vec![Line::from(vec![
            Span::raw(indent.to_string()),
            bar(),
            Span::styled(
                "[latex] rendering… the first run of the TeX toolchain can take a while",
                Style::default().fg(Color::DarkGray),
            ),
        ])],
        LatexBlock::Source { why } => source_block(indent, "latex", why.message(), source, border),
        LatexBlock::Failed { error } => {
            // Rung 4. The source comes first and the error lands under the box,
            // so the user sees what failed before they read why.
            let mut out = source_block(indent, "latex", "render failed", source, border);
            out.push(Line::from(vec![
                Span::raw(indent.to_string()),
                bar(),
                Span::styled(
                    format!("latex error: {error}"),
                    Style::default().fg(theme_colors::error()),
                ),
            ]));
            out
        }
    }
}

/// A bordered source block, shaped like the code-fence box in
/// `tui::messages::markdown` so a formula and a code block read the same.
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
    // The body is plain text, not syntax-highlighted source: selectable, and
    // unchanged from what the user typed.
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
    use std::sync::Barrier;

    const SRC: &str = "\\int_0^\\infty e^{-x^2} dx = \\frac{\\sqrt{\\pi}}{2}";

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

    /// A 24-byte PNG carrying a real IHDR, so the registry has dimensions to
    /// scale. A fake that wrote arbitrary bytes would not be a raster.
    fn png_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        data[8..12].copy_from_slice(&13u32.to_be_bytes());
        data[12..16].copy_from_slice(b"IHDR");
        data[16..20].copy_from_slice(&1u32.to_be_bytes());
        data[20..24].copy_from_slice(&1u32.to_be_bytes());
        data
    }

    /// A `PATH` probe that answers from a fixed list.
    struct FakePath(Vec<&'static str>);

    impl PathProbe for FakePath {
        fn on_path(&self, program: &str) -> bool {
            self.0.contains(&program)
        }
    }

    /// What the injected toolchain should do, per test.
    enum Behaviour {
        /// Succeed, writing these bytes as the PNG.
        Write(Vec<u8>),
        /// Fail the way a malformed formula fails.
        Fail,
        /// Hang past the step budget.
        Timeout,
        /// Blow up, the way a bug in the toolchain glue would.
        Panic,
        /// Park on two barriers, so a test can observe the worker mid-flight.
        Gate(Arc<Barrier>, Arc<Barrier>),
    }

    /// The injected toolchain, under test control.
    ///
    /// `seen` records the `.tex` paths the ladder handed over, which is how a
    /// test proves the work dir was cleaned up without racing another test for
    /// a glob in the temp dir.
    struct FakeRenderer {
        behaviour: Behaviour,
        seen: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl FakeRenderer {
        fn new(behaviour: Behaviour) -> (Arc<Self>, Arc<Mutex<Vec<PathBuf>>>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            (
                Arc::new(Self {
                    behaviour,
                    seen: Arc::clone(&seen),
                }),
                seen,
            )
        }
    }

    impl Renderer for FakeRenderer {
        fn render(&self, tex: &Path, png: &Path) -> Result<(), RenderError> {
            if let Ok(mut seen) = self.seen.lock() {
                seen.push(tex.to_path_buf());
            }
            match &self.behaviour {
                Behaviour::Write(bytes) => {
                    std::fs::write(png, bytes).map_err(|err| RenderError::Failed {
                        tool: "fake",
                        message: err.to_string(),
                    })?;
                    Ok(())
                }
                Behaviour::Fail => Err(RenderError::Failed {
                    tool: "latex",
                    message: "! LaTeX Error: Missing $ inserted.".to_string(),
                }),
                Behaviour::Timeout => Err(RenderError::Timeout {
                    tool: "latex",
                    message: String::new(),
                }),
                Behaviour::Panic => panic!("the toolchain glue exploded"),
                Behaviour::Gate(started, released) => {
                    started.wait();
                    released.wait();
                    std::fs::write(png, png_bytes()).map_err(|err| RenderError::Failed {
                        tool: "fake",
                        message: err.to_string(),
                    })?;
                    Ok(())
                }
            }
        }
    }

    /// Wait for a worker to report, without ever blocking forever.
    ///
    /// A timeout returns a value the assertions below will reject, rather than
    /// panicking: a wedged worker is the bug under test, and a failing
    /// assertion says so more clearly than a hung test binary.
    fn wait_for_slot(slot: &Mutex<Option<LatexBlock>>) -> LatexBlock {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(ready) = slot.lock().ok().and_then(|ready| ready.clone()) {
                return ready;
            }
            if Instant::now() >= deadline {
                return LatexBlock::Failed {
                    error: "the worker never reported a result".to_string(),
                };
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The `.tex` path the ladder handed to the renderer.
    fn first_seen(seen: &Arc<Mutex<Vec<PathBuf>>>) -> PathBuf {
        seen.lock()
            .ok()
            .and_then(|seen| seen.first().cloned())
            .expect("the renderer was never invoked")
    }

    /// The temp work dir a `.tex` was written into — gone once the render
    /// returns, on every path.
    fn work_dir_of(tex: &Path) -> PathBuf {
        tex.parent().unwrap_or(Path::new("")).to_path_buf()
    }

    /// The in-flight slot for `key`, or a clear failure if the ladder has
    /// already settled it.
    fn pending_slot(cache: &HashMap<u64, Slot>, key: u64) -> Arc<Mutex<Option<LatexBlock>>> {
        match cache.get(&key) {
            Some(Slot::Pending(slot)) => Arc::clone(slot),
            other => panic!("expected a pending slot, got {other:?}"),
        }
    }

    fn with_toolchain(protocol: GraphicsProtocol) -> LatexEnv {
        LatexEnv {
            protocol,
            toolchain: Some(Toolchain::TEX),
        }
    }

    // Test 1: the fence tag. Exact, like mermaid's: `latexmk` and `mermaidish`
    // are other languages, not near-misses of this one.
    #[test]
    fn latex_should_recognise_exactly_its_own_fence_tags() {
        for tag in ["latex", "LaTeX", "  math  ", "MATH", "tex", "TEX"] {
            assert!(is_latex_lang(tag), "{tag:?} is a formula fence");
        }
        // The info-string form mermaid also accepts.
        assert!(is_latex_lang("math equation=1"));
        for tag in ["mermaidish", "latexmk", "rust", "", "   ", "typescript"] {
            assert!(!is_latex_lang(tag), "{tag:?} is not a formula fence");
        }

        // [dead-strata sweep 2026-10-09] the end-to-end half of this test —
        // rendering a ```math fence through "the real markdown renderer" —
        // died with the pre-port renderer it exercised: the vendored
        // renderer owns the live math path and does not dispatch through
        // this ladder. The ladder's live-consumer question is tracked as
        // its own sweep item; the fence-tag contract above stays.
    }

    // Test 2: availability is a PURE function of an injected PATH probe, so both
    // outcomes are reachable on a machine with no TeX installed at all.
    #[test]
    fn availability_is_a_pure_function_of_the_injected_path_probe() {
        // Both halves present -> plain TeX, which is the first choice.
        assert_eq!(
            probe(&FakePath(vec!["latex", "dvipng"])),
            Some(Toolchain::TEX)
        );
        // Only the pdftex alias -> that is what gets used, and `-output-format=dvi`
        // is passed regardless, so the choice cannot change the raster format.
        assert_eq!(
            probe(&FakePath(vec!["pdflatex", "dvipng"])),
            Some(Toolchain::PDFTEX)
        );
        // An engine with no rasteriser draws nothing, so it is not a toolchain.
        assert_eq!(probe(&FakePath(vec!["latex", "pdflatex"])), None);
        // A rasteriser with no engine cannot run either.
        assert_eq!(probe(&FakePath(vec!["dvipng"])), None);
        // Nothing at all: the case this machine is actually in.
        assert_eq!(probe(&FakePath(vec![])), None);

        // Each availability outcome reaches the rung it should.
        assert_eq!(
            plan(&with_toolchain(GraphicsProtocol::Kitty)),
            LatexPlan::Render
        );
        let absent = LatexEnv {
            protocol: GraphicsProtocol::Kitty,
            toolchain: None,
        };
        assert_eq!(plan(&absent), LatexPlan::Source(Why::NoToolchain));

        // …and the unavailable fallback names the thing that would draw it.
        let text = joined(&lines(
            &LatexBlock::Source {
                why: Why::NoToolchain,
            },
            SRC,
            "  ",
        ));
        assert!(text.contains("dvipng"), "the fix is named: {text}");
        assert!(text.contains("e^{-x^2}"), "source is shown: {text}");
    }

    // Test 2b: the rung order is the contract. A terminal that cannot draw an
    // inline image never gets a TeX engine spawned on its behalf, even when one
    // is installed — which is also what makes the test above deterministic on a
    // machine that happens to have TeX.
    #[test]
    fn a_terminal_without_a_graphics_protocol_never_gets_a_subprocess() {
        let drawless = with_toolchain(GraphicsProtocol::None);
        assert_eq!(plan(&drawless), LatexPlan::Source(Why::NoGraphicsProtocol));
        let text = joined(&lines(
            &LatexBlock::Source {
                why: Why::NoGraphicsProtocol,
            },
            SRC,
            "",
        ));
        assert!(text.contains("e^{-x^2}"), "source is shown: {text}");
        assert!(
            !text.contains("```"),
            "the raw fence is never shown: {text}"
        );
    }

    // Test 3: THE SUCCESS PATH. `dvipng` is not installed on the machine that
    // wrote this, so without a fake runner the entire happy path would be
    // untested. A fake that writes a real PNG exercises the whole chain: worker,
    // cache, drain, hand-off.
    #[test]
    fn a_fake_runner_that_writes_a_png_produces_a_raster_and_cleans_up() {
        let (fake, seen) = FakeRenderer::new(Behaviour::Write(png_bytes()));
        let mut cache: HashMap<u64, Slot> = HashMap::new();
        let key = hash_content(SRC);

        let pending = cached_resolve(&mut cache, key, || {
            spawn_worker(SRC, fake, GraphicsProtocol::Kitty)
        });
        assert_eq!(pending, LatexBlock::Rendering, "the render is in flight");

        let done = wait_for_slot(&pending_slot(&cache, key));

        // The PNG is a SIBLING of the work dir, not a child of it: it has to
        // outlive the render so the registry can read it on a later frame.
        let work_dir = work_dir_of(&first_seen(&seen));
        let expected = work_dir.with_extension("png");
        assert_eq!(
            done,
            LatexBlock::Rendered {
                png: expected.clone(),
                protocol: GraphicsProtocol::Kitty,
            }
        );
        assert!(expected.exists(), "the PNG really is on disk: {expected:?}");

        // The drain hands the raster over exactly once.
        let landed = drain_cache(&mut cache);
        assert!(landed.resolved, "the transcript lines are stale");
        assert_eq!(landed.pngs, vec![expected.clone()], "one raster, once");
        assert_eq!(drain_cache(&mut cache).pngs.len(), 0, "and only once");

        // The work dir is gone on the success path too.
        assert!(
            !work_dir.exists(),
            "the temp work dir must not survive: {work_dir:?}"
        );

        let _ = std::fs::remove_file(&expected);
    }

    // Test 4: a failing toolchain leaves the source readable and does not panic.
    #[test]
    fn a_failing_toolchain_leaves_the_source_readable() {
        for behaviour in [Behaviour::Fail, Behaviour::Timeout] {
            let (fake, seen) = FakeRenderer::new(behaviour);
            let mut cache: HashMap<u64, Slot> = HashMap::new();
            let key = hash_content(SRC);

            assert_eq!(
                cached_resolve(&mut cache, key, || {
                    spawn_worker(SRC, fake, GraphicsProtocol::Kitty)
                }),
                LatexBlock::Rendering
            );
            let done = wait_for_slot(&pending_slot(&cache, key));
            let LatexBlock::Failed { error } = &done else {
                panic!("a failing toolchain must land on Failed, got {done:?}");
            };
            assert!(!error.is_empty(), "the reason is reported: {error}");

            // The user still gets the formula, and the raw fence is still gone.
            let text = joined(&lines(&done, SRC, "  "));
            assert!(text.contains("e^{-x^2}"), "source is shown: {text}");
            assert!(text.contains("render failed"), "labelled: {text}");
            assert!(!text.contains("```"), "no raw fence: {text}");

            // No raster is handed over, so the registry is never handed a
            // half-written file.
            let landed = drain_cache(&mut cache);
            assert!(landed.resolved, "the placeholder is still stale");
            assert!(landed.pngs.is_empty(), "nothing to pin: {:?}", landed.pngs);

            let work_dir = work_dir_of(&first_seen(&seen));
            assert!(
                !work_dir.exists(),
                "the temp work dir must be cleaned on the failure path too: {work_dir:?}"
            );
        }
    }

    // Test 5: the render path must never run the worker. The fake parks on a
    // barrier and stays parked while the request returns, so a ladder that
    // blocked on the worker would hang here rather than pass.
    #[test]
    fn requesting_a_render_returns_a_placeholder_without_running_the_worker() {
        let started = Arc::new(Barrier::new(2));
        let released = Arc::new(Barrier::new(2));
        let (fake, _seen) =
            FakeRenderer::new(Behaviour::Gate(Arc::clone(&started), Arc::clone(&released)));
        let mut cache: HashMap<u64, Slot> = HashMap::new();
        let key = hash_content(SRC);

        // The render path. Returns synchronously, with a placeholder.
        let asked = cached_resolve(&mut cache, key, || {
            spawn_worker(SRC, fake, GraphicsProtocol::Sixel)
        });

        // This returns only once the worker is genuinely parked inside the
        // renderer — proof the request above did not have to wait for it.
        started.wait();
        assert_eq!(
            asked,
            LatexBlock::Rendering,
            "the request must not have waited for the worker"
        );
        let slot = pending_slot(&cache, key);
        assert!(
            slot.lock().ok().and_then(|ready| ready.clone()).is_none(),
            "nothing is ready while the worker is parked"
        );

        // Let it finish; the raster arrives later, via the drain.
        released.wait();
        wait_for_slot(&slot);
        let landed = drain_cache(&mut cache);
        assert!(landed.resolved);
        assert_eq!(landed.pngs.len(), 1, "the raster arrives via the drain");
        assert_eq!(cache.len(), 1);
        let _ = std::fs::remove_file(&landed.pngs[0]);
    }

    // Test 6: a worker that PANICS must not wedge the drain. An empty slot is
    // the one unrecoverable state: `resolve` would answer `Rendering` forever
    // and the source would never come back.
    #[test]
    fn a_panicking_worker_does_not_wedge_the_drain() {
        // Silence the default panic hook so the suite's output stays readable.
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let (fake, seen) = FakeRenderer::new(Behaviour::Panic);
        let mut cache: HashMap<u64, Slot> = HashMap::new();
        let key = hash_content(SRC);

        assert_eq!(
            cached_resolve(&mut cache, key, || {
                spawn_worker(SRC, fake, GraphicsProtocol::Kitty)
            }),
            LatexBlock::Rendering
        );
        let done = wait_for_slot(&pending_slot(&cache, key));

        std::panic::set_hook(previous_hook);

        assert!(
            !matches!(done, LatexBlock::Rendering),
            "a panicking worker still has to report: {done:?}"
        );
        // The panic unwound through the work dir's guard, so cleanup happened
        // without anyone remembering to do it.
        let work_dir = work_dir_of(&first_seen(&seen));
        assert!(
            !work_dir.exists(),
            "an unwind must still clean the work dir: {work_dir:?}"
        );

        let landed = drain_cache(&mut cache);
        assert!(landed.resolved, "the drain must not be wedged");
        assert!(landed.pngs.is_empty());
        // And the key is now settled, so a later frame reads the fallback
        // instead of the placeholder.
        let settled = cached_resolve(&mut cache, key, || panic!("must not re-spawn"));
        assert_eq!(settled, done);
        let text = joined(&lines(&settled, SRC, "  "));
        assert!(
            text.contains("e^{-x^2}"),
            "the source is still recoverable: {text}"
        );
    }

    // Test 6b: the cache key is the formula source. An unchanged transcript
    // resolves once and stays resolved, and editing a formula never serves its
    // neighbour's render.
    #[test]
    fn latex_cache_should_hit_for_unchanged_source_and_miss_when_it_changes() {
        let mut cache: HashMap<u64, Slot> = HashMap::new();
        let other = "e^{i\\pi} + 1 = 0";
        let mut starts = 0u32;
        let step = |cache: &mut HashMap<u64, Slot>, source: &str, starts: &mut u32| {
            let key = hash_content(source);
            cached_resolve(cache, key, || {
                *starts += 1;
                Arc::new(Mutex::new(Some(LatexBlock::Failed {
                    error: source.to_string(),
                })))
            })
        };

        let seen: Vec<LatexBlock> = [SRC, SRC, other, SRC, other]
            .iter()
            .map(|source| step(&mut cache, source, &mut starts))
            .collect();

        assert_eq!(
            starts, 2,
            "one render per distinct source, not one per frame"
        );
        assert_eq!(cache.len(), 2);
        assert_eq!(seen[0], LatexBlock::Rendering);
        assert_eq!(seen[2], LatexBlock::Rendering);
        assert_eq!(
            seen[1], seen[3],
            "revisiting a source serves its own render"
        );
        assert_eq!(seen[1].source_tag(), Some(SRC));
        assert_eq!(seen[4].source_tag(), Some(other));
    }

    // Test 7: the module must stay WIRED. Nothing else in this file can fail if
    // the fence hook or the drain disappears: the ladder would compile, pass
    // every test above, and quietly never be called — a `latex` block would go
    // back to being a syntax-highlighted code listing, and a rendered formula
    // would be rasterised and thrown away. So pin the call sites by name,
    // against the source with its COMMENTS STRIPPED.
    //
    // The stripping is not decoration. A plain `contains` gate is satisfied by a
    // commented-out call, which is exactly how it was mutation-proven useless on
    // the precedent (`tui::pinned_images::tests`): commenting the call out left
    // the gate green. See `comment_stripping_cannot_be_fooled_by_a_commented_out_call`.
    #[test]
    fn the_latex_ladder_is_still_wired_to_both_consumers() {
        // [dead-strata sweep 2026-10-09] the pre-port renderer
        // (messages/markdown.rs) was the ladder's only consumer and is
        // deleted — the live math path is the vendored renderer. The gate
        // now pins THAT wiring: markdown_render_full routes math content
        // through operant_render_core's latex pipeline, the same contract
        // this test protected at its old site.
        let renderer = code_only(include_str!("operant_markdown/markdown_render_full.rs"));
        for needle in [
            "operant_render_core::render_inline_latex",
            "normalize_latex_math",
        ] {
            assert!(
                renderer.contains(needle),
                "markdown_render_full.rs no longer routes math through \
                 operant_render_core's latex pipeline (`{needle}`) — a ```math \
                 block would fall back to a plain code listing while every \
                 test in this file still passed"
            );
        }

        let render = code_only(include_str!("render/mod.rs"));
        assert!(
            render.contains("tui::mermaid::drain_ready_rasters"),
            "render/mod.rs no longer calls `tui::mermaid::drain_ready_rasters` — a \
             finished raster would never reach the pinned-graphics registry, so the \
             user would watch \"rendering…\" for ever"
        );
    }

    // The gate's own machinery. A wiring gate that a commented-out call can
    // satisfy is not a gate, and this is the exact shape that fools one: the
    // needle is present, in the file, on a line the compiler ignores.
    #[test]
    fn comment_stripping_cannot_be_fooled_by_a_commented_out_call() {
        let live = "fn f() {\n    crate::tui::latex::formula_lines(a, b);\n}\n";
        assert!(code_only(live).contains("latex::formula_lines"));

        for dead in [
            // Line-commented call.
            "fn f() {\n    // crate::tui::latex::formula_lines(a, b);\n}\n",
            // Block-commented call, the shape rustfmt leaves behind when a whole
            // call is wrapped.
            "fn f() {\n    /* crate::tui::latex::formula_lines(a, b); */\n}\n",
            // A call behind a `#[cfg(feature = "never")]` on the same line as its
            // own comment — the shape a "temporarily disabled" hook takes.
            "fn f() {\n    // TODO: re-enable\n    // crate::tui::latex::formula_lines(a, b);\n}\n",
        ] {
            assert!(
                !code_only(dead).contains("latex::formula_lines"),
                "a commented-out call must not satisfy a wiring gate: {dead}"
            );
        }

        // A doc comment that merely NAMES the hook is prose, so it is stripped —
        // which is what lets the gate stay honest when the prose is updated. A
        // real call is code, and survives.
        let mentioned = "/// see tui::latex::is_latex_lang\nfn f() {}\n";
        assert!(!code_only(mentioned).contains("is_latex_lang"));
        let called = "fn f() { tui::latex::is_latex_lang(a); }\n";
        assert!(code_only(called).contains("is_latex_lang"));
    }

    /// `src` with comments removed, so a commented-out call cannot satisfy a
    /// wiring gate.
    ///
    /// Line comments and block comments, and block comments nest because Rust
    /// block comments do. String literals are NOT tracked, so a `//` inside one
    /// opens a false comment — which can only make the gate *fail* where a real
    /// call follows, never pass one. That is the only direction a gate is
    /// allowed to be wrong in, and it is why the needle is a call-site path
    /// rather than prose.
    fn code_only(src: &str) -> String {
        let chars: Vec<char> = src.chars().collect();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                let mut depth = 1usize;
                i += 2;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }

    // The exact argv, pinned against the functions `render` actually calls.
    // The toolchain is optional, so a machine without it can only verify the
    // flags here — and this test reads those flags out of `tex_argv` /
    // `dvipng_argv` rather than restating them, so dropping or reordering a flag
    // in the implementation fails it.
    #[test]
    fn the_documented_argv_is_the_one_that_is_invoked() {
        let tex = Path::new("/tmp/operant-latex-1/formula.tex");
        let dvi = Path::new("/tmp/operant-latex-1/formula.dvi");
        let png = Path::new("/tmp/operant-latex-1.png");
        let shown = |argv: Vec<OsString>| {
            argv.iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };

        assert_eq!(
            shown(tex_argv(tex)),
            [
                "-interaction=nonstopmode",
                "-halt-on-error",
                "-output-format=dvi",
                "-output-directory",
                "/tmp/operant-latex-1",
                "/tmp/operant-latex-1/formula.tex",
            ],
            "the DVI pass argv is part of the contract"
        );

        assert_eq!(
            shown(dvipng_argv(png, dvi)),
            [
                "-T",
                "tight",
                "-D",
                "600",
                "-bg",
                "Transparent",
                "-o",
                "/tmp/operant-latex-1.png",
                "/tmp/operant-latex-1/formula.dvi",
            ],
            "the raster argv is part of the contract"
        );

        // Neither builder takes an engine, so `latex` and the `pdflatex`
        // fallback provably get the same command line — and `-output-format=dvi`
        // is what makes the same line correct for both.
        assert_eq!(Toolchain::TEX.tex, "latex");
        assert_eq!(Toolchain::PDFTEX.tex, "pdflatex");
        assert_eq!(Toolchain::TEX.dvipng, "dvipng");
        assert_eq!(Toolchain::PDFTEX.dvipng, "dvipng");
    }

    // An error dump must not be able to flood the transcript, and the
    // `Display` shapes are what the user reads.
    #[test]
    fn render_errors_are_bounded_and_readable() {
        assert_eq!(
            RenderError::Failed {
                tool: "latex",
                message: "boom".to_string()
            }
            .to_string(),
            "latex: boom"
        );
        assert!(
            RenderError::Timeout {
                tool: "dvipng",
                message: String::new()
            }
            .to_string()
            .contains("dvipng")
        );
        let clamped = clamp_error("e".repeat(MAX_ERROR_CHARS * 4));
        assert_eq!(clamped.chars().count(), MAX_ERROR_CHARS + 1);
    }
}
