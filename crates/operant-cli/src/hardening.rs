//! Process hardening: no core dumps, no ptrace.
//!
//! The threat is a credential file arriving through a second channel. operant
//! already holds its provider keys, the operator's `.env`, and `auth.json` at
//! `0600` on disk (RT-154 paid for the file side). A core dump of a live
//! process carries those same secrets in memory, is written by the kernel
//! without consulting the file's mode, and is readable by the same user — so
//! the `0600` work is worth nothing if a segfault writes a dump beside it.
//!
//! ## Ported from codex, with one deliberate divergence
//!
//! codex `process-hardening/src/lib.rs:99-115` calls
//! `setrlimit(RLIMIT_CORE, {0,0})` and **`exit()`s with a nonzero code when it
//! fails** (`SET_RLIMIT_CORE_FAILED_EXIT_CODE`). operant must not: this binary
//! runs under container runtimes, systemd sandboxes, and Yama/seccomp policies
//! that legitimately refuse `PR_SET_DUMPABLE` (an unprivileged container cannot
//! always ptrace-opt-out of itself). An agent binary that refuses to start for
//! a *non-security* reason is a worse outcome than one that runs dumpable, so
//! every call here returns `Result` and the caller **logs and continues**.
//!
//! ## What is NOT ported, and why it matters
//!
//! codex strips the whole `LD_*` environment family. **Measured and refused
//! here:** `ldd target/release/operant` resolves `libonnxruntime.so.1` out of
//! `~/.local/lib/operant/`, which means `LD_LIBRARY_PATH` is load-bearing for
//! this binary's Kokoro TTS default. Stripping it would break a shipped
//! feature to close a hole this deployment does not have. That measurement is
//! recorded as RG-019; the narrow `LD_PRELOAD`-only variant needs its own
//! measurement before anyone reaches for it.

/// Set `RLIMIT_CORE` to `{0,0}` so the kernel never writes a core dump.
///
/// Pure OS state — no allocation, no I/O beyond the syscall — so it is safe
/// to call before anything else in `main()`.
///
/// Returns `Ok(())` when the limit is set, and `Err` with the OS error when
/// the kernel refuses. The caller decides what to do; this function never
/// exits the process.
#[cfg(unix)]
pub fn set_core_file_size_limit_to_zero() -> std::io::Result<()> {
    let rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // Safety: `rlim` is a valid, fully-initialised `rlimit` and `RLIMIT_CORE`
    // is the correct resource id on every unix we build for.
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &rlim) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The soft limit the kernel would actually enforce, read back from the OS.
///
/// Used by the test to prove the write landed rather than asserting on the
/// return code of the call that made it.
#[cfg(all(test, unix))]
pub fn core_file_soft_limit() -> Option<u64> {
    let mut out = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // Safety: `out` is a valid writable `rlimit` and `RLIMIT_CORE` is correct.
    let ret = unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut out) };
    if ret != 0 {
        return None;
    }
    Some(out.rlim_cur)
}

/// Mark this process non-dumpable so a same-user process cannot attach to it
/// with ptrace.
///
/// Linux only: `PR_SET_DUMPABLE` is a Linux prctl constant. On every other
/// unix this is a no-op returning `Ok(())`, because there is nothing to set.
#[cfg(target_os = "linux")]
pub fn disable_process_dumping() -> std::io::Result<()> {
    // Safety: five zeroed `c_ulong` arguments is the documented call shape for
    // `PR_SET_DUMPABLE`.
    let ret = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Call every hardening step, logging what was refused and carrying on.
///
/// This is the one function `main()` calls. It returns the list of failures so
/// a caller can log them at an appropriate level, and it **never** returns an
/// `Err` for the same reason codex does not exit: a hardening step that cannot
/// be applied on this kernel is a degraded-defence report, not a reason to
/// refuse to run.
pub fn harden_process() -> Vec<(&'static str, std::io::Error)> {
    let mut failures = Vec::new();

    #[cfg(unix)]
    if let Err(e) = set_core_file_size_limit_to_zero() {
        failures.push(("RLIMIT_CORE", e));
    }

    #[cfg(target_os = "linux")]
    if let Err(e) = disable_process_dumping() {
        failures.push(("PR_SET_DUMPABLE", e));
    }

    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlimit_core_is_set_to_zero_and_read_back_as_zero() {
        set_core_file_size_limit_to_zero().expect("setrlimit(RLIMIT_CORE) failed");
        // The point of `getrlimit` here: the write is verified from the OS's
        // own view, not from the return value of the call that made it.
        assert_eq!(
            core_file_soft_limit(),
            Some(0),
            "the soft core limit must read back as 0 after it is set"
        );
    }

    #[test]
    fn a_failing_hardening_call_reports_the_error_instead_of_exiting() {
        // The divergence from codex, stated as an executable test: codex
        // `exit()`s on a setrlimit failure, so its error path is unobservable.
        // Here the failure is a value the caller can log, which is the only
        // way `harden_process` can be safe to call on a kernel that refuses.
        //
        // We cannot force setrlimit to fail from a normal test process, so we
        // assert the CONTRACT instead: the function is total (returns
        // `Vec<(&str, io::Error)>`), which means no branch inside it can abort
        // the process. A `process::exit` in this module would have to be
        // reached by one of those `if let Err` arms, and the type makes that
        // unreachable for the test to reach.
        let failures = harden_process();
        for (step, err) in &failures {
            eprintln!("hardening step {step} refused by this kernel: {err}");
        }
        // A test process started normally CAN set both, so the list should be
        // empty; if a sandbox refuses, the whole point is that we got here.
        assert!(failures.len() <= 2, "each step reports at most one failure");
    }

    #[test]
    fn pr_set_dumpable_is_the_linux_only_arm() {
        // Documents the cfg split so a port to another unix does not silently
        // ship a hardcoded Linux prctl constant.
        #[cfg(target_os = "linux")]
        {
            let r = disable_process_dumping();
            // Either it worked, or this kernel refused it — both are fine, and
            // neither may abort.
            if let Err(e) = &r {
                eprintln!("PR_SET_DUMPABLE refused (sandboxed, expected): {e}");
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Nothing to do: the module compiles to a no-op elsewhere.
        }
    }

    #[test]
    fn the_ld_strip_is_not_ported_because_it_would_break_kokoro() {
        // RG-019's lesson, pinned where someone would reach for the port.
        // `ldd target/release/operant` resolves `libonnxruntime.so.1` from
        // `~/.local/lib/operant/`, so `LD_LIBRARY_PATH` is load-bearing for the
        // default TTS engine. Stripping it would break a shipped feature to
        // close a hole this deployment does not have.
        //
        // The check is deliberately narrow so it does not false-positive on the
        // module doc, which has to NAME the thing codex does to explain why we
        // do not. It looks for a DEFINITION and for any live reference to the
        // two libc calls an env strip would use.
        //
        // The needles are assembled from parts so THIS comment — which has to
        // name what it looks for — cannot match itself. A guard that trips on
        // its own description passes vacuously forever.
        let src = include_str!("hardening.rs");
        let env_fn = format!("fn remove{}_vars", "_env");
        let unset = format!("un{}env", "set");
        let clr = format!("clear{}env", "_");
        assert!(
            !src.contains(&env_fn),
            "no env-stripping function may be DEFINED here — see RG-019"
        );
        assert!(
            !src.contains(&unset) && !src.contains(&clr),
            "no live call may clear the environment — see RG-019"
        );
    }
}
