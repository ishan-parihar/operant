# CLI Parity Audit: @operant-agent (Py) vs @operant-rs (Rust)

## 1. Overview
This report identifies the parity gap between the original Python implementation of the Hermes CLI and the Rust port. While the Rust CLI provides a nearly identical command surface, several critical subsystems are currently implemented as "delegated stubs" that call back into Python.

**Overall Parity Status:** 🟡 Partial (Surface Complete, Implementation Gaps)

---

## 2. Parity Matrix

| Command Group | Py Surface | Rust Surface | Rust Implementation | Status | Gap/Note |
|---------------|:----------:|:------------:|:--------------------:|:------:|:-----------------------------------------------------------------------|
| **Core/Chat**  | ✅         | ✅           | Native                | 🟢     | Full parity on `chat`, `run`, `autonomous`. |
| **Config**     | ✅         | ✅           | Native                | 🟢     | Full parity on `config`, `model`, `fallback`. |
| **Sessions**   | ✅         | ✅           | Native                | 🟢     | Full parity on session management. |
| **Skills**     | ✅         | ✅           | Native (mostly)       | 🟡     | Most lifecycle commands native; some discovery logic differs. |
| **Kanban**     | ✅         | ✅           | Native (Partial)      | 🟡     | **Missing `boards` management** (Multi-board support). |
| **MCP**        | ✅         | ✅           | Mixed                 | 🟡     | `add/list/test` native; `serve` is a Python stub. |
| **Gateway**    | ✅         | ✅           | Mixed                 | 🔴     | Management native; **Runtime is Python-delegated**. |
| **Curator**    | ✅         | ✅           | Stub                  | 🔴     | **100% Python-delegated**. |
| **Auth/Profile**| ✅         | ✅           | Native                | 🟢     | Full parity. |
| **System/Logs**| ✅         | ✅           | Native                | 🟢     | Full parity on `status`, `doctor`, `logs`, `dump`. |
| **Backup/Imp**  | ✅         | ✅           | Native                | 🟢     | Full parity. |
| **Plugins**    | ✅         | ✅           | Mixed                 | 🟡     | `list/enable` native; `install` is a stub. |
| **Specialized** | ✅         | ✅           | Stub                  | 🔴     | `claw`, `slack manifest`, `dashboard` are Python stubs. |
| **RL Tooling**  | ✅         | ❌           | Missing              | 🔴     | `rl_cli.py` functionality not yet ported to a separate tool. |

---

## 3. Detailed Implementation Gaps

### 🔴 High Priority: The "Delegation Debt"
The following commands are currently "liars"—they appear in `operant --help` but are actually wrappers for `python3 -m operant_agent ...`:
- `operant curator <subcommand>`
- `operant gateway start/stop/restart`
- `operant acp server`
- `operant dashboard server`
- `operant claw migrate/cleanup`
- `operant mcp serve`

### 🟡 Medium Priority: Feature Depth
- **Kanban Boards**: Python allows `operant kanban boards create <name>` and `switch <name>`. Rust assumes a single global board.
- **Interactive Registry**: Python's `COMMAND_REGISTRY` defines 50+ slash commands. Rust's interactive mode needs to implement the same registry to ensure the user experience is identical.

---

## 4. Upgrade Plan (Roadmap)

### Phase 1: The "Independence" Sprint
**Goal**: Eliminate the most critical Python dependencies.
- [ ] Implement `curator` logic natively in Rust.
- [ ] Implement `plugins install` logic (fetching and extracting skill/plugin archives).
- [ ] Port the `claw` migration scripts to Rust.

### Phase 2: Infrastructure & Runtime
**Goal**: Move from "Management" to "Execution".
- [ ] Implement the `gateway` runtime engine in Rust (Socket handling, platform adapters).
- [ ] Port the `acp` server implementation.
- [ ] Implement a basic `dashboard` (or a native TUI alternative).

### Phase 3: Feature Depth & Polish
**Goal**: Achieve 1:1 functional parity.
- [ ] Add `kanban boards` (Multi-board support) to the Rust Kanban store.
- [ ] Implement the full `COMMAND_REGISTRY` for in-chat slash commands.
- [ ] Port `rl_cli` as a separate binary or a dedicated `operant rl` subcommand suite.

### Phase 4: Verification
- [ ] Run `operant doctor` and `operant status` to verify all components are native.
- [ ] Perform end-to-end validation of the `curator` $\rightarrow$ `skills` $\rightarrow$ `agent` pipeline without Python.
