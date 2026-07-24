# Operant TUI Audit Report & Refactor Plan

> **Scope**: Presentation/TUI layer ONLY. Core agentic loop is out of scope.
> **Date**: 2026-07-23 (updated 2026-07-24 with deeper contrastive analysis and multi-crate migration plan)
> **Reference implementations**: claurst (fork derivative), jcode (advanced reference)

---

## 1. Executive Summary

Operant's TUI is a **feature-rich but monolithic** implementation built on ratatui 0.28.1. It has strong functional coverage (~55 submodules, 25+ dialogs) but suffers from architectural debt that limits testability, performance, and extensibility. Compared to claurst (a close fork with incremental polish) and jcode (a heavily modularized, performance-optimized reference), operant's TUI has three critical gaps:

1. **God-object state management** — A single `App` struct with ~180 fields makes the TUI untestable and fragile.
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
| Rust edition | 2024 | 2021 | **2024** |
| TUI crate count | 1 (monolith) | 1 (monolith) | **15+ crates** |
| Workspace separation | None for TUI | None for TUI | **Full crate-per-concern** |

### 2.2 Module Organization

| Aspect | operant | claurst | jcode |
|--------|---------|---------|-------|
| App state | Single `App` struct (~180 fields) | Single `App` struct (~180 fields) | **`TuiState` trait (114+ methods)** + App impl |
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
| Markdown rendering | Re-rendered every frame (basic thread-local cache for completed messages only) | Re-rendered every frame | **Cached per width** (`PreparedChatFrame` with content-hash invalidation) |
| Scroll | Manual offset tracking with VirtualList | Manual offset tracking with VirtualList | **Virtual list + tail-catchup + elastic overscroll** |
| Incremental rendering | Basic `transcript_version` invalidation (thread-local caches) | Basic `transcript_version` invalidation | **Full `PreparedChatFrame` with `PreparedMessages` + `WrappedLineMap`** |
| Image support | Basic `image_render.rs` (Kitty/Sixel/iTerm2 protocols) | Kitty graphics protocol (`kitty_image.rs`) | **`jcode-terminal-image` crate** + inline viewport with expand levels |
| Diagram support | None | None | **`jcode-tui-mermaid` crate** (full mermaid rendering) |
| Side panel | None | None | **Diff pane + diagram pane + side panel with markdown rendering cache** |
| Video export | None | None | **`video_export` module** (offline replay with SVG pipeline) |
| Hyperlinks | ✅ OSC8 support (`osc8.rs`) — post-paint buffer scan | ✅ OSC8 support (`osc8.rs`) | ✅ OSC8 + click detection |
| Copy selection | ✅ Drag-select + double/triple-click word/paragraph | Basic | **Full drag-select** with copy viewport |

### 2.4 Performance

| Aspect | operant | claurst | jcode |
|--------|---------|---------|-------|
| Redraw cadence | **Fixed 60fps** with basic `PerformanceTier::detect()` | Fixed 60fps | **Multi-tier**: 60fps anim / 20fps fast / 250ms idle / 5s deep idle |
| Performance tiers | ⚠️ Enum exists but is underutilized | None | **Full tier system** with adaptive cadence per state |
| Prepared frames | ⚠️ Basic thread-local caches (`MessageLinesCache`, `CompletedMsgCache`, `StreamingTextCache`) | None | **`PreparedChatFrame` with content-hash invalidation + full prep cache** |
| Markdown cache | ⚠️ Thread-local cache invalidated on transcript_version bump | None | **Width-keyed LRU cache** |
| Idle animation | None | None | **3D idle donut** (`jcode-tui-anim` crate, opt-level=3) |
| Focus-aware | No `FocusGained`/`FocusLost` handling | No | **Pauses animations when terminal unfocused** |
| Memory profiling | None | None | **`memory_profile` module** with JSON dumps |
| Frame metrics | Basic `debug_hub` with `FrameRendered` event | None | **`FramePerfStats` + `DrawCallAttribution` + `FlickerFrameSample` + `SlowFrameSample`** |
| Smoothness | None | None | **`AnchorStabilityRecorder`** + `AnchorFrame` for detecting jarring render jumps |

### 2.5 Features Matrix

| Feature | operant | claurst | jcode |
|---------|---------|---------|-------|
| Vim mode | ✅ | ✅ | ✅ |
| Slash commands | ✅ | ✅ | ✅ |
| Permission dialogs | ✅ | ✅ | ✅ |
| MCP view | ✅ | ✅ | N/A (different architecture) |
| Model picker | ✅ | ✅ | ✅ (inline picker) |
| Session browser | ✅ | ✅ | ✅ (session picker with crash banner) |
| Diff viewer | ✅ | ✅ | ✅ (file diff with syntax highlighting) |
| Theme picker | ✅ | ✅ | ✅ (with auto-detection) |
| Stats dialog | ✅ | ✅ | ✅ (usage widget) |
| Help overlay | ✅ | ✅ | ✅ |
| Global search | ✅ | ✅ | ✅ |
| Notifications | ✅ | ✅ | ✅ |
| Keyboard enhancement | ✅ | ✅ | ✅ |
| Mouse support | ✅ | ✅ | ✅ |
| Text selection | ✅ (drag-select + word/paragraph) | Basic | **Full drag-select** with copy viewport |
| OSC8 hyperlinks | ✅ | ✅ | ✅ |
| Kitty images | ❌ | ✅ | ✅ (`jcode-terminal-image`) |
| Mermaid diagrams | ❌ | ❌ | ✅ (`jcode-tui-mermaid`) |
| Side panel | ❌ | ❌ | ✅ (diff + diagram + markdown) |
| Video export | ❌ | ❌ | ✅ |
| Idle animation | ❌ | ❌ | ✅ (`jcode-tui-anim`) |
| Anchor stability | ❌ | ❌ | ✅ (`AnchorStabilityRecorder`) |
| Info widgets | ❌ | ❌ | ✅ (git, usage, todos, model, timeline, tips, swarm) |
| Stream buffer | ❌ | ❌ | ✅ (`StreamBuffer` in `jcode-tui-core`) |
| Copy selection mode | ❌ | ❌ | ✅ |

