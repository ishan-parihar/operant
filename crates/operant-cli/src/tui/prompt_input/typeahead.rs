//! Typeahead / autocomplete suggestions.

#[derive(Debug, Clone, PartialEq)]
pub enum TypeaheadSource {
    SlashCommand,
    FileRef,
}

/// Outcome of accepting a suggestion during an Enter (submit) keypress.
///
/// Differentiates between:
/// - `ExtendInput`: keep editing (e.g. file-ref expansion that wants a trailing space)
/// - `Submit`: the suggestion consumed the slash-command so Enter now submits
/// - `NoSuggestion`: caller should fall back to the normal submit path
#[derive(Debug, PartialEq, Eq)]
pub enum AcceptForSubmitOutcome {
    ExtendInput,
    Submit,
    NoSuggestion,
}

/// A single typeahead suggestion.
#[derive(Debug, Clone)]
pub struct TypeaheadSuggestion {
    pub text: String,
    pub description: String,
    pub source: TypeaheadSource,
}

/// Compute typeahead suggestions for the current input.
///
/// Handles two kinds of suggestions:
/// - `/` slash commands (e.g. `/help`, `/clear`)
/// - `@` file references (e.g. `@src/`, `@~/Documents/`)
pub fn compute_typeahead(
    input: &str,
    slash_commands: &[(&str, &str)],
    file_autocomplete_limit: usize,
    file_autocomplete_show_hidden: bool,
) -> Vec<TypeaheadSuggestion> {
    // Handle slash commands: /help, /clear, etc.
    if input.starts_with('/') {
        // /skill <prefix> and /bundle <prefix> expand to installed skill /
        // bundle names (hermes parity — the model-facing expansion takes a
        // name, so completing the name is the useful step).
        if let Some(rest) = input.strip_prefix("/skill ") {
            return compute_name_completions("skill", rest, TypeaheadSource::SlashCommand);
        }
        if let Some(rest) = input.strip_prefix("/bundle ") {
            return compute_name_completions("bundle", rest, TypeaheadSource::SlashCommand);
        }
        return compute_slash_suggestions(input, slash_commands);
    }

    // Handle file references: @, @/, @~/, @src/, etc.
    compute_file_suggestions(
        input,
        file_autocomplete_limit,
        file_autocomplete_show_hidden,
    )
}

/// Compute typeahead suggestions for slash commands only (e.g., `/help`).
///
/// Ordering rules (iter-125 — smart slash-command ordering):
///   1. Exact prefix matches ranked by recency (most-recently-used first),
///      then by frequency, then by declaration order.
///   2. When the user has typed just `/` (empty prefix), recently-used
///      commands float to the top — closes the user-reported "smart ordering
///      of slash commands rather than generic-ordering" request.
///
/// We pull usage stats from the global `UsageStore` on every call (cheap —
/// it's a small HashMap deserialized from `~/.operant/slash-usage.json`,
/// cached in `App`).
pub(super) fn compute_slash_suggestions(
    input: &str,
    slash_commands: &[(&str, &str)],
) -> Vec<TypeaheadSuggestion> {
    let Some(cmd_prefix) = input.strip_prefix('/') else {
        return Vec::new();
    };
    let prefix_lower = cmd_prefix.to_lowercase();

    // Filter to prefix matches first.
    let mut matching: Vec<usize> = (0..slash_commands.len())
        .filter(|&i| {
            slash_commands[i]
                .0
                .to_lowercase()
                .starts_with(&prefix_lower)
        })
        .collect();
    if matching.is_empty() {
        return Vec::new();
    }

    // Apply smart ordering: recency → frequency → declaration order.
    // We pull the usage store lazily so we don't re-read the file per
    // keystroke (App caches it in `slash_usage` and calls us via
    // `update_suggestions` which has access to that cache — but for
    // standalone callers we fall back to a fresh load).
    let usage = crate::tui::slash_usage::UsageStore::load();
    matching.sort_by(|&a, &b| {
        let ra = usage.recency_rank(slash_commands[a].0);
        let rb = usage.recency_rank(slash_commands[b].0);
        ra.cmp(&rb)
            .then_with(|| {
                let fa = usage.frequency_rank(slash_commands[a].0);
                let fb = usage.frequency_rank(slash_commands[b].0);
                fb.cmp(&fa)
            })
            .then_with(|| a.cmp(&b))
    });

    matching
        .into_iter()
        .map(|i| TypeaheadSuggestion {
            text: format!("/{}", slash_commands[i].0),
            description: slash_commands[i].1.to_string(),
            source: TypeaheadSource::SlashCommand,
        })
        .collect()
}

