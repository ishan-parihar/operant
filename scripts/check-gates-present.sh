#!/usr/bin/env bash
# Guard the gate scripts against a rebase silently DROPPING their checks.
#
# The failure this exists for, proven twice on 2026-11-17:
#   - iter-2551 "restore the iter-2539 verifier fix a rebase dropped AGAIN":
#     scripts/verify-library.sh reverted to its pre-iter-2539 state, so a
#     hand-written tool allowlist naming 4 phantom tools came back and the
#     derived-registry loop that replaced it was gone.
#   - iter-2549 "restore the library tool-count gate iter-2548's rebase
#     dropped": the tool-count block vanished from
#     scripts/check-doc-claims.sh, and a wrong count (106 vs 105) then sat
#     undetected in docs.
#
# Root cause (iter-2551's body): "rebasing onto a peer's reorganized tree
# silently drops same-region edits."
#
# A gate that loses a check still exits 0, so the loss is invisible. This
# script makes deletion LOUD: for each gate script it asserts a set of
# REQUIRED MARKERS -- the semantic core of each check -- is still present, and
# fails the battery when one is missing. It lives in its own file so a rebase
# cannot drop the guard and the thing it protects in one motion.
#
# Marker choice matters. A marker that is incidental wording gets a gate
# failed by a harmless reword, and a gate that cries wolf gets ignored. So
# each marker below is the invariant core of a check -- a line that MUST
# survive any rewording -- chosen to hold across BOTH regex variants the
# check-doc-claims.sh tool-count block has carried (origin/main's tolerant
# `types? implement` / `names? in ... builtin_tool_names()` and the narrower
# parenthesised form). All markers were verified present in origin/main's
# scripts at iter-2557, and all but two deliberately general ones are
# byte-exact.
#
# Two history-loss cases are asserted explicitly:
#   - the phantom allowlist must not come back into verify-library.sh, as a
#     REGISTRATION arm (a `tool_shape` alternative). The explanatory comment
#     that names the four phantoms is legitimate and stays.
#   - the library tool-count derivation must still read tools/builtin.rs and
#     call builtin_tool_names(), rather than being hardcoded to a number.
#
# POSIX sh + grep. Run from the repo root:  bash scripts/check-gates-present.sh
# Exit 0 when every marker is present, 1 when any is missing.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

fail=0
say() { printf '%s\n' "$*"; }

# require <file> <fixed-string> <what-it-protects> <iteration>
# A missing marker is a failure: the check it names was dropped.
require() {
  _f=$1; _needle=$2; _what=$3; _iter=$4
  if [ ! -f "$_f" ]; then
    say "MISSING-GATE $_f: file does not exist (marker from $_iter)"
    fail=1
    return
  fi
  if ! grep -qF -- "$_needle" "$_f"; then
    say "MISSING-MARKER $_f: missing [$_needle] -- protects: $_what (introduced $_iter)"
    fail=1
  fi
}

DC=scripts/check-doc-claims.sh
VL=scripts/verify-library.sh

# =========================================================================
# scripts/check-doc-claims.sh -- the docs-check gate.
# Markers are the semantic core of each check, not incidental comments.
# =========================================================================

# The shared claim comparator: prints "DRIFT <name>: doc says X, code has Y"
# and sets fail=1. Every count check in this script funnels through it.
require "$DC" 'check() { # name actual expected' \
  'the doc-vs-code count comparator' iter-2549
require "$DC" 'DRIFT $1: doc says $3, code has $2' \
  'the comparator printing a fix-list line' iter-2549

# CLI command count, derived from the Commands enum in main.rs.
require "$DC" "src.index('enum Commands {')" \
  'the CLI command-variant count (enum Commands)' iter-2549
require "$DC" 'check "CLI command variants"' \
  'the CLI command-variant count check' iter-2549
# The CLI LIST, both directions (ghosts and missing), as its own python block.
require "$DC" 'DRIFT CLI list:' \
  'the CLI list set-difference (ghosts/missing)' iter-2549