---

## 3. Operant Gap Analysis

### Critical Gaps (blocks quality/UX)

| # | Gap | Impact | Source |
|---|-----|--------|--------|
| C1 | **ratatui 0.28.1 → 0.30** | Missing scrollbar improvements, better widget APIs, performance | jcode uses 0.30 |
| C2 | **No prepared-frame caching** | Markdown re-rendered every frame, jank on long conversations | jcode's `PreparedChatFrame` |
| C3 | **God-object App struct** | Untestable, ~180 fields, every change risks side effects | jcode's `TuiState` trait |
| C4 | **No redraw cadence optimization** | Wastes CPU on idle terminals, battery drain on laptops | jcode's `PerformanceTier` + multi-tier cadence |
| C5 | **Terminal setup lacks panic hook** | Panics leave terminal in raw mode | claurst's `setup_terminal()` pattern |
| C6 | **Monolithic crate structure** | No compile-time isolation, no incremental builds, all-or-nothing compilation | jcode's 15-crate TUI workspace |

### Important Gaps (quality-of-life)

| # | Gap | Impact | Source |
|---|-----|--------|--------|
| I1 | **No Kitty image protocol** | No inline image rendering | claurst's `kitty_image.rs` |
| I2 | **No focus-aware rendering** | Backgrounded tabs burn CPU | jcode's `client_focused()` |
| I3 | **No elastic overscroll** | Abrupt scroll bounds | jcode's `chat_overscroll_active` |
| I4 | **No theme mode detection** | Can't auto-detect light/dark terminal | jcode's `terminal-colorsaurus` |
| I5 | **Basic markdown rendering** | No table alignment, limited syntax | claurst's `markdown_enhanced.rs` |
| I6 | **render.rs monolith** (~2,675 LOC) | Hard to navigate, test, and extend | Should be split into focused sub-modules |
| I7 | **adapter_types.rs monolith** (~2,521 LOC) | Secondary god-module mixing auth, models, voice, git, config | Should be split into focused sub-modules |

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

#### 1.1 Upgrade ratatui to 0.30

```toml
# Cargo.toml
ratatui = "0.30"  # was 0.28.1
```

**Files**: `Cargo.toml`, all files using removed/changed APIs
**Risk**: Low — 0.28→0.30 is mostly additive
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

#### 1.3 Decompose render.rs monolith

Split `render.rs` (~2,675 LOC) into focused sub-modules:

| New Module | Responsibility | Source Functions |
|------------|---------------|------------------|
| `render/mod.rs` | Top-level `render_app()` orchestrator | `render_app()` |
| `render/messages.rs` | Transcript rendering, message items, live content | `render_messages()`, `render_message_items()`, `append_live_content()`, `append_turn_items()` |
| `render/overlays.rs` | Modal dialogs, error modals, context menu | `render_error_modal()`, `render_context_menu()` |
| `render/footer.rs` | Status bar, footer, tips | `render_status_row()`, `render_footer()` |
| `render/welcome.rs` | Welcome box, banner, startup notices | `render_welcome_box()`, `render_banner_block()`, `render_startup_notices()` |
| `render/selection.rs` | Text selection highlight, row cache | `apply_selection_highlight()`, `cache_selectable_row_text()` |
| `render/input.rs` | Prompt input rendering | `render_input()`, `render_prompt_suggestions()` |

**Files**: Split `tui/render.rs` into `tui/render/` directory
**Risk**: Low — mechanical extraction, no logic changes
**Test**: All existing tests pass, visual smoke test

#### 1.4 Decompose adapter_types.rs monolith

Split `adapter_types.rs` (~2,521 LOC) into focused sub-modules:

| New Module | Responsibility | Source |
|------------|---------------|--------|
| `adapter_types/mod.rs` | Re-exports, `TuiApp`, `LaunchMode` | Current top-level |
| `adapter_types/config.rs` | `Settings`, `Theme`, `PermissionMode`, `InnerConfig` | `config` module |
| `adapter_types/auth.rs` | `AuthStore`, `StoredCredential`, `ProviderId` | `AuthStore` impl |
| `adapter_types/models.rs` | `ModelRegistry`, `RegistryModelEntry`, `ModelInfo`, `AnthropicClient` | `ModelRegistry` impl |
| `adapter_types/voice.rs` | `VoiceRecorder`, `VoiceEvent` | `voice` module |
| `adapter_types/history.rs` | `SessionRecord`, `list_sessions()`, `load_session()` | `history` module |
| `adapter_types/git_utils.rs` | `get_current_branch()`, `get_repo_root()` | `git_utils` module |
| `adapter_types/types.rs` | `Message`, `Role`, `ContentBlock`, `MessageContent` | `types` module |
| `adapter_types/cost.rs` | `CostTracker` | `cost` module |
| `adapter_types/import_config.rs` | `ImportSelection`, `ImportPreview`, `ImportResult` | `import_config` module |
| `adapter_types/tips.rs` | `select_tip()` | `tips` module |
| `adapter_types/spinner.rs` | Spinner verbs | `spinner` module |

**Files**: Split `tui/adapter_types.rs` into `tui/adapter_types/` directory
**Risk**: Low — mechanical extraction
**Test**: All existing tests pass

#### 1.5 Clean up dead code (20 items from DEAD_CODE_AUDIT.md)

**Files**: Various
**Risk**: Low
**Test**: `cargo check --workspace && cargo test --workspace`

### Phase 2: Rendering Pipeline (Week 3-4) — Performance

