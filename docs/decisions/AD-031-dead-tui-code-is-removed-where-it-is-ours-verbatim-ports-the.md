---
ID: "AD-031"
Title: "Dead TUI code is removed where it is ours; verbatim ports, the keybinding registry and vendored style modules stay allowlisted"
Status: "Active"
Date: "2026-10-19"
Description: "The clippy allowlist's dead-code entries drop from 52 to 47: unused TUI items with no caller are removed, and the verbatim jcode ports, the keybinding registry and the vendored style modules keep their allowance"
---

# AD-031: Dead TUI code is removed where it is ours; verbatim ports, the keybinding registry and vendored style modules stay allowlisted

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

`.ci/clippy-allowlist.txt` held 52 `dead_code` file entries. The BUGS row "Event and notification variants never built" and the entries behind it were all allowed by the gate, so an unused variant or helper could sit in the tree without a failing build. The gate (`scripts/clippy-warning-gate.sh`) is the authority for warnings.

148 TUI files carry the header `Ported verbatim @ 0a9dc7805`. Those are copies of upstream code, kept close to their source on purpose.

## Decision

1. Remove dead items that are ours and have no caller and no test use: unused enum variants with their match arms, unused fields with their initializers, and unused helper functions. Five files lost all their entries. The allowlist went from 52 to 47.
2. Keep, with the allowance:
   - `tui/vendor/**`: vendored upstream style code.
   - The keybinding registry (`KeyAction` variants, `KeyBindingRegistry::find_with_fallback`, `get_bindings`, `remove`). These are the vocabulary of the RT-3 dispatch migration.
   - Helpers that tests call. Removing them would break tests.
   - `Sink::as_str`, `Entry.name` and `Entry.sink`, which the paint-order guard test reads.
   - The verbatim ports under `operant_app/`, `operant_markdown/`, `operant_model/prepared.rs`, `operant_render/swarm_gallery.rs` and `operant_ui/`. Their dead items stay so each port remains a copy of its source.
   - Items documented as deliberate: the adapter wire types in `adapter_types/types.rs`, `TurnState::Connecting` (a planned event), `SystemAnnotation.style` (35 `push_system_message` callers), `App.bridge_state_tx` (dropping it closes `bridge_state_rx`), and `config_shim.rs`.
3. Do not gate test-only helpers with `#[cfg(test)]`. That would clear most of the remaining warnings, but it reads as suppression and hides the test-only surface from the build.

## Consequences

- 47 allowlisted entries remain. Any new dead code in a file that is not listed fails the gate.
- The verbatim-port classification is a judgement, based on the header. The owner should review it. If the ports should be treated as Operant code, the 47 entries become work, not allowance.
- The 148 verbatim ports are a large share of the TUI. Their legacy surface is the reason the dead-code count is high; this record does not change that.

## Orthogonality

AD-027 to AD-030 cover the review items in the same iteration. The keybinding registry stays for the RT-3 migration (`AD-024`, which keeps `[keybindings]` out of dispatch until then).

## Revisions

- 2026-10-19: created
