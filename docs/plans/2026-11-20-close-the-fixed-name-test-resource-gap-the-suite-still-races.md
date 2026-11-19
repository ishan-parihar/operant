# Close the fixed-name test-resource gap: the suite still races on files every test shares

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Written by the operator agent after iter-2711/2712/2713 landed. Every number below was
MEASURED at `origin/main` = `95fc927a` (iter-2714) in a CLEAN worktree before it was
written down.

## Claim rule

Never take a number from a header or a previous plan. `git fetch origin`, read the top
label from `origin/main`, add one. Re-run every command in the baseline before trusting
the row that cites it.

## Why this plan exists next to iter-2703's

`docs/plans/2026-11-20-close-the-test-pollution-gap-the-lib-suite-is-red-at-tip-and.md`
owns the FIRST-order fix: iter-2711 found that a config test deleted the process cwd and
took the lib suite from 27 failures to 5; iter-2712 added the canary. That work is done
and correct.

What it did NOT close is the SECOND-order defect it exposed. iter-2711's fix was
incomplete in a way the suite immediately re-measured: the same class of race remains,
because the tests it touched still share a resource by a FIXED NAME. This plan owns that.

## Measured baseline (all commands re-runnable)

At `origin/main` = `95fc927a`, CLEAN worktree at `/tmp/audit2713`, `source scripts/dev-env.sh`:

```text
$ cargo test -p operant-core --lib
test result: FAILED. 3648 passed; 8 failed; 2 ignored; 0 measured; 0 filtered out

$ cargo test -p operant-core --lib -- config::tests::lifeos_key_is_gone_from_config
test result: ok. 1 passed; 0 failed          # <- passes alone

$ cargo test -p operant-core --lib -- gateway::slack::tests::a_short_slack_body_is_one_message
test result: ok. 1 passed; 0 failed          # <- passes alone

$ cargo test -p operant-core --lib -- gateway::slack::
test result: ok. 6 passed; 0 failed          # <- passes as a module

$ cargo test -p operant-core --lib -- tools::kernel::tests::
test result: ok. 12 passed; 0 failed         # <- in the MAIN tree (has .venv)
```

The 8 failures split into two groups and NONE is a broken feature:

* 5 `tools::kernel::tests::*` -- environmental, the worktree has no
  `kernel-sidecar/.venv` (untracked, AGENTS fleet rule 6). VERIFIED: 12 passed in the
  main tree. Already in iter-2712's canary baseline.
* 1 `config::tests::lifeos_key_is_gone_from_config` -- passes alone, fails in the suite.
* 2 `gateway::slack::tests::*` -- pass alone AND as a module, fail only in the full suite.

## The defect: a sidecar keyed on the DIRECTORY, not the config file

`config_heal::unknown_sidecar_for` (`crates/operant-core/src/config_heal.rs:206`):

```rust
pub const UNKNOWN_SIDECAR: &str = "operant.unknown.toml";
pub fn unknown_sidecar_for(config_path: &Path) -> PathBuf {
    let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(UNKNOWN_SIDECAR)
}
```

The sidecar's name is a CONSTANT. So EVERY config file in a given directory shares ONE
sidecar. Three tests do exactly that:

| test | config path |
|---|---|
| `config::tests::harness_settings_roundtrip_and_default_is_dark` | `$TMP/operant-harness-park-<pid>.toml` |
| `config::tests::pk_settings_roundtrip_and_default_is_dark` | `$TMP/operant-...-<pid>.toml` |
| `config::tests::lifeos_key_is_gone_from_config` | `$TMP/operant-lifeos-probe-<pid>-<nanos>.toml` |

All three resolve to `$TMP/operant.unknown.toml`. MEASURED:

```text
$ python3 -c "import os; pid=os.getpid(); a=f'operant-harness-park-{pid}.toml'; b=f'operant-harness-park-{pid}.toml'; print('identical:', a==b)"
identical: True
```