**Goal**: Eliminate per-frame markdown re-rendering and add redraw cadence optimization.

#### 2.1 Add prepared-frame caching for messages

The biggest performance win. Port jcode's concept of content-addressed caching:

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

**Files**: Create `tui/messages/cache.rs`, update `tui/messages/mod.rs`, integrate into `render/messages.rs`
**Risk**: Medium — must invalidate correctly on resize and new messages
**Test**: Benchmark frame times before/after with 100+ message conversation

#### 2.2 Add redraw cadence optimization

Wire the existing `PerformanceTier` enum into the main loop:

```rust
// Enhance existing tui/redraw.rs
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

**Files**: Update `tui/redraw.rs`, update `tui/app.rs` main loop
**Risk**: Medium — must not miss events while throttled
**Test**: Measure CPU usage on idle terminal before/after

#### 2.3 Add focus-aware rendering

```rust
// In app state
pub client_focused: bool,

// In main event loop:
Event::FocusLost => { app.client_focused = false; }
Event::FocusGained => { app.client_focused = true; }
```

**Files**: Update `tui/app.rs` to handle `Event::FocusGained`/`Event::FocusLost`
**Risk**: Low
**Test**: Background the terminal tab — CPU should drop to near-zero

### Phase 3: State Architecture (Week 5-8) — Testability

**Goal**: Break the god-object App struct into composable, testable pieces.

#### 3.1 Define TuiState trait (jcode pattern)

Operant already has a `TuiState` trait in `tui/state.rs`. Expand it to match jcode's 114+ method coverage:

```rust
// Expand existing tui/state.rs
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

    // ... expand to ~80+ methods for full renderer coverage
}
```

**Files**: Expand `tui/state.rs`, update render functions to accept `&dyn TuiState`
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

Port jcode's `jcode-tui-mermaid` concept. Consider making it a separate crate if pursued.

**Files**: Create `tui/mermaid/` module or separate crate
**Risk**: High — complex rendering pipeline
**Test**: Mermaid code blocks should render as ASCII diagrams

### Phase 5: Polish & Product Features (Ongoing)

#### 5.1 Add elastic overscroll
#### 5.2 Add theme mode auto-detection
#### 5.3 Enhance markdown table rendering
#### 5.4 Replace hand-rolled markdown parser with pulldown-cmark

`messages/markdown.rs` is a hand-rolled markdown parser (~300 LOC). Replace with `pulldown-cmark` for parsing + a custom `Renderer` that produces `Vec<Line<'static>>`. This delivers immediate rendering quality improvements (correct CommonMark, tables, task lists).

---

## 5. Migration Path

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
| ratatui 0.28→0.30 API breakage | Pin to 0.30, fix compilation errors incrementally |
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
| `tui/render/mod.rs` | Top-level render orchestrator (split from render.rs) | 1 |
| `tui/render/messages.rs` | Transcript rendering | 1 |
| `tui/render/overlays.rs` | Modal dialogs, error modals | 1 |
| `tui/render/footer.rs` | Status bar, footer | 1 |
| `tui/render/welcome.rs` | Welcome box, banner | 1 |
| `tui/render/selection.rs` | Text selection highlight | 1 |
| `tui/render/input.rs` | Prompt input rendering | 1 |
| `tui/adapter_types/config.rs` | Settings, themes (split from adapter_types.rs) | 1 |
| `tui/adapter_types/auth.rs` | Auth store, credentials | 1 |
| `tui/adapter_types/models.rs` | Model registry, providers | 1 |
| `tui/adapter_types/voice.rs` | Voice recorder bridge | 1 |
| `tui/adapter_types/history.rs` | Session history | 1 |
| `tui/adapter_types/types.rs` | TUI message types | 1 |
| `tui/messages/cache.rs` | Prepared-frame message cache | 2 |
| `tui/kitty_image.rs` | Kitty graphics protocol | 4 |

### Modified Files

| File | Changes | Phase |
|------|---------|-------|
| `Cargo.toml` | ratatui 0.28→0.30 | 1 |
| `tui/mod.rs` | Add new module declarations | 1,2,3 |
| `tui/app.rs` | Extract sub-structs, implement TuiState | 3 |
| `tui/state.rs` | Expand TuiState trait to ~80+ methods | 3 |
| `tui/render.rs` | Split into render/ directory | 1 |
| `tui/adapter_types.rs` | Split into adapter_types/ directory | 1 |
| `tui/theme_colors.rs` | Add Deuteranopia palette | 1 |
| `tui/app.rs` (main loop) | Add redraw cadence, focus handling | 2 |

---

## 8. Supplementary Contrastive Findings (2026-07-24 deep analysis)

### 8.1 Corrected Caching Assessment

The original audit states "No prepared-frame caching" for operant. This is imprecise. Operant has **three thread-local caches** in `render.rs`:

- `MessageLinesCache` — full-result cache keyed on `(transcript_version, messages_ptr, messages_len, ...)` — invalidates when any message changes or transcript_version bumps
- `CompletedMsgCache` — caches rendered lines for committed (non-streaming) messages
- `StreamingTextCache` — memoizes markdown render of live streaming text (avoids re-running syntect every frame)

These are functional but limited: they are **frame-level caches** (same input → same output), not **content-addressed caches** (same content → same output regardless of frame). The gap is that first-render of any message always pays the full markdown parse cost, and on resize, ALL cached entries are invalidated rather than only those that changed width.

### 8.2 Corrected PerformanceTier Assessment

Operant already has `PerformanceTier::Minimal/Normal/High` in `tui/redraw.rs` with `detect()` logic. However, the main event loop in `TuiApp::run()` does **not use it for cadence adaptation** — it redraws at a fixed interval regardless of tier. The tier is computed once at startup but has no runtime effect on redraw frequency.

