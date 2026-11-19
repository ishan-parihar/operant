#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Hydrogen finding F4 (iter-2686): the doctor must not tell an operator that
//! `[tts] enabled = false` disables TTS.
//!
//! It does not. `enabled` is read by the setup/doctor surfaces
//! (`cmd_doctor/checks_tools.rs`, `cmd_setup.rs`, `post_setup.rs`) and is
//! **not consulted by `TtsTool::execute`**, and the `tts` tool is registered
//! unconditionally in `builtin.rs`. AGENTS.md's design preference states this
//! as a rule: *"Do not 'fix' a disabled TTS by looking for a guard in the
//! dispatch path -- there is none to find."*
//!
//! So the old doctor line -- which told the operator to flip the flag in order
//! to use TTS -- was a false claim that sent operators hunting for a guard that
//! does not exist. This test pins the corrected wording in both directions: the
//! false promise must be gone, and the honest description must be present.

const CHECKS_TOOLS: &str = include_str!("../src/cmd_doctor/checks_tools.rs");

#[test]
fn the_doctor_does_not_claim_enabled_gates_tts() {
    // The needle is split so this test's own source cannot satisfy it.
    // (The doc comment above deliberately does NOT quote the old wording, or
    // this test would pass on its own prose.)
    let false_promise = format!("set [tts] enabled = {} to use it", "true");
    assert!(
        !CHECKS_TOOLS.contains(&false_promise),
        "the doctor must not claim `enabled = true` is what makes TTS usable -- \
         TtsTool::execute never reads the flag (AGENTS.md design preference: \
         there is no dispatch guard to find)"
    );
}

#[test]
fn the_doctor_explains_what_enabled_actually_does() {
    assert!(
        CHECKS_TOOLS.contains("marks the feature off in doctor/setup"),
        "the honest line must say the flag switches the operator-facing surface, \
         not the dispatch"
    );
    assert!(
        CHECKS_TOOLS.contains("blocked via tool policy"),
        "the real block is tool policy; the line must point there rather than at \
         a dispatch guard that does not exist"
    );
}

#[test]
fn the_tts_tool_is_still_registered_unconditionally() {
    // The other half of the claim: if `tts` ever became conditional, the
    // doctor's wording would be describing a different system. This is the
    // evidence the wording rests on, so it is pinned with it.
    let builtin = include_str!("../../operant-core/src/tools/builtin.rs");
    assert!(
        builtin.contains("\"tts\""),
        "the `tts` tool must stay registered -- the doctor's wording says it is \
         available regardless of the `enabled` flag, and that is only true while \
         registration is unconditional"
    );
}
