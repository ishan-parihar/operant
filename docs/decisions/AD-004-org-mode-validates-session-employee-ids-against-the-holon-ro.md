---
ID: "AD-004"
Title: "Org mode validates /session employee ids against the Holon roster before binding"
Status: "Active"
Date: "2026-10-19"
Description: "Org mode validates /session employee ids against the Holon roster before binding"
---

# AD-004: Org mode validates /session employee ids against the Holon roster before binding

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

`/session <employee>` bound any id it was given, under a comment that the id resolves at the Holon seam. In org mode an id no employee backs binds a seat with no roster row. The live store (`~/.operant/database.db`, `gateway_sessions.employee_id`) holds `premiere` (26 rows), `chief-of-staff` (13), `crew-chief` (6), `hrmaster` (3), `compass` (1) and eight `emp-cron_*` ids. The live roster (`api_employees`) has four employees, Tess, Milo, Cara and Oto, under ids `emp_01M…`. None of the bound ids is a roster id.

## Decision

In org mode, `/session <id>` checks the id against `api_employees` before the bind (`roster_has_employee`, `crates/operant-cli/src/cmd_org.rs`). An id not on the roster is refused and nothing is bound. An unreadable roster also refuses, because the check did not run. Personal mode binds as before.

## Consequences

- The match is on the roster id, not the display name. `/session Tess` is refused; the id is what Holon binds.
- Stale rows already bound to off-roster ids keep their binding. Rebinding them is an owner decision and is not done here.
- The default binding for a new chat (`premiere`) is not checked, because the check runs only on an explicit `/session <id>`. `premiere` is not on the roster, so that default is the next gap to decide.
- Only existence is checked. Roster status (retired, archived) is not.

## Orthogonality

The gateway /session comment says the id resolves at the Holon seam, but no record decides whether the bind checks the roster first. (recon R1019053854: session tags body run holon roster)

## Revisions

- 2026-10-19: created