### 8.3 New Gap: `render.rs` Monolith (~2,675 LOC)

`render.rs` is itself a monolith that needs decomposition. It contains:
- `render_app()` — the top-level layout orchestrator
- `render_messages()` — transcript rendering with VirtualList
- `render_status_row()` — status bar
- `render_input()` — prompt input
- `render_footer()` — footer with tips and stats
- `render_welcome_box()` — two-column welcome screen
- `render_banner_block()` — ASCII art banner
- `render_error_modal()` — error dialog
- `render_context_menu()` — right-click menu
- `apply_selection_highlight()` — text selection post-pass
- `cache_selectable_row_text()` — row text cache for double-click
- `append_live_content()` — streaming content append
- `render_message_items()` — full message item builder
- Plus 15+ helper functions

This should be split into `render/messages.rs`, `render/overlays.rs`, `render/footer.rs`, `render/welcome.rs`, `render/selection.rs` etc. This is a prerequisite for Phase 3 (TuiState trait) since the trait boundary will be cleaner with smaller modules.

### 8.4 New Gap: `adapter_types.rs` Complexity (~2,521 LOC)

`adapter_types.rs` is a secondary god-module containing:
- `Settings` struct (config persistence)
- `AuthStore` (credential management)
- `ModelRegistry` (provider model catalog)
- `AnthropicClient` (API client)
- `VoiceRecorder` (voice recording bridge)
- `CostTracker` (token/cost tracking)
- `import_config` module (config import/export)
- `git_utils` (branch detection)
- `tips` (rotating tips)
- `history` (session history from DB)
- `types` (TUI message types)
- `constants`, `output_styles`, `spinner`, `codex_oauth`

Each of these should be its own module file. This refactor should happen before Phase 2 to reduce the blast radius of the TuiState trait change.

### 8.5 New Gap: Custom Markdown Parser

`messages/markdown.rs` is a hand-rolled markdown parser (~300 LOC) that handles basic formatting (bold, italic, code, headers, lists, links). This is a candidate for replacement with a proper library (pulldown-cmark → ratatui spans) rather than incremental improvement.

**Recommendation**: Replace with `pulldown-cmark` for parsing + a custom `Renderer` that produces `Vec<Line<'static>>`. The hand-rolled parser works but misses edge cases (nested emphasis, complex lists, tables, task lists). A library replacement should be a standalone PR in Phase 1 — it's independent of the architectural refactor and delivers immediate rendering quality improvements.

### 8.6 New Gap: No Testing Strategy

The original audit mentions tests but doesn't specify a testing strategy for the refactor. Key testing needs:

- **Phase 1 (Performance)**: Before/after frame time benchmarks with 100+ messages
- **Phase 2 (Architecture)**: Mock `TuiState` implementations for unit-testing individual renderers
- **Phase 3 (State)**: Integration tests that verify App state transitions
- **Existing test gaps**: Many dialogs have `render_does_not_panic()` smoke tests but no behavioral assertions

### 8.7 New Gap: No Backward Compatibility Plan

If settings.json format changes or config fields move between modules, users need migration. The existing `Settings::load_sync()` doesn't handle schema versioning. Consider adding a `version` field to settings.json.

### 8.8 Clarified Feature Parity

The original audit marks several features as ❌ for operant that are actually present:
- **OSC8 hyperlinks**: ✅ `tui/osc8.rs` exists and is integrated into the post-paint pass in `render_app()`
- **Copy selection**: ✅ `apply_selection_highlight()` + `cache_selectable_row_text()` provide drag-select with word/paragraph detection via `last_row_text` cache
- **Text selection**: ✅ Full drag-select with `selection_anchor`/`selection_focus` state and `selection_text` extraction
- **Performance tiers**: ⚠️ Enum exists but is underutilized (see 8.2)
- **Prepared-frame cache**: ⚠️ Basic thread-local caches exist (see 8.1)

### 8.9 What to Keep (Cross-Referenced with Refactor Plan)

| Strength | Location | Preserved by Phase | Enhanced by Phase |
|----------|----------|--------------------|--------------------|
| VirtualList abstraction | `tui/virtual_list.rs` | All (stable API) | Phase 2 (TuiState-aware) |
| TranscriptTurn grouping | `tui/transcript_turn.rs` | All | Phase 2 (mock-friendly) |
| Dialog priority routing | `DialogPriority` enum | All | Phase 2 (stateless) |
| Context menu | `render.rs::render_context_menu` | All | Phase 2 (extracted to render/selection.rs) |
| Text selection | `render.rs::apply_selection_highlight` | All | Phase 2 (extracted) |
| OSC8 hyperlinks | `tui/osc8.rs` | All | Phase 1 (already complete) |
| StreamingTextCache | `render.rs` | All | Phase 1 (enhanced with content-hash) |
| CompletedMsgCache | `render.rs` | All | Phase 1 (enhanced with width-key) |
| Adapter types layer | `tui/adapter_types.rs` | All | Phase 0 (split into modules) |
| Shimmer animation | `render.rs::shimmer_spans` | All | Phase 2 (extracted) |

---

## 9. Conclusion

Operant's TUI has excellent feature coverage but needs architectural investment to match jcode's quality bar. The most impactful changes are:

1. **TuiState trait** (Phase 3) — Enables testability and renderer decoupling. This is the single most impactful architectural change.
2. **Prepared-frame caching** (Phase 2) — Eliminates the biggest performance bottleneck. The existing thread-local caches are a good foundation but need content-hash invalidation.
3. **Panic hook + terminal setup** (Phase 1) — Prevents terminal corruption on crash.
4. **Redraw cadence** (Phase 2) — Reduces CPU/battery usage by 5-10x on idle. The PerformanceTier enum already exists but needs to be wired into the main loop.
5. **render.rs decomposition** (Phase 1) — Prerequisite for Phase 3. Split into focused sub-modules.
6. **adapter_types.rs decomposition** (Phase 1) — Prerequisite for Phase 3. Split into focused sub-modules.

