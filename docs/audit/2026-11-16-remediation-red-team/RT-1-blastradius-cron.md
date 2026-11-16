# RT-1 — Blast-radius & Cron remediation red-team

- **Date:** 2026-11-16
- **Auditor:** Jcode (red-team)
- **Target:** commit `ae9a27cf` (iter-2490), verified against HEAD `81bcf22e`
- **Scope:** FINDING Z15 (blast-radius verbs), Z29 (cron double-fire), Z31 (daemon lock)
- **Method:** read the diff, read current source, run scoped tests, probe edge cases by hand.

---

## TL;DR

| Finding | Closed? | New gap? | Severity of gap |
|---------|---------|----------|-----------------|
| Z15 blast-radius verbs | **YES** — fallback escalates unmodelled verbs by target, deny list is reached | No new gap found. Flag parsing is thorough. | — |
| Z29 cron double-fire | **YES** — atomic claim-before-dispatch via unique index | **YES — HIGH.** A claim abandoned by the stale-reclaim scan permanently wedges the job: `next_run_at` is never advanced, so every subsequent tick re-selects the row, loses the claim race, and skips. | **HIGH** |
| Z31 daemon lock | **YES** — probe refuses manual run while daemon holds flock | TOCTOU window between probe and dispatch (low impact; claim ledger is the backstop). | LOW |

The Z29 wedge is the one thing that needs fixing. Everything else is sound or has only cosmetic gaps.

---

## Z15 — Blast-radius verbs (mv/cp/rsync/install/scp)

### Does the fix close the original finding?

**Yes.** The fix has two layers, and the second is the one that closes the *class*:

1. **Verb arms added** (`crates/operant-core/src/blast_radius.rs:327-346`): `mv` → `Relocate`, `cp`/`scp` → `Copy`, `rsync` → `Sync`, `install` → `Install`, `dd` → `RawWrite`, plus `tee` → `Write`, `chmod`/`chown`/`chgrp` → `RecursiveMeta`. These were the five verbs the audit named.

