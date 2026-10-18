// Vendored from jcode (crates/operant-base/src/platform.rs:249), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported at batch-4 sweep tail — the PID
// liveness check the session crash detector consumes. Unix arm verbatim;
// the upstream windows arm (windows_sys) stays unported (operant-cli does
// not depend on windows-sys; add it with the arm if a Windows build is
// ever targeted).