These six changes alone would bring operant's TUI from "functional but unoptimized" to "production-grade and performant."

**Prerequisites for Phase 3**: Before starting the TuiState trait refactor, decompose `render.rs` (~2,675 LOC monolith) into `render/messages.rs`, `render/overlays.rs`, `render/footer.rs`, `render/welcome.rs`, `render/selection.rs`, etc. Similarly decompose `adapter_types.rs` (~2,521 LOC) into `config.rs`, `auth.rs`, `models.rs`, `voice.rs`, `history.rs`, `git_utils.rs`, etc. These decompositions are mechanical and should be done as standalone PRs before the Phase 3 TuiState work.

**The core agentic loop is already well-architected (per the prior audit). This plan focuses exclusively on the presentation layer.**

---

## 10. Multi-Crate Architecture Migration (2026-07-24)

> **The single highest-leverage structural change**: migrating operant's monolithic TUI module (~44,300 LOC across 44 files in one crate) to a multi-crate workspace modeled on jcode's 15-crate TUI architecture.

### 10.1 Why Multi-Crate Matters

jcode's TUI is split into 15 dedicated crates with a strict dependency DAG. This provides:

| Benefit | Monolith (operant today) | Multi-Crate (jcode model) |
|---------|-------------------------|---------------------------|
| **Incremental compilation** | Changing `theme_colors.rs` recompiles all 44K LOC | Only recompiles `operant-tui-style` (~500 LOC) |
| **Compile-time isolation** | Any module can import any other — no enforced boundaries | Crates can only import their declared dependencies |
| **Testability** | Must construct full `App` to test any renderer | Each crate has its own test binary with minimal setup |
| **Parallel compilation** | Single crate = single compilation unit | Multiple crates compile in parallel on all cores |
| **Dependency hygiene** | All deps shared in one Cargo.toml | Each crate declares only what it needs (smaller dep tree) |
| **API surface control** | Everything is `pub(crate)` or `pub` — no middle ground | `pub` exports are the crate boundary — explicit API |

### 10.2 jcode's TUI Crate Dependency Graph

jcode's 15 TUI crates form a clean DAG:

```
Layer 0 (Leaf crates — no internal deps):
  jcode-tui-core          ── anchor_stability, copy_selection, graph_topology, keybind, stream_buffer
  jcode-tui-style          ── color capability, theme mode, terminal adaptation
  jcode-tui-anim           ── idle animation math (opt-level=3)
  jcode-tui-workspace      ── workspace/project metadata
  jcode-tui-tool-display   ── tool call rendering
  jcode-tui-usage-overlay  ── usage/cost display
  jcode-tui-visual-debug   ── debug capture, frame metrics
  jcode-tui-session-picker ── session list/selection
  jcode-tui-account-picker ── account/provider selection

Layer 1 (Depends on Layer 0):
  jcode-tui-render         ── layout utils, chrome, rounded boxes   [depends on: style]
  jcode-tui-permissions    ── permission dialogs                     [depends on: style]
  jcode-tui-mermaid        ── mermaid diagram rendering              [depends on: workspace]

Layer 2 (Depends on Layer 1):
  jcode-tui-markdown       ── markdown → ratatui spans               [depends on: mermaid (opt), workspace]
  jcode-tui-messages       ── message rendering, prepared frames     [depends on: markdown]

Layer 3 (Top-level orchestrator):
  jcode-tui                ── App struct, event loop, module glue     [depends on: anim, + re-exports app-core]
```

### 10.3 Operant's Proposed Multi-Crate Layout

Mapping operant's 44 TUI modules into 15 dedicated crates (matching jcode's granularity) plus a thin orchestrator:

```
operant/
├── Cargo.toml                  # Workspace root (add TUI crates as members)
├── crates/
│   ├── operant-core/           # (existing — agentic loop, tools, memory)
│   │   └── src/                # ADD: provider.rs ← from tui/provider.rs (PROVIDERS, ProviderDef)
│   ├── operant-cli/            # (existing — thin binary, TUI logic moves out)
│   │
│   ├── operant-tui-core/       # Layer 0 — foundational traits & abstractions
│   │   └── src/
│   │       ├── lib.rs          # re-exports
│   │       ├── keybind.rs      # ← from tui/keybindings.rs (KeyAction enum + process_key_action)
│   │       ├── state.rs        # ← from tui/state.rs (TuiState trait definition)
│   │       └── selection.rs    # ← from tui/render.rs (CopySelection types)
│   │
│   ├── operant-tui-style/      # Layer 0 — themes & terminal adaptation
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── palette.rs      # ← from tui/theme_colors.rs (ColorPalette, all 5 themes + Deuteranopia)
│   │       ├── capability.rs   # NEW: terminal color capability detection
│   │       └── mode.rs         # NEW: light/dark auto-detection (port terminal-colorsaurus)
│   │
│   ├── operant-tui-anim/       # Layer 0 — idle animation math
│   │   └── src/
│   │       ├── lib.rs
│   │       └── rustle.rs       # ← from tui/rustle.rs (ASCII art math, Rustle mascot)
│   │
│   ├── operant-tui-render/     # Layer 1 — rendering primitives
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── layout.rs       # NEW: rect_contains, point_in_rect, parse_area_spec
│   │       ├── chrome.rs       # ← from tui/overlays.rs (render_dark_overlay, render_dialog_bg, render_modal_title_frame)
│   │       ├── virtual_list.rs # ← from tui/virtual_list.rs (VirtualList, VirtualItem trait)
│   │       ├── truncation.rs   # ← from tui/render.rs (truncate_end, truncate_middle, truncate_text)
│   │       └── banner.rs       # ← from tui/banner.rs (banner_with_subtitle)
│   │
│   ├── operant-tui-markdown/   # Layer 2 — markdown → ratatui spans
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── parse.rs        # NEW: pulldown-cmark parser (replaces hand-rolled markdown.rs)
│   │       ├── render.rs       # ← from tui/messages/markdown.rs (render_markdown)
│   │       ├── enhanced.rs     # ← from tui/messages/markdown_enhanced.rs (render_table)
│   │       └── code_block.rs   # syntax-highlighted code block rendering (uses syntect)
│   │
│   ├── operant-tui-messages/   # Layer 2 — message rendering pipeline
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── transcript.rs   # ← from tui/transcript_turn.rs (TranscriptTurn, build_transcript_turns)
│   │       ├── render.rs       # ← from tui/messages/mod.rs (render_transcript_*, render_assistant_text)
│   │       ├── cache.rs        # content-addressed message cache (MessageCache with content-hash)
│   │       └── prepared.rs     # NEW: PreparedChatFrame (port from jcode-tui-messages)
│   │
│   ├── operant-tui-permissions/ # Layer 1 — permission & approval dialogs
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── permission.rs   # ← from tui/dialogs.rs (render_permission_dialog, PermissionRequest)
│   │       ├── mcp_approval.rs # ← from tui/dialogs.rs (McpApprovalDialogState)
│   │       └── bypass.rs       # ← from tui/bypass_permissions_dialog.rs
│   │
│   ├── operant-tui-input/      # Layer 0 — prompt input & vim
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── prompt_input.rs # ← from tui/prompt_input.rs (InputMode, VimMode, render_prompt_input)
│   │       ├── vim.rs          # vim mode logic extracted from prompt_input.rs
│   │       ├── history.rs      # ← from tui/input_history.rs
│   │       └── suggestions.rs  # ← from tui/prompt_input.rs (TypeaheadSource, slash suggestions)
│   │
│   ├── operant-tui-dialogs/    # Layer 0 — all non-permission dialog overlays
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── ask_user.rs     # ← from tui/ask_user_dialog.rs
│   │       ├── custom_provider.rs # ← from tui/custom_provider_dialog.rs
│   │       ├── device_auth.rs  # ← from tui/device_auth_dialog.rs
│   │       ├── dialog_select.rs # ← from tui/dialog_select.rs (DialogSelectState, SelectItem)
│   │       ├── elicitation.rs  # ← from tui/elicitation_dialog.rs
│   │       ├── effort_picker.rs # ← from tui/effort_picker.rs
│   │       ├── export.rs       # ← from tui/export_dialog.rs
│   │       ├── file_injection.rs # ← from tui/file_injection_dialog.rs
│   │       ├── free_mode.rs    # ← from tui/free_mode_dialog.rs
│   │       ├── hooks_config.rs # ← from tui/hooks_config_menu.rs
│   │       ├── import_config.rs # ← from tui/import_config_dialog.rs
│   │       ├── invalid_config.rs # ← from tui/invalid_config_dialog.rs
│   │       ├── key_input.rs    # ← from tui/key_input_dialog.rs
│   │       ├── memory_file_selector.rs # ← from tui/memory_file_selector.rs
│   │       ├── model_picker.rs # ← from tui/model_picker.rs (ModelPickerState, render_model_picker)
│   │       ├── onboarding.rs   # ← from tui/onboarding_dialog.rs
│   │       ├── settings.rs     # ← from tui/settings_screen.rs
│   │       ├── theme.rs        # ← from tui/theme_screen.rs
│   │       └── stats.rs        # ← from tui/stats_dialog.rs
│   │
│   ├── operant-tui-views/      # Layer 0 — full-screen view overlays
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── agents.rs       # ← from tui/agents_view.rs (AgentsMenuState, render_agents_menu)
│   │       ├── journey.rs      # ← from tui/journey_view.rs
│   │       ├── mcp.rs          # ← from tui/mcp_view.rs (McpViewState, render_mcp_view)
│   │       ├── skills.rs       # ← from tui/skills_view.rs
│   │       ├── plugins.rs      # ← from tui/plugins_hub.rs
│   │       ├── context_viz.rs  # ← from tui/context_viz.rs
│   │       ├── diff_viewer.rs  # ← from tui/diff_viewer.rs (DiffViewerState, render_diff_dialog)
│   │       ├── tasks.rs        # ← from tui/tasks_overlay.rs (TasksOverlay)
│   │       └── session_branching.rs # ← from tui/session_branching.rs
│   │
│   ├── operant-tui-tool-display/ # Layer 0 — tool call rendering
│   │   └── src/
│   │       ├── lib.rs
│   │       └── tool_block.rs   # ← from tui/render.rs (render_tool_block_lines, ToolUseBlock)
│   │
│   ├── operant-tui-visual-debug/ # Layer 0 — debug diagnostics
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── hub.rs          # ← from tui/debug/debug_hub.rs (TuiDebugHub)
│   │       ├── overlay.rs      # ← from tui/debug/overlay.rs (render_debug_overlay)
│   │       ├── event_bus.rs    # ← from tui/debug/event_bus.rs (TuiEvent bus)
│   │       └── metrics.rs      # NEW: FramePerfStats, DrawCallAttribution (port from jcode)
│   │
│   ├── operant-tui-session/    # Layer 0 — session management UI
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── browser.rs      # ← from tui/session_browser.rs
│   │       └── picker.rs       # NEW: session picker (port from jcode-tui-session-picker)
│   │
│   ├── operant-tui-voice/      # Layer 0 — voice capture UI
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── capture.rs      # ← from tui/voice_capture.rs
│   │       └── notice.rs       # ← from tui/voice_mode_notice.rs
│   │
│   ├── operant-tui-images/     # Layer 0 — image rendering
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── render.rs       # ← from tui/image_render.rs (Kitty/Sixel/iTerm2 protocols)
│   │       ├── paste.rs        # ← from tui/image_paste.rs
│   │       └── kitty.rs        # ← from tui/osc8.rs (OSC8 hyperlink overlay)
│   │
│   ├── operant-tui-notifications/ # Layer 0 — notification banners
│   │   └── src/
│   │       ├── lib.rs
│   │       └── banner.rs       # ← from tui/notifications.rs (NotificationQueue, render_notification_banner)
│   │
│   └── operant-tui/            # Layer 3 — orchestrator (thin glue crate)
│       └── src/
│           ├── lib.rs
│           ├── app.rs          # ← from tui/app.rs (App struct refactored with sub-structs)
│           ├── state_impl.rs   # ← TuiState impl for App (delegates to sub-structs)
│           ├── terminal.rs     # ← from tui/terminal.rs (setup/teardown with panic hook)
│           ├── redraw.rs       # ← from tui/redraw.rs (PerformanceTier + adaptive cadence)
│           ├── bridge_state.rs # ← from tui/bridge_state.rs
│           ├── figures.rs      # ← from tui/figures.rs
│           ├── slash_usage.rs  # ← from tui/slash_usage.rs
│           ├── render/
│           │   ├── mod.rs      # render_app() orchestrator
│           │   ├── messages.rs # render_messages, append_live_content, render_message_items
│           │   ├── footer.rs   # render_footer, render_status_row
│           │   ├── welcome.rs  # render_welcome_box, render_banner_block
│           │   ├── selection.rs # apply_selection_highlight, cache_selectable_row_text
│           │   └── input_render.rs # render_input, render_prompt_suggestions
│           └── adapter_types/  # ← from tui/adapter_types.rs (split into sub-modules)
│               ├── mod.rs
│               ├── config.rs   # Settings, Theme, PermissionMode, InnerConfig
│               ├── auth.rs     # AuthStore, StoredCredential
│               ├── models.rs   # ModelRegistry, AnthropicClient
│               ├── voice.rs    # VoiceRecorder, VoiceEvent
│               ├── history.rs  # SessionRecord, list_sessions(), load_session()
│               ├── git_utils.rs # get_current_branch(), get_repo_root()
│               └── types.rs    # Message, Role, ContentBlock, MessageContent
```

