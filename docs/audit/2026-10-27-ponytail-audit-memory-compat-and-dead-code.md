# Ponytail audit: memory-provider compat and workspace redundancies (2026-10-27)

## Premise check (run before cutting)

Operator's ruling: no users exist, therefore no backwards compatibility is
required. VERIFIED against the only surface where it is checkable — the live
config /home/ishanp/.operant/operant.toml carries ZERO retired keys (no
agentmemory_*, no audit_enabled/response_cache_enabled). It is not git-tracked,
so deleting fields cannot break a repo artifact.

That verifies the premise for config FIELD deletion. It does NOT verify it for
the two other compat classes, and those are the ones where cutting would
introduce a defect rather than remove one. Detail below.

## Finding 1 — 2,584 lines of dead code, proven dead and unguarded (THE REAL FINDING)

Two modules have zero callers anywhere in the workspace, including tests:
- crates/operant-core/src/retrieval.rs — 1,068 lines, 6 pub fns
- crates/operant-core/src/voice.rs — 1,516 lines, 25 pub fns

Proven by MODULE-QUALIFIED call-path search (`operant_core::retrieval::`,
`voice::` etc): 0 qualified sites outside their own files. The first pass at this
scan used bare fn names and produced a false "alive" verdict (`new` matches 7,779
sites across the workspace, `as_str` 1,429) — that method was discarded as
worthless, and only the qualified-path method is relied on.

ACID TEST (not just search): with both `pub mod` declarations removed from
lib.rs, `./scripts/check.sh check -p operant-core --lib` finishes clean in 24s,
0 errors. lib.rs was restored immediately after. So this is a genuine delete,
not a risky one.

Unprotected: nothing depends on either module, unlike the fields below. This is
the cleanest deletion in the audit — no warning, no doc, no test to adjust.

## Finding 2 — SIX dead config fields (the user-requested deletion), one caveat

18 config fields have zero readers outside config.rs. The six that matter here
are the compat survivors: agentmemory_url, agentmemory_secret,
agentmemory_version, agentmemory_auto_spawn, agentmemory_version,
audit_enabled, response_cache_enabled.

CAVEAT, and it inverts the naive conclusion: "zero readers" is FALSE for the
warning mechanism. config.rs:1789-1791 reads all three agentmemory_* PRESENCE
flags, and config.rs:2751 reads them in the PartialEq — because that IS the
iter-1412 warning I added. So they are not dead weight, they are the load-bearing
part of the only thing protecting a parked credential. Deleting the fields
deletes the warning, which re-creates the exact defect iter-1412 fixed:
agentmemory_secret could sit in a config and in backups forever with nothing
telling the operator it is unused.

Resolution that satisfies the operator's premise AND keeps the protection:
delete the fields, and let the *generic* unknown-key path carry the message.
Holding back 66 deny_unknown_fields sites is the cost of the naive version; the
correct version drops deny_unknown_fields, then deletes the fields, then makes
the unknown-key rejection print the retired-key hint. Order matters: fields
first (today) breaks every config that names them.

## Finding 3 — the alias map must NOT be cut (a real user-facing dependent exists)

builtin_tool_aliases() maps memory_recall/memory_save/memory_smart_search. The
"no users" premise would cut it. But four SHIPPED artifacts teach the retired
names as current (skills.md:27 lists all five as the Memory toolset,
configuration.md:20 names memory_recall, docs/library/memory.md 3x,
tools.md 1x). Removing the map without moving those breaks users who followed
shipped instructions — the exact class the layer exists to protect. This is
unchanged from the iter-1367 stress-test; the premise being true does not make
the shipped docs wrong. Scope is already fully enumerated.

## Finding 4 — one dead trait claim RETRACTED

My first pass reported `ToolProvider` as a vestigial trait with 0 impls (a
delete candidate). RETRACTED: it is a plain struct fixture in
tests/harness_live_loop.rs, not a trait at all — `grep "trait ToolProvider"`
returned nothing because there is no trait. Deleting it breaks a live test.
Recorded because the audit method (impl-count without kind-check) produced a
confident false positive that would have shipped.

## Ranked shortlist

1. retrieval.rs + voice.rs (2,584 LOC) — DONE, iter-1419/dead_modules_stay_dead.rs.
   Compile-proven, zero callers, no dependents, no docs. The ponytail answer.
2. Six dead config fields — DELETE DEFERRED, then cancelled on evidence (see
   the 2026-10-27 addendum below).
3. One-caller factories to collapse: build_browser_provider(_name) — CANCELLED
   as a FALSE POSITIVE (see the addendum).
4. Alias map — do NOT cut (Finding 3).

## 2026-10-27 addendum: two of my own shortlist items were wrong

Executed against the list and found two of the four entries did not survive
investigation. Both are recorded because a future agent reading the audit
without the addendum would repeat them.

**Item 3 (build_browser_provider) — RETRACTED, it is not indirection.** The
audit called `_name` an ignored argument, which is false: the caller in
`browser_tool.rs` passes a REAL config value (`cfg.browser.provider`), and the
factory's whole documented job is to map every name — including the retired
`igs` / `obscura` / `lightpanda` — onto sourcehound instead of failing the
config load. Three tests pin that, one of them naming the reason. Collapsing it
would delete a compatibility shim the operator's config still depends on, not
remove dead weight. The single-impl count that made it look like a
collapsible factory was the same impl-count method that produced the ToolProvider
false positive (Finding 4).

**Item 2 (config fields) — CANCELLED, deletion would break the thing that
protects the secret.** The operator's premise is verified true (zero retired
keys in the live config), and it does license deletion. But deletion is not
"done right" here, because the fields are load-bearing in a way the audit
framed as dead weight: `every_retired_memory_key_warns` is the regression test
for the iter-1412 fix, and removing the fields removes the only notice that a
parked `agentmemory_secret` is unused. A generic unknown-key message could
replace it, but that is a behavior change across 68 deny_unknown_fields
structs, and the struct comment states the decision it would violate: "a
config that parsed yesterday must parse today". This is a judgement call
between two defensible positions, so it is left to the operator rather than
quietly executed on an audit's ranking.

The general lesson, stated for the record: a ponytail audit's shortlist is a
hypothesis, not a work order. Both retracted items came from counting
implementations and counting readers — structural signals that look like
indirection and are sometimes deliberate compatibility.
