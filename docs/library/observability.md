# Observability — events, metrics, logs, insights

Owns: operant's observability mechanisms — the observer trait, runtime
retry metrics, logging, the insights engine, and the dashboards.
Not for: the recap surfaces ([context-injection.md](context-injection.md)),
the doctor ([operability.md](operability.md)).

Source: `crates/operant-core/src/observer.rs`,
`crates/operant-core/src/runtime_metrics.rs`,
`crates/operant-core/src/agent/insights.rs` + `tools/insights_tool.rs`,
`crates/operant-core/src/config.rs` (`[logging]` keys),
`crates/operant-cli/src/dashboard_server.rs`,
`crates/operant-cli/src/cmd_insights.rs`, `cmd_logs.rs`,
`crates/operant-api/src/observability_traits.rs` (the trait contract),
`crates/operant-core/src/turn_summary.rs`.

## The observer trait (`observer.rs` + api contract)

Structured events + metrics an observer can record, aggregate, or
forward (structured logging, Prometheus, OpenTelemetry — modeled on
`operant-api`'s `observability_traits.rs`). `ConsoleObserver` is the
built-in. `TurnSummaryCollector` (turn_summary.rs) is the display-only
observer that tallies a turn from the tool event feed, holding no
agent-loop state.

## Runtime metrics (`runtime_metrics.rs`)

A lock-free aggregation hook shared between the agent loop, the memory
sync executor, and the TUI status bar: atomic counters incremented at
the exact points the code already logs WARNs (stream-drop re-issues,
empty-content retries, sync_turn failures, channel-full job drops). The
TUI holds an `Arc<RuntimeMetrics>` and renders the counts — the numbers
are the same ones the logs carry.

## Insights (`agent/insights.rs`)

The session insights engine over the SQLite history: token consumption,
cost estimates, tool usage patterns, activity trends, model/platform
breakdowns, session metrics (hermes insights.py parity — the Python
reference is a machine-local checkout, not a repo path).
Surfaces: the `insights` tool (agent-callable), `operant insights`
(operator), and the stats dialog's `~/.operant/stats.jsonl` feed.

## Logging

`[logging]` config; the interactive TUI defaults to
`<home>/logs/operant.log` (rotation-capped) — a default-config run
logging nothing was the defect that rule closed (iter-1255). `operant
logs` reads them.

## The dashboard (`dashboard_server.rs`)

A FOUR-ROUTE status view served from a first-party page
(`crates/operant-cli/src/dashboard/index.html`, inlined):
`/api/status`, `/api/boards`, `/api/health`, `/api/config`. Not the
full web console — the 1.9 MB bundle calling 79 unimplemented routes
was removed (iter-1120, RT-44). Auth: a startup-generated session token
(accepted as `Authorization: Bearer`, `X-Hermes-Session-Token`, or the
HttpOnly cookie the CLI's browser launch exchanges); a foreign `Host`
header is refused (AD-040). `operant dashboard server --stop` /
`--status` manage a real pid file.

## Intention

Observability is opt-in wiring with zero hot-path cost: counters are
atomic and free, observers are trait objects subscribed at boot, and
the recap/advisor surfaces that DO cost LLM calls are separate and
default-off (see [context-injection.md](context-injection.md)). The
dashboard is deliberately small — a status view that cannot rot, rather
than a console whose frontend outruns its backend.