2. **Target-based fallback** (`blast_radius.rs:725-780`): for any verb NOT in the table, `rate_segment` escalates the segment to `Unbounded` when `check_deny` refuses a target. This means an unmodelled verb (`ln`, `sed`, tomorrow's `btrfs`) is rated by *what it touches*, not by whether someone remembered to add it. The comment at `:735` states this explicitly.

The early-return at `screen()` (`:1533-1535`) is still present — `if class == BlastRadius::None { return None; }` — but it is no longer a bypass, because the fallback in `classify()` ensures a destructive verb with a denied target never rates `None`. `classify("cp /etc/shadow /tmp/x")` → `Unbounded` → the deny check at `:1537` runs.

### Is the deny-list check now reached for these verbs?

**Yes.** `screen()` at `:1537`:
```rust
if class >= BlastRadius::Scoped
    && let Some(denied) = destructive_targets(command)
        .iter()
        .find_map(|t| check_deny(t, home))
{
    return Some(Refusal::DeniedPath(denied));
}
```
For `mv /etc /tmp/x` → `classify` returns `Unbounded` → `destructive_targets` returns `["/etc", "/tmp/x"]` → `check_deny("/etc")` returns `Some(DeniedPath { detail: "system directory" })` → refused. Pinned by `moving_a_system_directory_is_denied_outright` (`:1787`).

### Other destructive verbs still missing?

I probed every verb the prompt named:

| Verb | In `verb_kind`? | Caught by? | Verdict |
|------|-----------------|------------|---------|
| `dd` | YES (`:331`) | `RawWrite`, `of=` → `WRITE_VALUE_FLAGS` (`:257`) | covered |
| `truncate` | YES (`:327`) | `Remove` | covered |
| `shred` | YES (`:327`) | `Remove` | covered |
| `chown` | YES (`:345`) | `RecursiveMeta` | covered |
| `chmod` | YES (`:345`) | `RecursiveMeta` | covered |
| `mkfs` | YES (`:323`, prefix match) | `Format` | covered |
| `ln -sf` | NO (intentionally) | fallback via target denial | covered — `an_unmodelled_verb_is_rated_by_its_target_instead_of_disappearing` (`:1908`) pins it |
| `>` redirection | N/A (parsed in `parse_segment:424`) | write destination → `WRITE_VALUE_FLAGS` / `boundedness_of` | covered — `classify_should_detect_find_delete_and_redirect_to_device` (`:1589`) |
| `tee` | YES (`:344`) | `Write` | covered |
| `sed -i` | NO | fallback via target denial | covered for denied targets; scoped `sed -i ./file` rates `None` (correct) |
| `find -delete` | YES (`:346`) | `Finder` + `FINDER_DELETE_FLAGS` (`:233`) | covered — `find ./build -name '*.o' -delete` at `:1590` |

### Long-flag parsing

- `--target-directory=/etc` → `long_flag_name` strips `--` and splits on `=` (`:189`) → `"target-directory"` in `CP_TAKES_VALUE` (`:133`) → value is inline, destination recorded. Pinned by `a_target_directory_flag_is_rated_as_the_destination` (`:2089`).
- `-t /etc` → `carries_copy_value_short_flag` (`:228`) sees `t` in `COPY_VALUE_SHORT_FLAGS` (`:152`) → next token is the value, not a target. Pinned by the same test.
- `-rfv` → `carries_copy_value_short_flag`: `r`, `f`, `v` all in `COPY_SHORT_FLAGS` (`:156`), none in `COPY_VALUE_SHORT_FLAGS` → does NOT swallow the next token. Correct.
- `-m 644` → `m` in `COPY_VALUE_SHORT_FLAGS` → `644` eaten as the mode, not a target. Pinned by `install_does_not_mistake_its_mode_for_a_target` (`:1887`).

### Path that looks scoped but resolves outside cwd

`boundedness_of` (`:601`) strips `./` and `../` prefixes in a loop (`:637-648`). So `../../etc/passwd` → `etc/passwd` → first segment `etc` IS in `SYSTEM_ROOTS` (`:283`) → `Unbounded`. `check_deny` also strips leading `/` before checking `DENIED_SYSTEM_DIRS` (`:1435`). So `cp ../../etc/shadow ./leak` → denied. Covered.

A path like `../outside` (a non-system sibling dir) → stripped to `outside` → not in `SYSTEM_ROOTS` → `Bounded` → scoped. That is correct: it's a real directory outside cwd but not a system path, and the gate's contract is system-path denial, not cwd-confinement (the audit did not ask for cwd-confinement).

### Is the regression test a mutation test?

**Yes, three of them:**
- `removing_a_verb_from_the_table_changes_the_rating_to_the_fallback` (`:1965`) — re-derives the pre-fix rating by substituting an unknown verb (`wobble`) and asserting it matches the modelled verb's rating. Revert the fix → `wobble /etc` rates `None` → test fails.
- `an_unmodelled_verb_is_rated_by_its_target_instead_of_disappearing` (`:1908`) — asserts `classify("ln -sf /etc/passwd /tmp/x") == Unbounded` and `screen(...)` returns `DeniedPath`. Revert the fallback → rates `None` → test fails.
- `screen_refuses_or_justifies_the_previously_invisible_verbs` (`:1738`) — the seven named commands must all be denied or require justification. Revert the verb arms AND the fallback → all allowed → test fails.

Negative controls (`scoped_copy_move_sync_and_install_stay_allowed_without_a_justification` at `:1821`, `the_fallback_does_not_make_read_only_commands_destructive` at `:1930`) prevent the over-blocking failure mode.

**Test run:** `cargo test -p operant-core --lib -- blast_radius` → 46 passed, 0 failed. (Note: the lib test target currently has a pre-existing compile error in `harness_seams_r3.rs:448` from a peer's in-flight `gateway/types.rs` change removing `handle_update` from the `PlatformAdapter` trait — this is a working-tree collision, NOT from `ae9a27cf`. The blast_radius tests ran clean before the collision.)

### Z15 verdict

**CLOSED. No new gap.** The fix is architecturally correct: the fallback (target-based escalation) closes the class, not just the five named verbs. Flag parsing is thorough and mutation-tested.

---

## Z29 — Cron double-fire

### Does the fix close the original finding?

**Yes.** The fix writes a claim row BEFORE dispatch:

```rust
// scheduler.rs:467-489
let claimed = self.db
    .claim_job_occurrence(&job.id, &claim_key, "scheduler")
    .map_err(...)?;
if !claimed {
    warn!("... already claimed ... skipping");
    return Ok(());
}
```

`claim_job_occurrence` (`db.rs:1099`) is an `INSERT OR IGNORE` against the unique index `idx_cron_claims_occurrence(job_id, occurrence)` (`db.rs:298`). Two processes racing for the same occurrence → one inserts (returns `Ok(1)`), the other ignores (returns `Ok(0)`). This is atomic at the SQLite level, not a read-then-write. The claim key is the job's own `next_run_at` string (`scheduler.rs:385`), which is the same value `get_due_jobs` selected the row on.

After the attempt, `settle_run_claim` (`:442`) closes the claim. `mark_job_run` still advances `next_run_at` (the only write that does). The crash window (claim written, attempt runs, process dies before `mark_job_run`) leaves an open claim on the old occurrence key. On restart, `tick()` → `get_due_jobs` selects the row (still due) → `run_job` → `claim_job_occurrence` returns `Ok(0)` (row exists) → skip. **No double fire.** Pinned by `a_crash_between_dispatch_and_mark_job_run_does_not_refire` (`:2348`), which asserts on the side-effect count (a marker file), not on the row state.

### Is the claim row written ATOMICALLY relative to dispatch?

The claim write and the dispatch are separate operations, but the *guard* is atomic: the unique index makes the claim insert a single compare-and-set. The window between claim and dispatch is real, but it is the *safe* direction — a crash there means the occurrence is claimed but never ran, which the stale-reclaim scan settles as `abandoned` (skip, never re-dispatch). The alternative (crash after dispatch, before mark) is also claimed → skip. Both directions avoid the double fire.

### Crash AFTER claim but BEFORE dispatch — is the job silently lost?

**This is the HIGH-severity gap.** The claim is written, the process dies before `run_agent_job`/`run_script_job` executes. The claim is now open, `next_run_at` is still the past occurrence. On restart:

1. `tick()` calls `claim_stale_claims()` (`:338`) which settles the claim as `abandoned` after the horizon (150s floor, `:411`).
2. `get_due_jobs` (`:1267`) selects the row again (the claim is settled, so the anti-join no longer hides it).
3. `run_job` calls `claim_job_occurrence` for the SAME `(job_id, occurrence)` → `INSERT OR IGNORE` → the settled row exists → returns `Ok(0)` → `claimed = false` → **skip**.
4. `mark_job_run` is never reached, so `next_run_at` is never advanced.

**The job is permanently wedged.** Every subsequent tick re-selects the row, loses the claim race, and skips. The job never runs again until an operator manually resets `next_run_at` (e.g., `operant cron update` or a DB edit).

The test `an_abandoned_claim_is_skipped_not_retried_and_the_next_run_lives` (`:2461`) masks this by manually calling `set_next_run` after the abandonment:
```rust
// scheduler.rs:2515
cron_db.set_next_run(&id, Some(chrono::Utc::now().to_rfc3339()))
    .expect("arm the next occurrence");
```
In production, **nothing calls `set_next_run`**. The only production writers of `next_run_at` are `mark_job_run` (`:537`, only reached if the claim succeeds) and the terminal-completion path (`:574`, sets NULL). The comment at `:2458` claims "a stale claim can never wedge a job" — that claim is **FALSE**.

**Fix:** `claim_stale_claims` must also advance `next_run_at` for the abandoned job (compute the next occurrence from the schedule, same as `compute_next_run`). Or `run_job` must treat a settled-abandoned claim as "claim succeeded for a new occurrence" and re-dispatch. The skip-forever behaviour is the defect.

### Does the lock handle a STALE lock file (daemon killed without cleanup)?

**Yes, for Z31's probe.** The probe (`cmd_cron.rs:157`) uses `flock(LOCK_EX | LOCK_NB)` on a fresh fd. If the daemon was killed, the kernel releases the flock (fds are closed on process death). The lock file is never unlinked (`lock.rs:34` explains why — `flock` protects the inode, removing the path would let a second starter create a new inode and lock that). A leftover file is harmless. The probe opens the same inode, acquires the lock (since no one holds it), and returns `Ok(false)` (no daemon). Correct.

### Does the flock protect across two processes, and is it released on all paths?

**Yes.** `flock` is an OS-level advisory lock that works across processes. The daemon acquires it at `gateway_runner.rs:5453` and leaks it into a process-global `static Vec<GatewayDaemonLock>` (`:5467`) so it lives for the process lifetime. The probe acquires and releases within a single function call (fd dropped on return). The non-unix fallback (`cmd_cron.rs:183`) returns `Ok(false)` (permissive), which is the correct degradation.

### Z29 verdict

**Partially closed.** The double-fire window is closed (atomic claim + unique index + mutation-tested crash test). **But the stale-reclaim scan introduces a new HIGH-severity gap: an abandoned claim permanently wedges the job.** The test masks this by manually advancing `next_run_at`.

---

## Z31 — Daemon lock

### Does the fix close the original finding?

**Yes.** `cmd_run` (`cmd_cron.rs:684`) now probes the gateway daemon's flock before dispatching:

```rust
match gateway_daemon_holds_lock() {
    Ok(true) => {
        println!("{}", daemon_lock_refusal(id, &path));
        return Ok(());
    }
    Ok(false) => {}
    Err(e) => {
        eprintln!("Warning: could not read the gateway daemon lock ... proceeding ...");
    }
}
```

The probe (`:157`) opens `<operant home>/gateway.lock` and tries `flock(LOCK_EX | LOCK_NB)`. If it succeeds (nobody else holds it), the daemon is not running → `Ok(false)` → proceed. If it fails (someone holds it), the daemon is running → `Ok(true)` → refuse. The fd is dropped on return, releasing any lock the probe took.

The lock path comes from `cronjobs::lock::gateway_daemon_lock_path()` (`lock.rs:34`), which resolves through `platform::operant_home()` — the same resolver the daemon uses (`gateway_runner.rs:2434`). So the daemon and the probe cannot disagree about which file is the lock. Pinned by `the_probe_uses_the_core_lock_path_not_a_second_guess` (`:1126`).

### New gaps?

**TOCTOU (LOW).** Between the probe returning `Ok(false)` and the run actually dispatching, the daemon could start and acquire the lock. Both would then dispatch. The claim ledger (Z29) is the backstop: the second claim attempt for the same occurrence returns `Ok(0)` → skip. So the TOCTOU does not cause a double fire, only a wasted probe. The comment at `:708` acknowledges this: "The claim ledger (Z29) now also refuses a duplicate OCCURRENCE even across processes."

**Fail-open on lock-read error (intentional).** If the lock file cannot be read (filesystem error), the probe returns `Err`, and `cmd_run` proceeds with a warning (`:718`). The comment explains: refusing every manual run because of a filesystem error would be a worse outage than the race it prevents, and the claim ledger is still there. Reasonable tradeoff.

**Stale lock (no gap).** As above, `flock` is released by the kernel on process death. The file is never unlinked. Correct.

### Z31 verdict

**CLOSED. Low-severity TOCTOU only, mitigated by the Z29 claim ledger.** The probe is correct, the path is shared, the degradation is sound.

---

## Summary of required actions

1. **DELETE the claim that "a stale claim can never wedge a job"** (`scheduler.rs:2458` and the doc comment at `:421`). It is false.
2. **FIX the wedge:** `claim_stale_claims` (`db.rs:1171`) must advance `next_run_at` for each abandoned job (call `compute_next_run` and write it), OR `run_job` must treat a settled-abandoned claim as re-dispatchable. Without this, a single crash-before-dispatch permanently disables the job.
3. **FIX the test:** `an_abandoned_claim_is_skipped_not_retried_and_the_next_run_lives` (`:2461`) must NOT manually call `set_next_run`. It should assert that the next tick after abandonment fires the job *without* operator intervention. Revert the manual `set_next_run` line and the test will fail, proving the wedge.
4. **Z15 and Z31 need no code changes.** The blast-radius fallback is correct; the daemon lock probe is correct.

---

## Claims not verified (ASSERTED)

- **The lib test target compiles clean on clean HEAD.** I could not verify this because the working tree has a peer's in-flight `gateway/types.rs` change that breaks `harness_seams_r3.rs:448` (removed `handle_update` from the `PlatformAdapter` trait). This is a parallel-agent collision, not a defect in `ae9a27cf`. The blast_radius and cron tests are read-verified sound; I ran the blast_radius tests before the collision (46 passed).
- **`flock` release on daemon kill.** I read the code paths (process-global static Vec, fd dropped on process death) but did not kill a live daemon to confirm. This is standard POSIX `flock` semantics; ASSERTED.
- **`compute_next_run` never returns the same string as the old occurrence.** I read the implementation (`:1110`): interval phrases use `Utc::now() + period`, cron uses `upcoming(Utc).next()`. Both advance. The wedge test asserts `second_occurrence != first_occurrence`. ASSERTED but low-risk (the only way to violate it is a zero-length interval, which `interval_seconds` would reject).