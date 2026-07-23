# Operant TUI Audit Report & Refactor Plan

> **Scope**: Presentation/TUI layer ONLY. Core agentic loop is out of scope.
> **Date**: 2026-07-23
> **Reference implementations**: claurst (fork derivative), jcode (advanced reference)

---

## 1. Executive Summary

Operant's TUI is a **feature-rich but monolithic** implementation built on ratatui 0.28.1. It has strong functional coverage (~55 submodules, 25+ dialogs) but suffers from architectural debt that limits testability, performance, and extensibility. Compared to claurst (a close fork with incremental polish) and jcode (a heavily modularized, performance-optimized reference), operant's TUI has three critical gaps:

1. **God-object state management** — A single `App` struct with 200+ fields makes the TUI untestable and fragile.
2. **No rendering pipeline optimization** — Every frame re-renders everything; no prepared-frame caching, no width-parameterized markdown, no incremental rendering.
3. **No performance tier system** — Animations and redraw cadence are hardcoded; no adaptation for SSH, WSL, or minimal terminals.

The refactor plan below is ordered by impact/effort ratio and can be executed incrementally without breaking the existing UI.

---

## 2. Architecture Comparison

### 2.1 Stack & Versions

| Aspect | operant | claurst | jcode |
|--------|---------|---------|-------|
| ratatui | **0.28.1** | 0.29 | **0.30** |
| crossterm | 0.29 | 0.29 | 0.29 |
| Rust edition | 2021 | 2021 | **2024** |
| TUI crate count | 1 (monolith) | 1 (monolith) | **15+ crates** |
| Workspace separation | None for TUI | None for TUI | **Full crate-per-concern** |

### 2.2 Module Organization

| Aspect | operant | claurst | jcode |
|--------|---------|---------|-------|
| App state | Single `App` struct (200+ fields) | Single `App` struct (~180 fields) | **`TuiState` trait (114+ methods)** + App impl |
| Render entry | `render_app()` in render.rs | `render_app()` in render.rs | **Prepared-frame pipeline** (cached) |
| Keybindings | Centralized in `keybindings.rs` | Centralized in `keybindings.rs` | **Trait-based + keybind crate** |
| Dialogs | ~25 individual dialog files | ~25 individual dialog files | **Inline pickers + overlay system** |
| Messages | `messages/mod.rs` + `markdown.rs` + `markdown_enhanced.rs` | Same structure | **Dedicated `jcode-tui-markdown` crate** + `jcode-render-core` |
| Themes | `theme_colors.rs` (5 themes) | `theme_colors.rs` (5 themes + Deuteranopia) | **`jcode-tui-style` crate** + `terminal-colorsaurus` detection |
| Terminal setup | In `TuiApp::run()` | In `setup_terminal()` with panic hook | **Separate `terminal.rs`** with TuiRuntimeGuard |
| Testing | Unit tests scattered in modules | Unit tests + snapshot tests | **TestBackend + prepared-frame tests + bench binary** |

### 2.3 Rendering Pipeline

| Aspect | operant | claurst | jcode |
|--------|---------|---------|-------|
| Markdown rendering | Re-rendered every frame | Re-rendered every frame | **Cached per width** (`get_cached_message_lines`) |
| Scroll | Manual offset tracking | Manual offset tracking | **Virtual list + tail-catchup + elastic overscroll** |
| Incremental rendering | None | None | **`transcript_version` invalidation** |
| Image support | Basic `image_render.rs` | Kitty graphics protocol (`kitty_image.rs`) | **`jcode-terminal-image` crate** + inline viewport |
| Diagram support | None | None | **`jcode-tui-mermaid` crate** (full mermaid rendering) |
| Side panel | None | None | **Diff pane + diagram pane + side panel** |
| Video export | None | None | **`video_export` module** (offline replay) |
| Hyperlinks | None | OSC8 support (`osc8.rs`) | OSC8 + click detection |
| Copy selection | None | None | **Full drag-select + word/paragraph detection** |

### 2.4 Performance