**Module coverage**: All 44 of operant's TUI modules are mapped. No orphaned modules.

**Note on `operant-tui-core`**: The `anchor.rs` and `stream.rs` modules from jcode-tui-core are **deferred** to a later PR (after the migration is complete). They are jcode-specific rendering smoothness concepts that operant doesn't need yet. The foundational crate starts with just `keybind.rs`, `state.rs`, and `selection.rs` — the minimum needed for the other crates to compile.

### 10.4 Dependency Graph for operant-tui crates

```
Layer 0 (Leaf crates — no internal TUI deps, ~15 crates):
  operant-tui-core          ── keybind, state trait, selection types
  operant-tui-style         ── palette, color capability, theme mode
  operant-tui-anim          ── rustle ASCII math
  operant-tui-input         ── prompt input, vim, history, suggestions
  operant-tui-dialogs       ── all non-permission dialog overlays (~16 dialogs)
  operant-tui-views         ── full-screen views (agents, journey, mcp, skills, etc.)
  operant-tui-tool-display  ── tool block rendering
  operant-tui-visual-debug  ── debug hub, event bus, frame metrics
  operant-tui-session       ── session browser
  operant-tui-voice         ── voice capture, notice
  operant-tui-images        ── image render, paste, OSC8 hyperlinks
  operant-tui-notifications ── notification queue, banner rendering

Layer 1 (Depends on Layer 0):
  operant-tui-render        ── layout, chrome, virtual_list, truncation, banner  [→ style]
  operant-tui-permissions   ── permission dialogs, MCP approval                   [→ style]

Layer 2 (Depends on Layer 1):
  operant-tui-markdown      ── markdown parser, render, enhanced, code_block    [→ style]
  operant-tui-messages      ── transcript, message render, cache, prepared       [→ markdown, core]

Layer 3 (Top-level — depends on everything):
  operant-tui               ── App, event loop, state impl, render orchestration
                              [→ core, style, anim, input, render, messages, permissions,
                               dialogs, views, tool_display, visual_debug, session,
                               voice, images, notifications]
```

### 10.5 Dependency Upgrades

| Dependency | Current | Target | Migration Notes |
|-----------|---------|--------|-----------------|
| Rust edition | 2024 | **2024** (already there) | No change needed |
| Rust version | 1.85 | **1.85+** (already there) | No change needed |
| ratatui | 0.28.1 | **0.30** | Biggest change. `Stylize` trait moved to prelude. Scrollbar API updated. Some widget method renames. |
| crossterm | 0.29 | **0.29** (keep) | Compatible with ratatui 0.30 |
| pulldown-cmark | N/A | **0.12+** | NEW: Replace hand-rolled markdown parser |
| syntect | indirect | **5.x** | Verify version used for syntax highlighting in markdown code blocks |
| unicode-width | used | **0.2+** | Verify compatibility |
| dirs | used | **6.x** | Verify version for XDG paths |
| arboard | N/A | **3.x** | NEW: Cross-platform clipboard (replaces shell-command clipboard in app.rs) |
| async-trait | used | **0.1** (keep) | Compatible |

### 10.6 Phased Multi-Crate Migration

This migration should be executed in sub-phases within the existing Phase 1 (Quick Wins).

#### Sub-Phase 1A: Create Leaf Crates (Week 1-2)

Create the Layer 0 leaf crates with extracted code:

1. **`operant-tui-core`** — Extract `keybind.rs`, move `state.rs` trait definition, selection types. This is the foundational crate that all others depend on.
2. **`operant-tui-style`** — Extract `theme_colors.rs`, add color capability detection.
3. **`operant-tui-anim`** — Extract `rustle.rs` (ASCII art math only).
4. **`operant-tui-input`** — Extract `prompt_input.rs` (5,305 LOC), `input_history.rs`. Split vim logic into `vim.rs`.
5. **`operant-tui-dialogs`** — Extract all 16 non-permission dialog files (model_picker, settings_screen, theme_screen, onboarding, etc.).
6. **`operant-tui-views`** — Extract agents_view, journey_view, mcp_view, skills_view, plugins_hub, context_viz, diff_viewer, tasks_overlay, session_branching.
7. **`operant-tui-tool-display`** — Extract tool block rendering from render.rs.
8. **`operant-tui-visual-debug`** — Extract debug hub, overlay, event_bus.
9. **`operant-tui-session`** — Extract session_browser.rs.
10. **`operant-tui-voice`** — Extract voice_capture.rs, voice_mode_notice.rs.
11. **`operant-tui-images`** — Extract image_render.rs, image_paste.rs, osc8.rs.
12. **`operant-tui-notifications`** — Extract notifications.rs.

**Validation**: `cargo check --workspace` passes. `operant-cli` temporarily depends on all leaf crates.

#### Sub-Phase 1B: Create Layer 1 Crates (Week 2-3)

13. **`operant-tui-render`** — Extract `virtual_list.rs`, truncation helpers from render.rs, chrome from overlays.rs, banner.rs. Create layout.rs utils.
14. **`operant-tui-permissions`** — Extract permission dialog rendering from dialogs.rs.

**Validation**: `cargo check --workspace` passes. Tests pass.

#### Sub-Phase 1C: Create Layer 2 Crates (Week 3)

15. **`operant-tui-markdown`** — Extract markdown rendering from messages/markdown.rs. Replace hand-rolled parser with pulldown-cmark.
16. **`operant-tui-messages`** — Extract transcript rendering from transcript_turn.rs, message render from messages/mod.rs, message cache. Add PreparedChatFrame.

**Validation**: `cargo check --workspace` passes. Markdown rendering tests pass.

#### Sub-Phase 1D: Create operant-tui Orchestrator (Week 3-4)

17. Create `operant-tui` crate as the top-level orchestrator.
18. Move `app.rs` (8,648 LOC) into `app.rs` with sub-module extraction (event loop, slash commands, provider logic).
19. Split `adapter_types.rs` (2,521 LOC) into `adapter_types/` sub-modules.
20. Move `terminal.rs`, `redraw.rs`, `bridge_state.rs`, `figures.rs`, `slash_usage.rs`.
21. Create `render/` directory with `mod.rs`, `messages.rs`, `footer.rs`, `welcome.rs`, `selection.rs`, `input_render.rs`.

**Validation**: `cargo check --workspace` passes.

#### Sub-Phase 1E: Update Workspace & Clean Up (Week 4)

22. Move `provider.rs` from `operant-cli` to `operant-core` (it defines PROVIDERS, ProviderDef, infer_provider_from_model).
23. Update workspace `Cargo.toml` to include all 15 new TUI crates as members.
24. Update `operant-cli/Cargo.toml` to depend only on `operant-tui` (thin binary).
25. Remove old `tui/` directory from `operant-cli/src/`.
26. Run full validation: `cargo fmt --all && cargo check --workspace && cargo test --workspace && cargo clippy --workspace --all-targets --all-features -- -D warnings`.
27. Visual smoke test of the TUI.

**Validation**: Full workspace build passes. TUI smoke test passes.

### 10.7 Before vs. After

| Metric | Before (Monolith) | After (Multi-Crate) |
|--------|-------------------|---------------------|
| TUI crates | 1 | **15** |
| Largest file | `app.rs` (8,648 LOC) | `app.rs` (~3,000 LOC + sub-modules) |
| Second largest | `prompt_input.rs` (5,305 LOC) | Moved to `operant-tui-input` (~2,000 LOC + vim.rs) |
| Third largest | `render.rs` (2,675 LOC) | Split into `render/` (~500 LOC per sub-module) |
| `adapter_types.rs` | 2,521 LOC monolith | Split into 8 modules (~300 LOC each) |
| `overlays.rs` | 2,251 LOC | Split into `render/chrome.rs` + `operant-tui-permissions` |
| Cold build time | Full 44K LOC recompile | Only changed crate recompiles |
| Incremental build | ~30s (entire TUI) | ~3-5s (single crate) |
| Test isolation | Full App construction | Mock TuiState per renderer |
| Module coverage | 1 monolith | 100% — all 44 modules mapped to 15 crates |
| Dependency hygiene | All deps shared | Each crate declares only what it needs |

---

## 11. Conclusion (Updated)

Operant's TUI has excellent feature coverage but needs both **architectural** and **structural** investment to match jcode's quality bar. The two highest-leverage changes are:

1. **Multi-crate workspace migration** (Section 10) — Enables compile-time isolation, incremental builds, and testability. This is the structural prerequisite for all other improvements.
2. **TuiState trait expansion** (Phase 3) — Enables renderer-level testing and decoupling. This becomes trivially easy after the multi-crate migration.

The recommended execution order is:

```
Multi-Crate Migration (Section 10, Sub-Phases 1A-1E)
  ↓ (structural foundation)
Phase 1: Quick Wins (panic hook, ratatui upgrade, dead code cleanup)
  ↓
Phase 2: Rendering Pipeline (prepared-frame cache, redraw cadence, focus-aware)
  ↓
Phase 3: State Architecture (TuiState expansion, App sub-structs)
  ↓
Phase 4-5: Visual Richness & Polish
```

**The core agentic loop is already well-architected (per the prior audit). This plan focuses exclusively on the presentation layer.**
