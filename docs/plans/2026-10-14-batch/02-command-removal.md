# 02 — Remove dead and jcode-only slash commands

## Evidence

A headless sweep of every TUI-registered command on the deployed binary (iter-811) ran
each one as `tui debug simulate --keys "/<cmd><enter>"` with an isolated `HOME`. 86
commands are registered in `tui/operant_app/app.rs` (`RegisteredCommand::public/hidden`,
including multi-line entries). The dead ones fall into three groups.

**A. Named by the user (11)**, all of which print "`/x is registered but not wired in operant yet`":
`/commit /commit-and-push /commit-push-release /cut-release /git /test /swarm /subagent
/subagent-model /ssh /remote`

**B. Same class, found by the sweep (15)**, which print the same notice:
`/commit-push /merge /merge-remote-release /remote-release /fast-release
/fast-macos-release /swarm-prompt /initiatives /onboarding-preview /onboarding-sim
/productivity /provider-test-coverage` (plus its hidden alias `/model-status`)
`/compact-notifications /show-agentgrep-output /tool-call-details`

**C. These hit the core registry fallback "Command /x is not yet wired to a handler"**
(`operant-cli/src/commands.rs:995-1003`):
`/btw` (alias of `/background`), `/voice`, `/fork` (→ `/branch`), `/restart` (a gateway
command), and also `/observe`, which prints "Observe (side panel pending)".

The user asked to remove group A "and the like". Groups B and C are the like: advertised in
`/help` and in autocomplete, but doing nothing. `/goals` is wired (iter-808), but its
description says "Legacy alias for /initiatives", and `/initiatives` itself is dead.

## Where commands live (all must agree)

1. `tui/operant_app/app.rs`: `RegisteredCommand` list, which drives autocomplete, `/help`
   discovery and the "registered but not wired" notice.
2. `operant-cli/src/commands.rs`: `COMMAND_REGISTRY` (`CommandDef`) plus handlers. It is
   **shared with the gateway** (`gateway_commands.rs`), so only remove defs that are
   TUI-only. `/restart` and `/background` are gateway commands, so keep their `CommandDef`;
   just don't list them in the TUI registry.
3. `tui/operant_ui/ui_overlays.rs`: hand-curated `/help` entries (`help_entry(...)`) plus
   the auto-discovered "uncovered" list.
4. `tui/app/commands.rs`: the `handle_tui_command` match arms. Remove any arm that only
   served a removed command.
5. `tui/adapter_types/tui_app.rs:695-712`: the "registered but not wired" fallback. Keep
   it: it stays as the guard for future regressions.

## Steps

1. Tests first (`tui/app/tests.rs`, next to the iter-808 command tests):
   - `no_registered_tui_command_is_unwired`: for every name from
     `registered_command_names_including_hidden()`, drive the real dispatch path (the same
     one the sweep exercises: `handle_tui_command`, then the registry `resolve`/`execute`)
     and assert that the output is neither the "registered but not wired" notice nor the
     core "not yet wired to a handler" fallback. It is RED today for the 31 names above.
   - `removed_commands_are_absent`: the explicit list from A, B and C is absent from the TUI
     registry and from the rendered `/help` text.
2. Delete the `RegisteredCommand` rows for A, B, C (`/btw`, `/voice`, `/fork`, `/restart`,
   `/observe` included), and the hidden aliases that point at removed names
   (`/commit-and-push`, `/commit-push-release`, `/cut-release`, `/model-status`).
3. `/goals`: change its description to "Show goals and initiatives" (or whatever its
   handler actually shows). Drop the `/initiatives` row.
4. Remove the `/help` hand-curated entries for removed names, and any `info_widget` tip
   that advertises them (also covered by plan 10).
5. Remove the TUI-only `CommandDef`s with no other surface (`voice` is TUI-only: check
   `gateway_commands.rs` before deleting; if the gateway lists it, leave the def). Remove
   handler code that becomes unreachable (`cargo clippy` dead-code warnings guide this;
   keep the clippy count at parity).
6. Re-run the sweep script (see appendix) on the new binary. The only acceptable output is
   zero "not wired" hits.
7. Corpus: `help-overlay` (×2 variants) and `command-palette` goldens will change. Re-prove
   them and review the diffs.
8. `CHANGELOG.md`: list the removed commands under "Removed". `README.md`: remove any
   mention.

## Acceptance

- The sweep reports 0 unwired commands, and both new tests pass.
- `/help` lists no removed command, and autocomplete on `/co` doesn't offer `/commit*`.
- Gateway command tests (`cargo test -p operant-cli --bin operant gateway_commands`) are
  unchanged.

## Appendix: sweep script (run from any directory)

```bash
S=$(mktemp -d); printf '[client]\nbase_url = "http://127.0.0.1:9/v1"\n\n[agent]\nmodel = "offline-capture"\n' > $S/operant.toml
python3 -c "import re,pathlib;s=pathlib.Path('crates/operant-cli/src/tui/operant_app/app.rs').read_text();print('\n'.join(sorted(set(re.findall(r'RegisteredCommand::(?:public|hidden)\(\s*\"(/[a-z0-9-]+)\"',s)))))" > $S/cmds
while read c; do n=${c#/}; h=$S/h_$n; mkdir -p $h/.operant
  env -i HOME=$h HERMES_HOME=$h/.operant TERM=xterm-256color LANG=C.UTF-8 OPERANT_PERF_TIER=full ANTHROPIC_API_KEY=x PATH=/usr/bin:/bin \
    timeout 60 $OPERANT_BIN -c $S/operant.toml tui debug simulate --keys "$c<enter>" --size 120x40 --dump-screen $S/$n.txt >/dev/null 2>&1
  grep -qE "registered but not wired|not yet wired to a handler|side panel pending" $S/$n.txt && echo "DEAD $c"
done < $S/cmds
```