`iter-2364` fixed half of this and the comment in `config.rs:5860` records the fix:
"a path of this test's OWN ... `unknown_sidecar_for` keys on the DIRECTORY only". But
the path it chose was `format!("operant-harness-park-{}.toml", std::process::id())` --
and `std::process::id()` is CONSTANT for the whole test binary. The whole lib suite is ONE
process. So the fix made each test's config unique and left the sidecar shared.

iter-2711's nanos stamp for the lifeos test does not help either, because the sidecar
does not include the config's filename. The stamp makes the CONFIG unique; the sidecar is
still `operant.unknown.toml`.

That is why the failure is nondeterministic: whether a test reads its own parked key or a
neighbour's depends on thread interleaving, and `write_parked_sidecar` OVERWRITES.

## The second-order defect: the config-surface ratchet cannot see a 2-character field

`scripts/audit-config-surface.py` scans for uses with:

```python
for ident in set(re.findall(r"\b[a-z_][a-z0-9_]{2,}\b", text)):
```

`{2,}` requires THREE or more characters. So a field named `ui` or `pk` is invisible.
MEASURED:

```text
config.ui.generative_enabled  ->  ['config', 'generative_enabled']     # 'ui' never matched
config.pk                     ->  ['config']                          # 'pk' never matched
```

And exactly two fields in `config.rs` are that short:

```text
$ python3 -c "...findall(r'^\s*pub (\w+)\s*:', src, re.M)..."
pub fields with <3 chars: ['pk', 'ui']
```

`pk` is already in `ACCOUNTED_FOR` (so its blind spot is masked and harmless).
`AppConfig.ui` is NOT, so at this tip the ratchet is RED:

```text
$ python3 scripts/audit-config-surface.py --check
AppConfig: 2/39
    pk  (the [pk] table line (RT-136): the live kernel surface is [tools.kernel])
    ui  <-- NOT ACCOUNTED FOR
DRIFT a config setting is accepted, defaulted and read by nobody:
  AppConfig.ui
```

**`AppConfig.ui` IS read.** MEASURED, five qualified reads:

```text
crates/operant-core/src/ui/gate.rs:72   if !config.ui.generative_enabled {
crates/operant-core/src/ui/gate.rs:118  config.ui.generative_enabled = enabled;
crates/operant-core/src/tools/ui_tool.rs:697   let was = crate::config::runtime_config().ui.generative_enabled;
crates/operant-core/src/tools/ui_tool.rs:699   config.ui.generative_enabled = enabled;
crates/operant-core/src/tools/ui_tool.rs:708   config.ui.generative_enabled = self.was;
```

