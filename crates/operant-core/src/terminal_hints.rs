//! Plan 010 — terminal command-failure hints (hermes `terminal_hints.py` parity).
//!
//! `annotate_failure` returns an actionable hint for common shell errors.
//! Used by the terminal tool to append `💡 Try: ...` to a failed command's
//! result so the model can self-correct without a human turn.

/// Annotate a command's failure with an actionable hint.
///
/// Returns `Some(hint)` when one of the heuristics fires, `None` otherwise.
/// The 7 heuristics match hermes `terminal_hints.py` (line ~60–165):
/// 1. `gh` unknown json field
/// 2. command not found
/// 3. module not found (Python)
/// 4. `cargo: command not found` (rust toolchain)
/// 5. `cd: no such file or directory`
/// 6. `npm: command not found`
/// 7. `git: not a git repository`
pub fn annotate_failure(command: &str, exit_code: Option<i32>, output: &str) -> Option<String> {
    let cmd = command.trim();
    let lc = cmd.to_ascii_lowercase();
    let out_lc = output.to_ascii_lowercase();

    // 1. gh unknown json field
    if lc.starts_with("gh ") && out_lc.contains("unknown json field") {
        return Some(
            "💡 Try: check `gh --version` and update the CLI; the json field is deprecated in this version.".to_string(),
        );
    }

    // 2. command not found
    if out_lc.contains("command not found") {
        if out_lc.contains("cargo") {
            return Some(
                "💡 Try: install rust via `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh` and source $HOME/.cargo/env.".to_string(),
            );
        }
        if out_lc.contains("npm") {
            return Some("💡 Try: install Node.js (e.g. `nvm install --lts` or your distro's package manager).".to_string());
        }
        if out_lc.contains("gh") {
            return Some(
                "💡 Try: install the GitHub CLI (`https://cli.github.com/`) and run `gh auth login`.".to_string(),
            );
        }
        return Some("💡 Try: install the missing command via your package manager.".to_string());
    }

    // 3. module not found (Python)
    if (out_lc.contains("modulenotfounderror") || out_lc.contains("no module named"))
        && let Some(modname) = extract_missing_module(output)
    {
        return Some(format!("💡 Try: `pip install {modname}` (or add to your requirements/pyproject)."));
    }

    // 5. cd: no such file or directory
    if (lc.starts_with("cd ") || out_lc.starts_with("cd:"))
        && out_lc.contains("no such file or directory")
    {
        return Some(
            "💡 Try: check the path with `ls` and `pwd`. Quoted paths with spaces must be exact.".to_string(),
        );
    }

    // 7. not a git repository
    if out_lc.contains("not a git repository")
        || (lc.starts_with("git ") && out_lc.contains("fatal: not a git repository"))
    {
        return Some(
            "💡 Try: `git init` first, or pass the repo path with `--path` / a positional argument.".to_string(),
        );
    }

    // 4. cargo: command not found (already covered by 2; explicit secondary
    // signal for the case where the SHELL does not print "command not
    // found" verbatim — e.g. inside a build script).
    if exit_code == Some(101) && out_lc.contains("error") && out_lc.contains("compilation") {
        return Some(
            "💡 Try: read the compiler error above — most cargo 101 errors are fixable from the message.".to_string(),
        );
    }

    None
}

fn extract_missing_module(stderr: &str) -> Option<String> {
    // Python's ModuleNotFoundError: "No module named 'X'"
    for line in stderr.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(idx) = lower.find("no module named '") {
            let rest = &line[idx + "no module named '".len()..];
            if let Some(end) = rest.find('\'') {
                return Some(rest[..end].to_string());
            }
        }
        if let Some(idx) = lower.find("modulenotfounderror:") {
            // ModuleNotFoundError: No module named 'X'
            let rest = &line[idx + "modulenotfounderror:".len()..];
            return extract_missing_module(rest);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_json_field_hint() {
        let hint = annotate_failure("gh pr list --json old", None, "gh: Unknown JSON field: old");
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("update the CLI"));
    }

    #[test]
    fn command_not_found_cargo() {
        let hint = annotate_failure("cargo build", Some(127), "cargo: command not found");
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("rustup"));
    }

    #[test]
    fn command_not_found_npm() {
        let hint = annotate_failure("npm install", Some(127), "npm: command not found");
        assert!(hint.is_some());
        assert!(hint.unwrap().to_lowercase().contains("node"));
    }

    #[test]
    fn module_not_found_python() {
        let hint = annotate_failure(
            "python3 -c 'import requests'",
            Some(1),
            "ModuleNotFoundError: No module named 'requests'",
        );
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("pip install requests"));
    }

    #[test]
    fn cd_no_such_file() {
        let hint = annotate_failure("cd /nope", Some(1), "cd: /nope: No such file or directory");
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("ls"));
    }

    #[test]
    fn not_a_git_repo() {
        let hint = annotate_failure("git status", Some(128), "fatal: not a git repository (or any parent up to mount point /)");
        assert!(hint.is_some());
        assert!(hint.unwrap().contains("git init"));
    }

    #[test]
    fn no_hint_for_silent_success() {
        let hint = annotate_failure("echo ok", Some(0), "ok");
        assert!(hint.is_none());
    }

    #[test]
    fn no_hint_for_unknown_error() {
        let hint = annotate_failure("weird-cmd", Some(2), "some random output");
        assert!(hint.is_none());
    }
}
