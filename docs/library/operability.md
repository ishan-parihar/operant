# Operability — doctor, setup, update, service, backup

Owns: the operator's lifecycle surface — the doctor's check/fix battery,
the setup wizard, self-update, the Linux service install, and
backup/import.
Not for: the gateway daemon ([gateway.md](gateway.md)), the dashboard
([observability.md](observability.md)), the iteration/deploy protocol
(`AGENTS.md`).

Source: `crates/operant-cli/src/cmd_doctor/` (mod, checks_api,
checks_config, checks_fix, checks_tools, check_result),
`crates/operant-core/src/doctor.rs`, `crates/operant-core/src/binary_updater.rs`
(the self-updating integrations), `crates/operant-cli/src/cmd_setup.rs`,
`cmd_update.rs`, `cmd_service.rs`, `cmd_backup.rs`, `cmd_import.rs`,
`cmd_uninstall.rs`, `crates/operant-cli/src/mcp_serve.rs` (server side),
`crates/operant-core/src/mcp.rs` (client side).

## The doctor (`operant doctor`)

A 5-check battery: API connectivity (probes the CONFIGURED endpoint,
never the provider default — the omp omniroute false-failure was the
defect, iter-336), config validity, database integrity, tool
registration, version/dependency status. Checks never send a base URL
as a Bearer token (iter-344) and the provider checks read the static
`provider_catalog.rs` (157-name recognition surface) since the second
provider surface was deleted (iter-1215, RT-153). `--fix` runs the
repair legs (`checks_fix.rs`); results are typed
(`check_result.rs::CheckResult`).

## Setup (`operant setup`)

The interactive wizard: model config → provider key → runtime defaults,
with a `--non-interactive` report mode. The product's intended
first-run path.

## Self-update (`operant update`)

`binary_updater.rs` is the ONE updater with FOUR consumers (operant
itself + three integrations: AFT, sourcehound, ponytail), distinguished
by `InstallKind` (Binary vs Content). For operant's own binary: fetch
the latest release → download → verify → swap-in. For integrations:
versioned directories with keep=2, guard-scanned content, auto-update
checks (AFT/sourcehound) or fully opt-in install (ponytail —
default OFF).

## Service install (`operant service install`)

`cmd_service.rs` — the systemd unit for the gateway daemon, with
`--start`, `--stop`, `--status`, `--uninstall`.

## Backup / import / uninstall

`operant backup` snapshots the operant home (state + config); `operant
import` restores one; `operant uninstall` removes the install (asks
before touching `~/.operant`).

## operant as an MCP server

`operant mcp serve` (mcp_serve.rs) exposes operant's tools over stdio
MCP — the symmetric other half of the MCP client story
([mcp.md](mcp.md)): operant both consumes external MCP servers and can
BE one.

## Intention

These commands exist so an operator never has to read source to keep a
deployment healthy: the doctor diagnoses, setup bootstraps, update
keeps current, service runs it headless, backup protects the state.
The one-updater/one-install-kind split is deliberate — the fleet
learned that per-integration updaters drift (four ad-hoc mechanisms
were unified into `binary_updater.rs` with versioned dirs + keep=2).
The MCP-serve surface exists for the same reason operant consumes MCP:
tools compose without forks.