So this is a FALSE POSITIVE produced by the gate's own regex, and it appeared only now
because iter-2699 registered the `ui` section (and thus made the field reachable to the
ratchet's struct walk). A gate that cries wolf on a live setting is exactly the failure
mode `audit-config-surface.py`'s own docstring warns about.

## The tranches

Numbering is deliberately unlike other plans' so they can be cited side by side.

### T1 -- Give the sidecar a per-config name (the real fix)

Change `unknown_sidecar_for` so the sidecar is derived from the CONFIG FILENAME, not only
its directory, e.g. `operant.<config-stem>.unknown.toml`. This is the structural fix:
two configs in one directory then get two sidecars, and no test can read another's
parked keys.

Check every existing caller before changing it -- the sidecar is an OPERATOR-VISIBLE
file, so a rename changes where an operator looks for parked keys. If a rename is too
disruptive, keep the constant for the DEFAULT config path and derive per-config names
only for non-default names. Decide, and record the reasoning in the commit.

**Verify:** a test that creates two configs in ONE directory, parks a distinct key in
each, and asserts each sidecar holds only its own key. Mutation: revert the derivation
and watch it fail.

### T2 -- Make the three parking tests use their own directory

Independently of T1, each of the three tests should own a `tempfile::TempDir` and put its
config INSIDE it, so even a directory-keyed sidecar is unambiguous. This is defence in
depth: T1 fixes the mechanism, T2 removes the shared-directory precondition.

**Verify:** each of the three passes in the full suite, repeatedly. Mutation: move one
config back to the shared `temp_dir()` and watch it flake.

### T3 -- Fix the ratchet's identifier regex (the false positive)

In `scripts/audit-config-surface.py`, stop relying on a minimum length to find field
uses. Scan each FIELD NAME explicitly with a word-boundary match, keeping the general
identifier scan for everything else. That makes `ui` and `pk` visible without widening
the general pattern to match one-character noise.

Do NOT add `AppConfig.ui` to `ACCOUNTED_FOR` -- it is genuinely read, so accounting for
it would be recording a lie. Fix the detector.

**Verify:** `python3 scripts/audit-config-surface.py --check` prints 0 new candidates,
AND a mutation that removes the `ui/gate.rs:72` read makes it fail by name.

### T4 -- Attribute the two Slack failures

`gateway::slack::tests::{a_short_slack_body_is_one_message,
the_slack_streaming_send_chunks_an_over_limit_body}` pass alone and pass as a module, and
fail only in the full suite. Their adapter takes a per-instance `with_api_base` override
and the send path reads no global (measured), so this is NOT the same shape as T1/T2.
Diagnose before changing anything: the likely candidates are a timeout under parallel
load or a shared `SLACK_*` env var another test sets. Record the verdict.

Do NOT add them to the canary baseline until the cause is known -- an unexplained
failure in the baseline is how a real bug becomes invisible, which is the whole thing
iter-2712 exists to prevent.

**Verify:** a written verdict, and either a fix or an evidence-backed environmental
disposition.

### T5 -- Regenerate the two genome zones

`genome lint` reports exactly two errors:

```text
E019 DRIFT TUI overlays: AGENTS.md says 29, the array holds 32
E104 AGENTS.md#stack: zone `stack` is stale -- `genome outline`
```

Both are generator fixes, not prose edits: `genome outline` for E104, and for E019 either
re-derive the overlay count from `App::overlay_flags()` in `check-doc-claims.sh` (the
authority AGENTS.md names) or regenerate the zone. E019 is the more interesting half:
the number drifted because the doc is hand-typed while the array grew, which is the same
class as iter-2699's hand-maintained registry list.

**Verify:** `genome lint` prints 0 errors, and a mutation (change the prose to 29) fails
`check-doc-claims.sh` by name.

## Gates / success criteria

(fixed before the run)

1. `cargo test -p operant-core --lib` -- **5 failed**, and all 5 are the accounted
   `tools::kernel::tests::*` environmental set. Not "fewer"; exactly these.
2. `python3 scripts/test-pollution-canary.py --check` exits 0 with 5 accounted and 0
   unaccounted.
3. `python3 scripts/audit-config-surface.py --check` exits 0 with no new candidates.
4. `python3 scripts/audit-ledger-index.py` prints OK.
5. `./scripts/genome lint` prints 0 errors.
6. `bash scripts/clippy-warning-gate.sh` reports `new=0`.
7. Every fix ships a mutation proof: reintroduce the defect, watch the named test or
   gate fail.

## Cost

T1 is a small change to one function with a wide blast radius (it is an operator-visible
path), so it needs the caller check done properly. T2 is three test-local changes. T3 is
one regex plus a per-field check. T4 is diagnosis that may end in "environmental".
T5 is two generator runs plus one derivation.

Estimate ~5 iterations.

## Order and dependency

```text
T1 (sidecar per-config)  --+
T2 (test-local dirs)     --+--> T3-independent
T3 (ratchet regex)       --+
T4 (slack diagnosis)     ----> (may join T1's class if it is shared state)
T5 (genome zones)        ----> independent
```

T1 and T2 are independent and can land in either order; T3 is independent of both.
T4 should be diagnosed BEFORE deciding whether it joins this plan's class or becomes its
own row.

## What this plan explicitly does NOT do

- It does not touch the three open ledger rows (RT-190/280/543) -- owner decisions.
- It does not re-do iter-2703's work; iter-2711/2712 landed it. This is the second-order
  residue that work exposed.
- It does not add an unexplained failure to the canary baseline (T4).
- It does not "fix" a gate by deleting it. T3 fixes the detector; it does not silence the
  ratchet.
- It does not rename the operator's sidecar without recording the reasoning (T1).