# Gateway platform adapters, counted as entries in fn platform_registry().
require "$DC" "src.index('fn platform_registry()')" \
  'the gateway platform-adapter count' iter-2549
require "$DC" 'check "gateway platforms"' \
  'the gateway platform-adapter check' iter-2549

# Error classes, derived from pub enum FailoverReason.
require "$DC" "src.index('pub enum FailoverReason {')" \
  'the error-class count (FailoverReason)' iter-2549
require "$DC" 'check "error classes"' \
  'the error-class count check' iter-2549
# The cross-document agreement of the taxonomy size (the wrapped-prose form).
require "$DC" 'taxonomy size: AGENTS.md states it several different ways' \
  'the taxonomy-size agreement check (wrapped prose forms)' iter-2549

# Hook events, derived from pub enum HookEvent.
require "$DC" "src.index('pub enum HookEvent {')" \
  'the hook-event count (HookEvent enum)' iter-2549
require "$DC" 'check "hook events"' \
  'the hook-event count check' iter-2549

# Workspace members, from Cargo.toml [workspace] members.
require "$DC" "tomllib.load(open('Cargo.toml','rb'))['workspace']['members']" \
  'the workspace-member count' iter-2549
require "$DC" 'check "workspace members"' \
  'the workspace-member count check' iter-2549

# Architecture Overview tree: every named path must exist; indented # Features:
# lines must be declared by the crate's [features] table.
require "$DC" 'Architecture Overview' \
  'the Architecture Overview tree path check' iter-2549
require "$DC" 'DRIFT tree paths that do not exist:' \
  'the tree-path existence check' iter-2549
require "$DC" 'DRIFT features named in the tree that the crate does not declare:' \
  'the tree Cargo.toml # Features: check' iter-2549

# AFT tools, registered via `.register(Aft`; every AFT count in the doc agrees.
require "$DC" '.register\(Aft' \
  'the AFT registered-tool count' iter-2549
require "$DC" 'check "AFT tools"' \
  'the AFT tool-count check' iter-2549
require "$DC" 'DRIFT AFT: AGENTS.md states the AFT count several different ways' \
  'the AFT count agreement check' iter-2549

# TUI modules, counted as `pub mod` lines in tui/mod.rs.
require "$DC" 'crates/operant-cli/src/tui/mod.rs' \
  'the TUI module count source' iter-2549
require "$DC" 'DRIFT TUI modules: AGENTS.md states no countable TUI-module count' \
  'the TUI-module count check' iter-2549

# Cargo features named in AGENTS.md must be declared by some crate.
require "$DC" 'cargo feature' \
  'the AGENTS.md cargo-feature mention scan' iter-2549
require "$DC" 'DRIFT cargo features named in AGENTS.md that no crate declares:' \
  'the cargo-feature declared check' iter-2549

# The live issue ledger's path, and the AGENTS.md tree must not name a root BUGS.md.
require "$DC" 'docs/audit/2026-06-19-BUGS.md' \
  'the issue-ledger path check' iter-2549
require "$DC" 'BUGS\.md' \
  'the root-BUGS.md absence check' iter-2549

# Library tool counts, derived from name() impls and builtin_tool_names().
# The marker is the semantic core: it reads the count from the code, so a
# rebase that hardcodes the count or drops the block fails here.
require "$DC" 'builtin_tool_names()' \
  'the library tool-count derivation (calls builtin_tool_names)' iter-2549
require "$DC" 'types? implement' \
  'the "N types implement name()" doc-count regex' iter-2549
require "$DC" 'DRIFT library tool counts:' \
  'the library tool-count agreement check' iter-2549

# =========================================================================
# scripts/verify-library.sh -- the library-page gate.
# =========================================================================

# Check 1: every backticked crates/ path exists.
require "$VL" 'MISSING-PATH' \
  'the backticked crates/ path existence check' iter-2551