| Aspect | operant | claurst | jcode |
|--------|---------|---------|-------|
| Redraw cadence | Fixed 60fps | Fixed 60fps | **Multi-tier**: 60fps anim / 20fps fast / 250ms idle / 5s deep idle |
| Performance tiers | None | None | **Minimal / Normal / High** |
| Prepared frames | None | None | **`PreparedChatFrame` with content-hash invalidation** |
| Markdown cache | None | None | **Width-keyed LRU cache** |
| Idle animation | None | None | **3D idle donut** (jcode-tui-anim, opt-level=3) |
| Focus-aware | No | No | **Pauses animations when terminal unfocused** |
| Memory profiling | None | None | **`memory_profile` module** with JSON dumps |

### 2.5 Features Present

| Feature | operant | claurst | jcode |
|---------|---------|---------|-------|
| Vim mode | ✅ | ✅ | ✅ |
| Slash commands | ✅ | ✅ | ✅ |
| Permission dialogs | ✅ | ✅ | ✅ |
| MCP view | ✅ | ✅ | ✅ |
| Model picker | ✅ | ✅ | ✅ |
| Session browser | ✅ | ✅ | ✅ |
| Diff viewer | ✅ | ✅ | ✅ |
| Theme picker | ✅ | ✅ | ✅ |
| Stats dialog | ✅ | ✅ | ✅ |
| Help overlay | ✅ | ✅ | ✅ |
| Global search | ✅ | ✅ | ✅ |
| Notifications | ✅ | ✅ | ✅ |
| Keyboard enhancement | ✅ | ✅ | ✅ |
| Mouse support | ✅ | ✅ | ✅ |
| Text selection | Basic | Basic | **Full drag-select** |
| OSC8 hyperlinks | ❌ | ✅ | ✅ |
| Kitty images | ❌ | ✅ | ✅ |
| Desktop upsell | ❌ | ✅ | N/A |
| Overage upsell | ❌ | ✅ | ✅ |
| Feedback survey | ❌ | ✅ | N/A |
| Paste viewer | ❌ | ✅ | N/A |
| Plugin views | ✅ | ✅ | N/A |
| Mermaid diagrams | ❌ | ❌ | ✅ |
| Side panel | ❌ | ❌ | ✅ |
| Video export | ❌ | ❌ | ✅ |
| Onboarding flow | Basic | Basic | **Full guided onboarding** |
| Copy selection mode | ❌ | ❌ | ✅ |
| Idle animation | ❌ | ❌ | ✅ |
| Performance tiers | ❌ | ❌ | ✅ |
| Prepared-frame cache | ❌ | ❌ | ✅ |
| TuiState trait | ❌ | ❌ | ✅ |
| Stream buffer | ❌ | ❌ | ✅ |
| Anchor stability | ❌ | ❌ | ✅ |
| Info widgets | ❌ | ❌ | ✅ |
| Swarm gallery | ❌ | ❌ | ✅ |
| Workspace map | ❌ | ❌ | ✅ |

---

## 3. Operant Gap Analysis

### Critical Gaps (blocks quality/UX)

| # | Gap | Impact | Source |
|---|-----|--------|--------|
| C1 | **ratatui 0.28.1 → 0.29+** | Missing scrollbar improvements, better widget APIs | claurst uses 0.29, jcode uses 0.30 |
| C2 | **No prepared-frame caching** | Markdown re-rendered every frame, jank on long conversations | jcode's `PreparedChatFrame` |
| C3 | **God-object App struct** | Untestable, 200+ fields, every change risks side effects | jcode's `TuiState` trait |
| C4 | **No redraw cadence optimization** | Wastes CPU on idle terminals, battery drain on laptops | jcode's `PerformanceTier` + multi-tier cadence |
| C5 | **Terminal setup lacks panic hook** | Panics leave terminal in raw mode | claurst's `setup_terminal()` pattern |

### Important Gaps (quality-of-life)

| # | Gap | Impact | Source |
|---|-----|--------|--------|
| I1 | **No OSC8 hyperlink support** | URLs in transcript not clickable | claurst's `osc8.rs` |
| I2 | **No Kitty image protocol** | No inline image rendering | claurst's `kitty_image.rs` |
| I3 | **No copy selection mode** | Can't select/copy text from transcript | jcode's `copy_selection` |
| I4 | **No focus-aware rendering** | Backgrounded tabs burn CPU | jcode's `client_focused()` |
| I5 | **No elastic overscroll** | Abrupt scroll bounds | jcode's `chat_overscroll_active` |
| I6 | **No theme mode detection** | Can't auto-detect light/dark terminal | jcode's `terminal-colorsaurus` |
| I7 | **Basic markdown rendering** | No table alignment, limited syntax | claurst's `markdown_enhanced.rs` |

