//! Message copy utility — clipboard integration only.
//!
//! The copy_as_markdown / copy_as_plaintext / copy_code_blocks /
//! copy_as_json / copy_selection helpers were never called from any UI
//! flow and have been removed. When a "copy as markdown" menu item is
//! wired up, restore them from git history.

use std::io::Write;

/// Attempt to copy text to clipboard using platform CLI tools.
///
/// Tries, in order: Windows `clip.exe`, macOS `pbcopy`, Linux `wl-copy`
/// (Wayland), `xclip` (X11), `xsel` (X11). Returns `true` on success.
pub fn copy_to_clipboard(text: &str) -> bool {
    // Windows
    #[cfg(target_os = "windows")]
    {
        // Call clip.exe directly (not through cmd.exe) for reliability in raw terminal mode.
        if let Ok(mut child) = std::process::Command::new("clip")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(_e) = stdin.write_all(text.as_bytes()) {
                    return false;
                }
                drop(stdin);
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    return true;
                }
            }
        }
    }

    // macOS
    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(_e) = stdin.write_all(text.as_bytes()) {
                    return false;
                }
                drop(stdin);
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    return true;
                }
            }
        }
    }

    // Linux: Wayland first, then X11
    #[cfg(target_os = "linux")]
    {
        // wl-copy (Wayland)
        if let Ok(mut child) = std::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            && let Some(mut stdin) = child.stdin.take()
        {
            if let Err(_e) = stdin.write_all(text.as_bytes()) {
                // fall through to xclip/xsel
            } else {
                drop(stdin);
                if let Ok(status) = child.wait()
                    && status.success()
                {
                    return true;
                }
            }
        }

        // xclip (X11)
        if let Ok(mut child) = std::process::Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(_e) = stdin.write_all(text.as_bytes()) {
                    return false;
                }
                drop(stdin);
            }
            if let Ok(status) = child.wait()
                && status.success()
            {
                return true;
            }
        }

        // xsel (X11 fallback)
        if let Ok(mut child) = std::process::Command::new("xsel")
            .arg("--clipboard")
            .arg("--input")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(_e) = stdin.write_all(text.as_bytes()) {
                    return false;
                }
                drop(stdin);
            }
            if let Ok(status) = child.wait()
                && status.success()
            {
                return true;
            }
        }
    }

    false
}
