//! Compose the prompt in an external editor (Ctrl+G, oh-my-pi
//! `app.editor.external`). The composer text goes to a temp file, the
//! editor runs on the real terminal while the TUI is suspended, and the
//! saved file becomes the composer text.

use std::path::PathBuf;

/// The editor command: `[tui] external_editor`, else `$VISUAL`, else
/// `$EDITOR`, else `vi`. Whitespace-separated, so `"code --wait"` works.
pub fn resolve_command(configured: &str) -> Vec<String> {
    let pick = [
        Some(configured.to_string()),
        std::env::var("VISUAL").ok(),
        std::env::var("EDITOR").ok(),
    ]
    .into_iter()
    .flatten()
    .find(|cmd| !cmd.trim().is_empty())
    .unwrap_or_else(|| "vi".to_string());
    pick.split_whitespace().map(str::to_string).collect()
}

/// Run `command` on a temp file seeded with `text`; returns the saved text
/// (one trailing newline, which editors add, is dropped). The caller owns
/// suspending and restoring the terminal.
pub fn edit(command: &[String], text: &str) -> std::io::Result<String> {
    let Some((program, args)) = command.split_first() else {
        return Err(std::io::Error::other("no editor command"));
    };
    let path = temp_path();
    std::fs::write(&path, text)?;
    let status = std::process::Command::new(program)
        .args(args)
        .arg(&path)
        .status();
    let result = match status {
        Ok(status) if status.success() => std::fs::read_to_string(&path),
        Ok(status) => Err(std::io::Error::other(format!(
            "{program} exited with {status}"
        ))),
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&path);
    let mut edited = result?;
    if edited.ends_with('\n') {
        edited.pop();
        if edited.ends_with('\r') {
            edited.pop();
        }
    }
    Ok(edited)
}

fn temp_path() -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "operant-prompt-{}-{}.md",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_editor_wins_and_splits_arguments() {
        assert_eq!(resolve_command("code --wait"), ["code", "--wait"]);
    }

    #[test]
    fn edit_round_trips_through_the_editor() {
        // `sh -c 'printf edited > "$0"'` stands in for an editor: it
        // overwrites the file passed as its last argument.
        let command: Vec<String> = ["sh", "-c", "printf 'edited text\\n' > \"$0\""]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(edit(&command, "original").unwrap(), "edited text");
        // A failing editor keeps the caller's text (error, not empty).
        let failing: Vec<String> = ["sh", "-c", "exit 3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(edit(&failing, "original").is_err());
    }
}