/// Process-wide snapshot of installed skill + bundle names for `/skill <Tab>`
/// and `/bundle <Tab>` typeahead. The app (re)registers it whenever the
/// skills directory is scanned; the pure prompt-input layer only reads it.
static SKILL_NAME_SNAPSHOT: std::sync::OnceLock<std::sync::RwLock<Vec<String>>> =
    std::sync::OnceLock::new();

/// Serializes snapshot writers. Parallel tests construct `App` (which
/// re-registers the real installed skills) while the typeahead test asserts
/// on its own registration — without this mutex the process-wide snapshot is
/// replaced mid-assertion and the test flakes. Production impact is nil:
/// registration happens only at app init / skills rescans.
pub(super) static SKILL_SNAPSHOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Compute `/skill <name>` / `/bundle <name>` name completions from the
/// installed-skills snapshot registered by the app.
fn compute_name_completions(
    command: &str,
    prefix: &str,
    source: TypeaheadSource,
) -> Vec<TypeaheadSuggestion> {
    let names = SKILL_NAME_SNAPSHOT.get_or_init(|| std::sync::RwLock::new(Vec::new()));
    let names = names.read().unwrap_or_else(|e| e.into_inner());

    let prefix_lower = prefix.trim().to_lowercase();
    names
        .iter()
        .filter(|n| {
            prefix_lower.is_empty()
                || n.to_lowercase().starts_with(&prefix_lower)
                || n.to_lowercase().contains(&prefix_lower)
        })
        .take(20)
        .map(|n| TypeaheadSuggestion {
            text: format!("/{} {}", command, n),
            description: format!("Expand {} '{}' into the turn", command, n),
            source: source.clone(),
        })
        .collect()
}

/// Register the installed skill + bundle names for `/skill <Tab>` typeahead.
/// Called by the app whenever the skills directory is (re)scanned.
pub fn register_typeahead_names(skill_names: Vec<String>, bundle_names: Vec<String>) {
    let _guard = SKILL_SNAPSHOT_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    set_typeahead_names(skill_names, bundle_names);
}

/// Lock-free inner writer used by `register_typeahead_names` and by the
/// typeahead test (which holds `SKILL_SNAPSHOT_LOCK` across its assertions).
pub(super) fn set_typeahead_names(skill_names: Vec<String>, bundle_names: Vec<String>) {
    let names = SKILL_NAME_SNAPSHOT.get_or_init(|| std::sync::RwLock::new(Vec::new()));
    let mut names = names.write().unwrap_or_else(|e| e.into_inner());
    names.clear();
    names.extend(skill_names);
    names.extend(bundle_names);
}

/// Compute typeahead suggestions for file references (e.g., `@src/main.rs`).
pub(super) fn compute_file_suggestions(
    input: &str,
    file_autocomplete_limit: usize,
    file_autocomplete_show_hidden: bool,
) -> Vec<TypeaheadSuggestion> {
    let mut suggestions = Vec::new();

    if let Some(at_idx) = input.rfind('@') {
        // Only suggest files if @ is at a word boundary (preceded by whitespace or start of string)
        let at_word_boundary = at_idx == 0
            || input[..at_idx]
                .chars()
                .last()
                .map(|c| c.is_whitespace())
                .unwrap_or(false);

        if at_word_boundary {
            let file_prefix = &input[at_idx + 1..];
            suggestions = suggest_files(
                file_prefix,
                file_autocomplete_limit,
                file_autocomplete_show_hidden,
            );
        }
    }

    suggestions
}