### Nice-to-Have Gaps (long-term)

| # | Gap | Impact | Source |
|---|-----|--------|--------|
| N1 | Mermaid diagram rendering | Visual diagrams in transcript | jcode's `jcode-tui-mermaid` |
| N2 | Side panel architecture | Diff/diagram alongside transcript | jcode's side panel system |
| N3 | Video export | Offline session replay | jcode's `video_export` |
| N4 | 3D idle animation | Polish / brand identity | jcode's `jcode-tui-anim` |
| N5 | Full guided onboarding | Better first-run experience | jcode's `OnboardingWelcomeKind` |
| N6 | Stream buffer | Replay streaming operations | jcode's `StreamBuffer` |
| N7 | Anchor stability tracking | Detect jarring render jumps | jcode's `AnchorStabilityRecorder` |
| N8 | Info widgets | Floating ambient information | jcode's `info_widget` |
| N9 | Swarm gallery | Multi-agent visualization | jcode's `swarm_gallery` |

---

## 4. Priority Refactor Plan

### Phase 1: Foundation (Week 1-2) — Quick Wins

**Goal**: Low-effort, high-impact improvements that don't require architectural changes.

#### 1.1 Upgrade ratatui to 0.29

```toml
# Cargo.toml
ratatui = "0.29"  # was 0.28.1
```

**Files**: `Cargo.toml`, all files using removed/changed APIs
**Risk**: Low — 0.28→0.29 is mostly additive
**Test**: `cargo check --workspace && cargo test --workspace`

#### 1.2 Add panic hook to terminal setup

Port claurst's `setup_terminal()` pattern:

```rust
// In tui/mod.rs or a new tui/terminal.rs
pub fn setup_terminal(mouse_capture: bool) -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    let main_thread_id = std::thread::current().id();
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        if std::thread::current().id() == main_thread_id {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
            let _ = execute!(io::stdout(), crossterm::cursor::Show);
        }
        original_hook(panic_info);
    }));
    // ... rest of setup
}
```

**Files**: Create `tui/terminal.rs`, update `tui/mod.rs`, update `app.rs`
**Risk**: Low
**Test**: Manual — panic during TUI should restore terminal

#### 1.3 Add OSC8 hyperlink support

Port `osc8.rs` from claurst (nearly identical codebase).

**Files**: Create `tui/osc8.rs`, integrate into `render.rs` post-paint step
**Risk**: Low — additive post-paint pass
**Test**: URLs in transcript should be Ctrl+clickable in supported terminals

#### 1.4 Upgrade theme_colors with Deuteranopia support

Port claurst's enhanced `theme_colors.rs` with accessibility-friendly color palettes.

**Files**: `tui/theme_colors.rs`
**Risk**: Low
**Test**: Visual — toggle deuteranopia theme

### Phase 2: Rendering Pipeline (Week 3-4) — Performance

**Goal**: Eliminate per-frame markdown re-rendering and add redraw cadence optimization.

#### 2.1 Add prepared-frame caching for messages

The biggest performance win. Port jcode's concept of caching rendered `Vec<Line>` per message per width.

```rust
// New module: tui/messages/cache.rs
pub struct MessageCache {
    /// (content_hash, width) -> rendered lines
    cache: HashMap<(u64, u16), Vec<Line<'static>>>,
    /// Version counter — bumped when messages change
    version: u64,
}

impl MessageCache {
    pub fn get_or_render(&mut self, text: &str, width: u16) -> &[Line<'static>] {
        let hash = hash_content(text);
        let key = (hash, width);
        if !self.cache.contains_key(&key) {
            let lines = render_markdown(text, width);
            self.cache.insert(key, lines);
        }
        self.cache.get(&key).unwrap()
    }

    pub fn invalidate(&mut self) {
        self.cache.clear();
    }
}
```

**Files**: Create `tui/messages/cache.rs`, update `tui/messages/mod.rs`, integrate into `render.rs`
**Risk**: Medium — must invalidate correctly on resize and new messages
**Test**: Benchmark frame times before/after with 100+ message conversation

#### 2.2 Add redraw cadence optimization

Port jcode's multi-tier redraw system:

```rust
// New module: tui/redraw.rs
pub enum PerformanceTier {
    Minimal,  // SSH, WSL — no animations, 20fps max
    Normal,   // Default — animations at 30fps
    High,     // Local terminal — animations at 60fps
}

pub fn redraw_interval(tier: PerformanceTier, is_streaming: bool, is_idle: bool) -> Duration {
    match (tier, is_streaming, is_idle) {
        (_, true, false) => Duration::from_millis(16),   // 60fps while streaming
        (PerformanceTier::High, _, true) => Duration::from_millis(250),  // idle
        (PerformanceTier::Normal, _, true) => Duration::from_millis(500),
        (PerformanceTier::Minimal, _, true) => Duration::from_secs(2),
        (PerformanceTier::High, _, _) => Duration::from_millis(33),  // 30fps
        (PerformanceTier::Normal, _, _) => Duration::from_millis(50),
        (PerformanceTier::Minimal, _, _) => Duration::from_millis(100),
    }
}
```

**Files**: Create `tui/redraw.rs`, update `tui/app.rs` main loop
**Risk**: Medium — must not miss events while throttled
**Test**: Measure CPU usage on idle terminal before/after

#### 2.3 Add focus-aware rendering

```rust
// In app state
pub fn on_focus_lost(&mut self) {
    self.client_focused = false;
    // Stop animations, slow down redraw
}

pub fn on_focus_gained(&mut self) {
    self.client_focused = true;
    // Resume animations
}
```

**Files**: Update `tui/app.rs` to handle `Event::FocusGained`/`Event::FocusLost`
**Risk**: Low
**Test**: Background the terminal tab — CPU should drop to near-zero

### Phase 3: State Architecture (Week 5-8) — Testability

**Goal**: Break the god-object App struct into composable, testable pieces.

#### 3.1 Define TuiState trait (jcode pattern)

This is the most impactful architectural change. Instead of the renderer taking `&App`, it takes `&dyn TuiState`:

```rust
// New module: tui/state.rs
pub trait TuiState {
    // Transcript
    fn messages(&self) -> &[Message];
    fn streaming_text(&self) -> &str;
    fn is_streaming(&self) -> bool;

    // Input
    fn input_text(&self) -> &str;
    fn cursor_pos(&self) -> usize;

    // Scroll
    fn scroll_offset(&self) -> usize;
    fn auto_scroll(&self) -> bool;

    // Provider
    fn model_name(&self) -> &str;
    fn active_provider(&self) -> Option<&str>;

    // Status
    fn status_message(&self) -> Option<&str>;
    fn frame_count(&self) -> u64;

    // Overlay state
    fn any_modal_open(&self) -> bool;
    fn help_visible(&self) -> bool;

    // ... more methods as needed
}

// App implements TuiState
impl TuiState for App {
    fn messages(&self) -> &[Message] { &self.messages }
    // ...
}
```

**Files**: Create `tui/state.rs`, update `tui/mod.rs`, update render functions to accept `&dyn TuiState`
**Risk**: High — touches every render function signature
**Test**: Can now test renderers with mock state

#### 3.2 Extract state groups into sub-structs

Break `App` into focused sub-structs:

```rust
pub struct TranscriptState {
    pub messages: Vec<Message>,
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub streaming_text: String,
    pub streaming_thinking: String,
}

pub struct InputState {
    pub prompt_input: PromptInputState,
    pub cursor_pos: usize,
    pub input_history: Vec<String>,
}

pub struct OverlayState {
    pub help: HelpOverlay,
    pub global_search: GlobalSearchState,
    pub history_search: HistorySearchOverlay,
    pub model_picker: ModelPickerState,
    pub session_browser: SessionBrowserState,
    // ... other overlays
}

pub struct ProviderState {
    pub model_name: String,
    pub active_provider: Option<String>,
    pub effort_level: EffortLevel,
    pub fast_mode: bool,
    pub cost_tracker: Arc<CostTracker>,
}

pub struct App {
    pub transcript: TranscriptState,
    pub input: InputState,
    pub overlays: OverlayState,
    pub provider: ProviderState,
    // ... remaining fields
}
```

**Files**: Create `tui/state/` module directory, extract from `tui/app.rs`
**Risk**: Medium — mechanical refactor, many call site updates
**Test**: All existing tests should pass after migration

