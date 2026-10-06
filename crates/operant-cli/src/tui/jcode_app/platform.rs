// Vendored from jcode (crates/jcode-base/src/platform.rs:249), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported at batch-4 sweep tail — the PID
// liveness check the session crash detector consumes. Unix arm verbatim;
// the upstream windows arm (windows_sys) stays unported (operant-cli does
// not depend on windows-sys; add it with the arm if a Windows build is
// ever targeted).

/// Whether a process with the given PID is running.
pub fn is_process_running(pid: u32) -> bool {
    // #[cfg(windows)] arm: upstream jcode-base/src/platform.rs:259-280
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as i32, 0) };
        if result == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        !matches!(err.raw_os_error(), Some(code) if code == libc::ESRCH)
    }
    #[cfg(not(unix))]
    {
        false
    }
}