require "$VL" 'crates/[A-Za-z0-9_./-]*' \
  'the crates/ path token regex' iter-2551

# Check 2: every mechanism page declares Source/Owns/Not for.
require "$VL" "grep -q '^Source:'" \
  'the Source: header requirement' iter-2551
require "$VL" "grep -q '^Owns:'" \
  'the Owns: boundary line' iter-2551
require "$VL" "grep -q '^Not for:'" \
  'the Not for: boundary line' iter-2551

# Check 3: every page ends with an Intention section.
require "$VL" "grep -q '^## Intention'" \
  'the Intention-section check' iter-2551

# Check 4: sibling .md links resolve.
require "$VL" 'DEAD-LINK' \
  'the sibling-link resolution check' iter-2551
require "$VL" 'docs/library/$t' \
  'the sibling-link target path' iter-2551

# Check 5 (the core, and the iter-2551 regression): the DERIVED registry.
# The registry is derived from name() impls, TOOL_NAME consts and the
# no-effect table -- NOT a hand-written allowlist. These markers are the
# semantic core of that derivation; a rebase that swaps in an allowlist
# drops them and fails here.
require "$VL" 'pathlib.Path("crates/operant-core/src").rglob("*.rs")' \
  'the derived-registry rglob over the source tree' iter-2551
require "$VL" 'fn name\(&self\)' \
  'the name() impl derivation (the trait-method shape)' iter-2551
require "$VL" 'const TOOL_NAME: &str' \
  'the TOOL_NAME const derivation' iter-2551
require "$VL" 'const TOOL_[A-Z_]*NAME: &str' \
  'the deferred-dispatch TOOL_<X>_NAME const derivation' iter-2551
require "$VL" 'const NO_EFFECT_TOOL_NAMES' \
  'the NO_EFFECT_TOOL_NAMES set derivation' iter-2551
require "$VL" 'UNKNOWN-TOOL' \
  'the unregistered-tool-name report' iter-2551
require "$VL" 'tool_shape=' \
  'the permissive tool-name shape regex' iter-2551

# The alias table and config-value derivations (so a retired name or a config
# value is not flagged as an unknown tool).
require "$VL" 'builtin_tool_aliases' \
  'the alias-table derivation' iter-2551
require "$VL" 'config_vals=' \
  'the config-value exclusion derivation' iter-2551
require "$VL" 'config_vals"' \
  'the config-value exclusion in the heading check' iter-2551

# Scoping: the check is applied to heading lines, not every backticked token.
require "$VL" "grep -n '^#\\+ .*' \"\$page\" | grep \"\${BT}\"" \
  'the heading-scoped tool-name scan' iter-2551

# Check 6: the coverage gate (E311 via scripts/genome).
require "$VL" 'lint --code E311' \
  'the E311 coverage gate' iter-2551

# --- HISTORY-LOSS CASE 1: the phantom allowlist must NOT come back --------
# iter-2551: a rebase restored a hand-written tool allowlist that named four
# phantom tools no module registers. The SIGNATURE of that regression is a
# `tool_shape` regex with the phantoms as explicit alternatives -- a shape
# class like `sub_agent` / `home_assistant` was how the old allowlist
# recognised them. A prose COMMENT naming them (the "never transcribe it"
# warning) is legitimate and must not trip this. So strip full-line comments
# first, then look for a shape alternative, not a bare mention.
if [ -f "$VL" ]; then
  _vl_code=$(grep -v '^[[:space:]]*#' "$VL")
  for _phantom in sub_agent learning_mutation home_assistant transcription; do
    if printf '%s\n' "$_vl_code" | grep -qF -- "$_phantom"; then
      say "FORBIDDEN-MARKER $VL: code (non-comment) names the phantom allowlist entry [$_phantom]"
      say "              protects: the derived registry -- a phantom tool name must never be an accepted shape (introduced iter-2551)"
      fail=1
    fi
  done
fi