### Phase 4: Image & Diagram Support (Week 9-12) — Visual Richness

**Goal**: Add inline image rendering and diagram support.

#### 4.1 Add Kitty graphics protocol support

Port claurst's `kitty_image.rs` for inline image rendering.

**Files**: Create `tui/kitty_image.rs` (port from claurst)
**Risk**: Medium — terminal compatibility varies
**Test**: Paste an image → should render inline in supported terminals

#### 4.2 Add mermaid diagram rendering (optional, long-term)

This is a larger effort — port jcode's `jcode-tui-mermaid` concept. Consider making it a separate crate if pursued.

**Files**: Create `tui/mermaid/` module or separate crate
**Risk**: High — complex rendering pipeline
**Test**: Mermaid code blocks should render as ASCII diagrams

### Phase 5: Polish & Product Features (Ongoing)

#### 5.1 Add elastic overscroll

```rust
// When scroll_offset reaches max and user scrolls further:
// Show a brief "overscroll" indicator, then spring back
```

#### 5.2 Add theme mode auto-detection

Port jcode's `terminal-colorsaurus` integration for automatic light/dark detection.

#### 5.3 Enhance markdown table rendering

Port claurst's `markdown_enhanced.rs` table detection and box-drawing rendering.

---

## 5. Migration Path

The refactor should be executed in this order to minimize breakage:

```
Phase 1 (Quick Wins)     ← No architectural changes, pure additions
  ↓
Phase 2 (Performance)    ← New modules, minimal changes to existing
  ↓
Phase 3 (Architecture)   ← Breaking changes to App struct, systematic
  ↓
Phase 4 (Visual)         ← New features, independent of Phase 3
  ↓
Phase 5 (Polish)         ← Incremental improvements
```

Each phase should be a separate PR/branch with:
- `cargo check --workspace` passing
- `cargo test --workspace` passing
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passing
- Visual smoke test of the TUI

---

## 6. Risks and Dependencies

| Risk | Mitigation |
|------|------------|
| ratatui 0.28→0.29 API breakage | Pin to 0.29, fix compilation errors incrementally |
| TuiState trait refactor touches 50+ render functions | Do it module-by-module, keep old signatures as wrappers temporarily |
| Prepared-frame cache invalidation bugs | Hash-based invalidation + integration tests |
| Performance tier misconfiguration | Default to Normal tier, let user override via config |
| Kitty image protocol not supported everywhere | Graceful fallback to text placeholder |
| claurst/operant drift | Keep adapter_types layer as the divergence boundary |

---

## 7. Files to Create/Modify (Summary)

### New Files
| File | Purpose | Phase |
|------|---------|-------|
| `tui/terminal.rs` | Terminal setup/teardown with panic hook | 1 |
| `tui/osc8.rs` | OSC8 hyperlink overlay | 1 |
| `tui/redraw.rs` | Redraw cadence + performance tiers | 2 |
| `tui/state.rs` | TuiState trait definition | 3 |
| `tui/messages/cache.rs` | Prepared-frame message cache | 2 |
| `tui/kitty_image.rs` | Kitty graphics protocol | 4 |

### Modified Files
| File | Changes | Phase |
|------|---------|-------|
| `Cargo.toml` | ratatui 0.28→0.29 | 1 |
| `tui/mod.rs` | Add new module declarations | 1,2,3 |
| `tui/app.rs` | Extract sub-structs, implement TuiState | 3 |
| `tui/render.rs` | Accept `&dyn TuiState`, use cached messages | 2,3 |
| `tui/theme_colors.rs` | Add Deuteranopia palette | 1 |
| `tui/app.rs` (main loop) | Add redraw cadence, focus handling | 2 |

---

## 8. Conclusion

Operant's TUI has excellent feature coverage but needs architectural investment to match jcode's quality bar. The most impactful changes are:

1. **Prepared-frame caching** (Phase 2) — Eliminates the biggest performance bottleneck
2. **TuiState trait** (Phase 3) — Enables testability and renderer decoupling
3. **Panic hook + terminal setup** (Phase 1) — Prevents terminal corruption on crash
4. **Redraw cadence** (Phase 2) — Reduces CPU/battery usage by 5-10x on idle

These four changes alone would bring operant's TUI from "functional but unoptimized" to "production-grade and performant."