/// Suggest files matching a path prefix.
///
/// Examples:
/// - `""` → files in cwd with names only (e.g., ["main.rs", "lib.rs"])
/// - `"src"` → suggest "src/" if it exists
/// - `"src/"` → files in src/ with names only (e.g., ["main.rs", "lib.rs"])
/// - `"/"` → files in root with full paths (e.g., ["/Users", "/Applications"])
/// - `"~"` → suggest "~/" if it exists
/// - `"~/"` → files in home with names only
///
/// Note: calls `fs::read_dir` synchronously on every invocation; may stall on slow/network
/// filesystems. Consider debouncing at the call site if this becomes a problem.
fn suggest_files(
    prefix: &str,
    max_suggestions: usize,
    show_hidden: bool,
) -> Vec<TypeaheadSuggestion> {
    use std::fs;
    use std::path::PathBuf;

    let mut suggestions = Vec::new();

    // Determine the directory to list and whether to show full paths
    let (search_dir, show_full_paths, partial_name) = if prefix.is_empty() {
        // Just @, show files from cwd
        if let Ok(cwd) = std::env::current_dir() {
            (cwd, false, String::new())
        } else {
            return suggestions;
        }
    } else if prefix.starts_with('/') || prefix.starts_with('~') {
        // Absolute or home path: show full paths
        let expanded = if prefix.starts_with('~') {
            prefix.replacen('~', &home_dir().unwrap_or_default(), 1)
        } else {
            prefix.to_string()
        };

        let path = PathBuf::from(&expanded);
        if path.is_dir() && prefix.ends_with('/') {
            // User typed a complete directory with trailing slash: list its contents
            (path, true, String::new())
        } else if let Some(parent) = path.parent() {
            // User typed a partial path or directory without slash: list parent's contents and filter
            let partial = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            (parent.to_path_buf(), true, partial)
        } else {
            return suggestions;
        }
    } else {
        // Relative path in cwd
        if let Ok(cwd) = std::env::current_dir() {
            let path = cwd.join(prefix);
            if path.is_dir() && prefix.ends_with('/') {
                // Complete directory with trailing slash: list its contents
                (path, false, String::new())
            } else if let Some(parent) = path.parent() {
                // Partial path or directory without slash: list parent and filter
                let partial = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                (parent.to_path_buf(), false, partial)
            } else {
                return suggestions;
            }
        } else {
            return suggestions;
        }
    };

    // List files in the directory
    if let Ok(entries) = fs::read_dir(&search_dir) {
        let mut files: Vec<_> = entries
            .filter_map(|e| {
                e.ok().and_then(|entry| {
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())?;

                    // Filter by partial name (case-insensitive)
                    if !partial_name.is_empty()
                        && !name
                            .to_lowercase()
                            .starts_with(&partial_name.to_lowercase())
                    {
                        return None;
                    }

                    // Filter hidden files unless user explicitly types a dot or show_hidden_files is enabled
                    if !show_hidden
                        && name.starts_with('.')
                        && !partial_name.to_lowercase().starts_with('.')
                    {
                        return None;
                    }

                    // Detect if this is a symlink or junction link
                    let is_symlink = entry
                        .file_type()
                        .ok()
                        .map(|ft| ft.is_symlink())
                        .unwrap_or(false);
                    let is_dir = path.is_dir();

                    Some((name, is_dir, is_symlink, path))
                })
            })
            .collect();

        files.sort_by(|a, b| {
            // Directories first, then alphabetically
            match (b.1, a.1) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => a.0.cmp(&b.0),
            }
        });

        for (name, is_dir, is_symlink, full_path) in files {
            if suggestions.len() >= max_suggestions {
                break;
            }

            if is_dir && !dir_has_visible_contents(&full_path, show_hidden) {
                continue;
            }

            let is_listing_mode = prefix.ends_with('/');
            let suggestion_text = if show_full_paths {
                let full = search_dir.join(&name);
                full.to_string_lossy().to_string() + if is_dir { "/" } else { "" }
            } else if is_listing_mode {
                // When listing a directory's contents, prepend the full prefix path
                format!("{}{}{}", prefix, name, if is_dir { "/" } else { "" })
            } else if !partial_name.is_empty() && prefix.ends_with(&partial_name) {
                // When filtering in a subdirectory, prepend the parent path
                let parent_path = &prefix[..prefix.len() - partial_name.len()];
                format!("{}{}{}", parent_path, name, if is_dir { "/" } else { "" })
            } else {
                // Fallback: just use the matched filename
                name.clone() + if is_dir { "/" } else { "" }
            };

            let description = if is_symlink {
                if is_dir {
                    "directory link".to_string()
                } else {
                    "file link".to_string()
                }
            } else if is_dir {
                "directory".to_string()
            } else {
                "file".to_string()
            };

            suggestions.push(TypeaheadSuggestion {
                text: format!("@{}", suggestion_text),
                description,
                source: TypeaheadSource::FileRef,
            });
        }
    }

    suggestions
}

/// Returns true if `dir` contains at least one visible entry.
/// When `show_hidden` is false, dotfiles are not counted as visible.
fn dir_has_visible_contents(dir: &std::path::Path, show_hidden: bool) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|e| e.ok()).any(|entry| {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            show_hidden || !name_str.starts_with('.')
        }),
        Err(_) => false,
    }
}

/// Get the home directory path.
fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
}