# --- HISTORY-LOSS CASE 2: the tool-count derivation must read builtin.rs --
# iter-2549: the block was dropped, then a hardcoded wrong count (106 vs 105)
# sat undetected. The derivation must read tools/builtin.rs and call the real
# function, rather than being a literal. If either stops appearing, fail.
if [ -f "$DC" ]; then
  if ! grep -qF 'tools/builtin.rs' "$DC"; then
    say "MISSING-MARKER $DC: missing [tools/builtin.rs] -- protects: the library tool-count derivation reads the code, not a hardcoded number (introduced iter-2549)"
    fail=1
  fi
  if ! grep -qF 'builtin_tool_names()' "$DC"; then
    say "MISSING-MARKER $DC: missing [builtin_tool_names()] -- protects: the library tool-count derivation calls the real function (introduced iter-2549)"
    fail=1
  fi
fi

# =========================================================================
# MEMORY: the legacy `agentmemory_*` config keys are a DELIBERATE compat shim.
#
# Why this is a gate and not a comment. `MemorySettings` carries four
# `agentmemory_*` fields that no code reads: they exist only so an existing
# `operant.toml` that still names them PARSES (the struct is
# `deny_unknown_fields`, so removing a field makes such a config fail to load
# outright -- a config that worked yesterday failing today) and so the operator
# gets told the key is retired. That is a policy choice with a real cost in
# both directions, and it was made silently once already: an audit asked for
# "remove all legacy memory references" and 73 references stayed, with nothing
# in the tree saying that was intentional. This block makes the choice
# explicit and enforced.
#
# It does NOT decide for the operator. It fails when the shim and the prose
# describing it DISAGREE -- someone removing the fields without updating the
# comment, or editing the comment as if the fields were gone. Either half
# alone is the defect; both together is a decision that was actually made.
MC=crates/operant-core/src/config.rs
if [ -f "$MC" ]; then
  _fields=$(grep -cE '^    pub agentmemory_(url|secret|auto_spawn|version):' "$MC")
  # BOTH halves of the caveat are pinned, not one sentence. A check on a
  # single phrase passes when an adjacent sentence is rewritten to argue the
  # opposite -- "a trivial cleanup" sitting next to "deny_unknown_fields makes
  # removing a field a breaking change" reads as a contradiction and the gate
  # stayed green. Mutation-proven iter-2584.
  _caveat_breaking=$(grep -cF 'deny_unknown_fields makes removing a field a breaking change' "$MC")
  _caveat_deliberate=$(grep -cE 'legitimate choice, but it must be a deliberate one' "$MC")
  if [ "$_fields" -ne 4 ]; then
    say "SHIM-DRIFT $MC: found $_fields legacy agentmemory_* fields, expected 4"
    say "            protects: the config-compat shim -- an existing operant.toml that names a retired key must still PARSE (introduced iter-2581)"
    say "            if this removal is INTENTIONAL, it is a breaking config change: say so in docs/CHANGELOG.md and update the caveat below"
    fail=1
  fi
  if [ "$_caveat_breaking" -eq 0 ]; then
    say "MISSING-MARKER $MC: missing the compat caveat [deny_unknown_fields makes removing a field a breaking change]"
    say "            protects: the reason the shim exists is recorded next to the shim -- without it the fields read as forgotten cruft (introduced iter-2581)"
    fail=1
  fi
  if [ "$_caveat_deliberate" -eq 0 ]; then
    say "MISSING-MARKER $MC: missing the half that says removal must be DELIBERATE [legitimate choice, but it must be a deliberate one]"
    say "            protects: against the caveat being softened in place -- a note calling the removal 'a trivial cleanup' next to 4 dead fields is the defect this gate exists to catch (iter-2584)"
    fail=1
  fi
fi

# =========================================================================
if [ "$fail" -eq 0 ]; then
  say ""
  say "gates-present: every required marker present; no forbidden marker found"
else
  say ""
  say "gates-present: a gate script is missing a check -- the fix list is above"
fi
exit "$fail"