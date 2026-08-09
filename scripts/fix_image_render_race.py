#!/usr/bin/env python3
"""Fix the env-var race between the kitty/iterm protocol detection tests.

Both tests mutate process-global env vars and run in parallel on cargo's
test threads. The kitty test sets TERM=xterm-kitty; if the iterm test asserts
while that is set, detect_graphics_protocol() returns Kitty (TERM is checked
first). Serialize both tests with a shared Mutex.
"""


def patch(fp: str, old: str, new: str, label: str) -> None:
    with open(fp) as f:
        c = f.read()
    assert old in c, f"{label}: pattern not found in {fp}"
    c = c.replace(old, new, 1)
    with open(fp, "w") as f:
        f.write(c)
    print(f"OK   {label}: patched {fp}")


fp = "crates/operant-cli/src/tui/image_render.rs"

# 1. Add a shared env mutex in the tests module
patch(
    fp,
    """#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_protocol_kitty_env() {
        unsafe {
            std::env::set_var("TERM", "xterm-kitty");
        }
        assert_eq!(detect_graphics_protocol(), GraphicsProtocol::Kitty);
        unsafe {
            std::env::remove_var("TERM");
        }
    }""",
    """#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes env-var-mutating tests: they run on parallel threads and
    /// share process-global environment, so they must not interleave.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_detect_protocol_kitty_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TERM", "xterm-kitty");
        }
        assert_eq!(detect_graphics_protocol(), GraphicsProtocol::Kitty);
        unsafe {
            std::env::remove_var("TERM");
        }
    }""",
    "kitty test lock",
)

# 2. Lock the iterm test too
patch(
    fp,
    """    #[test]
    fn test_detect_protocol_iterm_env() {
        unsafe {
            std::env::set_var("TERM_PROGRAM", "iTerm.app");
        }
        assert_eq!(detect_graphics_protocol(), GraphicsProtocol::ITerm2);
        unsafe {
            std::env::remove_var("TERM_PROGRAM");
        }
    }""",
    """    #[test]
    fn test_detect_protocol_iterm_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("TERM_PROGRAM", "iTerm.app");
        }
        assert_eq!(detect_graphics_protocol(), GraphicsProtocol::ITerm2);
        unsafe {
            std::env::remove_var("TERM_PROGRAM");
        }
    }""",
    "iterm test lock",
)
