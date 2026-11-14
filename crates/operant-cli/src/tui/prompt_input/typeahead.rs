//! Typeahead / autocomplete suggestions.

use crate::tui::slash_usage::{UsageStore, rank_commands};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeaheadSource {
    SlashCommand,
    FileRef,
    /// A `$skill` invocation mention (codex `$`-tier parity). Unlike a slash
    /// command it lives MID-SENTENCE, so accepting it must extend the buffer
    /// rather than submit it.
    SkillRef,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeaheadSuggestion {
    /// The INSERTED token on accept (the `@file:`/`@folder:` grammar the
    /// agent-side expander reads).
    pub text: String,
    /// The DISPLAYED row label (2026-10-27, OMP `AutocompleteItem.label`
    /// parity): the bare path — no `@`, no kind prefix. Defaults to `text`
    /// for slash-command rows, whose display IS the command token.
    pub display: String,
    pub description: String,
    pub source: TypeaheadSource,
}

impl TypeaheadSuggestion {
    /// A slash-command row: the display IS the token (`/help`).
    pub fn command(text: String, description: String) -> Self {
        Self {
            display: text.clone(),
            text,
            description,
            source: TypeaheadSource::SlashCommand,
        }
    }

    /// A file-ref row: `text` carries the insertion grammar, `display`
    /// the bare OMP-style label.
    pub fn file_ref(text: String, display: String, description: String) -> Self {
        Self {
            text,
            display,
            description,
            source: TypeaheadSource::FileRef,
        }
    }

    /// A `$skill` mention row: `text` is the token the agent-side expander
    /// reads (`$<name>`), `display` the same token shown in the popup.
    pub fn skill_ref(text: String, description: String) -> Self {
        Self {
            display: text.clone(),
            text,
            description,
            source: TypeaheadSource::SkillRef,
        }
    }
}

/// A fuzzy-tier request the pure prompt-input layer hands to the App
/// (plan 2026-10-24 M3). The state layer cannot spawn tasks; it only
/// describes what to rank. The App's answer is applied only while the
/// `@` token is still exactly [`PendingFileQuery::token`].
#[derive(Debug, Clone)]
pub struct PendingFileQuery {
    /// The full live `@` token (including the `@`) the answer must still
    /// match on arrival, else the result is dropped.
    pub token: String,
    /// The fuzzy query: the token's tail after the last `/` (the leading
    /// directory part rides the result paths; the tail is what ranks).
    pub query: String,
}

/// One fuzzy walk job for the blocking pool (App-side, M3).
#[derive(Debug, Clone)]
pub struct FileSuggestJob {
    pub token: String,
    pub query: String,
    pub root: std::path::PathBuf,
    pub include_hidden: bool,
}

/// The walk's answer, tagged with its generation and source token (M3).
pub struct FileSuggestResult {
    pub generation: u64,
    pub token: String,
    pub suggestions: Vec<TypeaheadSuggestion>,
}

/// Map ranked walk entries to popup suggestions (M3). Values use the
/// reference grammar (M4): `@file:<path>` / `@folder:<path>/`.
pub fn fuzzy_file_suggestions(
    entries: &[crate::tui::file_discovery::ScoredEntry],
) -> Vec<TypeaheadSuggestion> {
    entries
        .iter()
        .map(|e| {
            TypeaheadSuggestion::file_ref(
                file_ref_value(&e.rel_path, e.is_dir),
                file_ref_display_label(&e.rel_path, e.is_dir),
                if e.is_dir { "directory" } else { "file" }.to_string(),
            )
        })
        .collect()
}

/// A parsed `@` mention token (plan 2026-10-24 M4). The reference grammar
/// the agent-side expander reads is `@file:` / `@folder:`
/// (`operant_core::context_references::REFERENCE_PATTERN`); the popup
/// triggers on bare `@`, `@file:`, and `@folder:` alike and inserts the
/// grammar when a candidate is accepted — the kind comes from the accepted
/// entry, not the typed prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileRefToken<'a> {
    /// The raw token from the `@` to the end of `input`.
    pub raw: &'a str,
    /// The path part: after the optional `file:`/`folder:` kind prefix and
    /// an optional opening quote.
    pub path: &'a str,
}

/// Parse the live `@` mention token ending at the end of `input`, or
/// `None` when there is no `@` at a word boundary.
pub fn parse_file_ref_token(input: &str) -> Option<FileRefToken<'_>> {
    let at_idx = input.rfind('@')?;
    let at_word_boundary = at_idx == 0
        || input[..at_idx]
            .chars()
            .last()
            .is_some_and(char::is_whitespace);
    if !at_word_boundary {
        return None;
    }
    let tail = &input[at_idx..];
    let after_at = &tail[1..];
    // Terminate the token where the expander's grammar does —
    // `@kind:(quoted|\S+)` (`context_references::REFERENCE_PATTERN`): a
    // bare value ends at the first whitespace, a quoted value at its
    // closing quote (an open quote runs to the end of the input while it
    // stays open). The popup token is what the user is STILL typing, so it
    // must also reach the end of `input`: a terminated token that ends
    // earlier means the mention is complete and the cursor is in ordinary
    // prose. Without this, every message that ever contained an `@` kept a
    // token alive for its whole remaining length — the ghost gate
    // suppressed completions from the mention onward, every later
    // keystroke spawned a fuzzy walk, and a space-free tail could reopen
    // the popup mid-prose where Enter accepts a file instead of
    // submitting.
    let stripped_kind = after_at
        .strip_prefix("file:")
        .or_else(|| after_at.strip_prefix("folder:"))
        .unwrap_or(after_at);
    let (value_end, quote_closed) = if let Some(quoted_value) = stripped_kind.strip_prefix('"') {
        match quoted_value.find('"') {
            // +2: skip the opening quote and include the closing one.
            Some(i) => (i + 2, true),
            None => (stripped_kind.len(), false),
        }
    } else {
        (
            stripped_kind
                .find(char::is_whitespace)
                .unwrap_or(stripped_kind.len()),
            false,
        )
    };
    if value_end < stripped_kind.len() {
        // The mention is closed and the cursor moved on — no live token.
        return None;
    }
    let raw = tail;
    let quoted = stripped_kind.starts_with('"');
    let path = &stripped_kind[..value_end];
    let path = if quoted {
        // Strip the opening quote, and the closing one when the value
        // terminated at it (an open quote runs to the end of the input,
        // so there is nothing to strip there).
        let inner = &path[1..];
        if quote_closed {
            &inner[..inner.len() - 1]
        } else {
            inner
        }
    } else {
        path
    };
    Some(FileRefToken { raw, path })
}

/// The live `@` token at a word boundary ending at the end of `input`
/// (`@src/ma` → the full `@src/ma`), or `None` (M3; mirrors the boundary
/// rule of `compute_file_suggestions`).
pub fn active_file_token(input: &str) -> Option<&str> {
    parse_file_ref_token(input).map(|t| t.raw)
}

/// The inserted token for one accepted entry (M4): files complete as
/// `@file:<path>` (the accept path appends the trailing space),
/// directories as `@folder:<path>/` — no trailing space, so the popup
/// chains into the drilled directory. Paths with spaces quote the value
/// (`@file:"a b"`); a quoted DIRECTORY stays open (`@folder:"a b/`) and
/// closes when a file inside it is accepted, exactly OMP's
/// `buildCompletionValue` shape.
///
/// (2026-10-27) `path` may already end with `/` (the directory listing
/// builds `name/`); the old unconditional append produced the doubled
/// `@folder:crates//` the operator reported — on screen AND in the
/// accepted token, which then failed to round-trip through the
/// agent-side expander's path resolution.
pub fn file_ref_value(path: &str, is_dir: bool) -> String {
    let kind = if is_dir { "@folder:" } else { "@file:" };
    // OMP's `isDirectoryCompletionValue`: the separator decides chaining,
    // never append a second one. The caller's `name/` listing form and a
    // bare `name` both normalize to exactly one trailing separator.
    let path = path.trim_end_matches('/');
    if is_dir {
        // A directory always ends the separator (chain into the drilled
        // dir); quoted values keep it INSIDE the open quote (M4: a quoted
        // DIRECTORY stays open until a file inside closes it).
        if path.contains(char::is_whitespace) {
            format!("{kind}\"{path}/")
        } else {
            format!("{kind}{path}/")
        }
    } else if path.contains(char::is_whitespace) {
        format!("{kind}\"{path}\"")
    } else {
        format!("{kind}{path}")
    }
}

/// The DISPLAY label for one suggestion (2026-10-27, OMP parity): OMP's
/// `AutocompleteItem` splits `value` (the inserted token, `@file:` grammar
/// included) from `label` (what the row SHOWS — the bare `name/` for a
/// directory, `name` for a file, no `@`, no kind prefix). The operator's
/// report: `@folder:`/`@file:` prefixes eat reading space. The
/// file/directory distinction is already the description column's job
/// ("directory"/"file"), matching OMP's iconName column.
pub fn file_ref_display_label(path: &str, is_dir: bool) -> String {
    let path = path.trim_end_matches('/');
    if is_dir {
        format!("{path}/")
    } else {
        path.to_string()
    }
}

/// The byte range of the slash-command completion token under the caret:
/// the whitespace-delimited `/…` word that holds the last `/` before the
/// caret.
///
/// The token is whitespace-delimited rather than "the text starts with `/`",
/// so a command typed after prose completes too (`look at /rev`) and the
/// accept splices exactly this token — the text in front of it survives.
///
/// A command name holds exactly one `/`; a word with a second one is a path
/// (`/usr/bin`), and a path must never open the command popup.
pub fn slash_token_range(before_cursor: &str) -> Option<(usize, usize)> {
    let slash = before_cursor.rfind('/')?;
    let start = before_cursor[..slash]
        .rfind(char::is_whitespace)
        .map_or(0, |i| i + 1);
    let word = before_cursor[start..]
        .split_whitespace()
        .next()
        .unwrap_or_default();
    if !word.starts_with('/') || word[1..].contains('/') {
        return None;
    }
    Some((start, start + word.len()))
}

/// The fuzzy-tier request for the current input, or `None` when the popup
/// is not in an `@` file token or the tail after the last `/` is empty
/// (pure listing mode — the synchronous immediate listing suffices).
pub fn active_file_query(input: &str) -> Option<PendingFileQuery> {
    if input.trim_start().starts_with('/') || slash_token_range(input).is_some() {
        return None; // slash-command input: no file tier
    }
    let token = parse_file_ref_token(input)?;
    let query = token.path.rsplit('/').next().unwrap_or(token.path);
    if query.is_empty() {
        return None;
    }
    Some(PendingFileQuery {
        token: token.raw.to_string(),
        query: query.to_string(),
    })
}

/// Compute typeahead suggestions for the current input.
///
/// Handles three kinds of suggestions:
/// - `/` slash commands (e.g. `/help`, `/clear`)
/// - `@` file references (e.g. `@src/`, `@~/Documents/`)
/// - `$` skill mentions (e.g. `apply $ponytail to this`), mid-sentence
pub fn compute_typeahead(
    input: &str,
    slash_commands: &[(&str, &str)],
    usage: &UsageStore,
    file_autocomplete_limit: usize,
    file_autocomplete_show_hidden: bool,
) -> Vec<TypeaheadSuggestion> {
    // Handle slash commands: /help, /clear, etc. The token is the
    // whitespace-delimited `/…` word under the caret, so a command typed
    // after prose completes too (2026-11-12: the text had to START with
    // `/`, so "look at /rev" completed to nothing at all).
    if let Some((start, end)) = slash_token_range(input) {
        let token = &input[start..end];
        let after = &input[end..];
        // /skill <prefix> and /bundle <prefix> expand to installed skill /
        // bundle names (hermes parity — the model-facing expansion takes a
        // name, so completing the name is the useful step).
        if token == "/skill" && after.starts_with(char::is_whitespace) {
            return compute_name_completions(
                "skill",
                after.trim_start(),
                TypeaheadSource::SlashCommand,
            );
        }
        if token == "/bundle" && after.starts_with(char::is_whitespace) {
            return compute_name_completions(
                "bundle",
                after.trim_start(),
                TypeaheadSource::SlashCommand,
            );
        }
        // The caret has left the command name (a space follows it): the name
        // is complete, so the command popup closes.
        if after.starts_with(char::is_whitespace) {
            return Vec::new();
        }
        return compute_slash_suggestions(token, slash_commands, usage);
    }

    // Handle file references: @, @/, @~/, @src/, etc.
    let file_suggestions = compute_file_suggestions(
        input,
        file_autocomplete_limit,
        file_autocomplete_show_hidden,
    );
    if !file_suggestions.is_empty() {
        return file_suggestions;
    }

    // A `$` mention is the LAST token under the cursor (the same "live tail"
    // rule the `@` tier applies), so the two tiers never both claim one popup.
    compute_skill_mention_suggestions(input)
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
/// The caller passes the App's cached `UsageStore`; nothing is read from disk
/// here, so a keystroke never waits on the usage file.
pub(super) fn compute_slash_suggestions(
    input: &str,
    slash_commands: &[(&str, &str)],
    usage: &UsageStore,
) -> Vec<TypeaheadSuggestion> {
    let Some(cmd_prefix) = input.strip_prefix('/') else {
        return Vec::new();
    };
    let prefix_lower = cmd_prefix.to_lowercase();

    // Filter to prefix matches first.
    let matching: Vec<usize> = (0..slash_commands.len())
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

    // Smart ordering (recency → frequency → declaration order) is
    // `rank_commands`, the one ranker the composer uses.
    let candidates: Vec<(&str, &str)> = matching.iter().map(|&i| slash_commands[i]).collect();
    rank_commands(&candidates, usage)
        .into_iter()
        .map(|j| {
            TypeaheadSuggestion::command(
                format!("/{}", candidates[j].0),
                candidates[j].1.to_string(),
            )
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
            display: format!("/{} {}", command, n),
            description: format!("Expand {} '{}' into the turn", command, n),
            source: source.clone(),
        })
        .collect()
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

/// The installed skills with their descriptions, for the `$` mention tier.
///
/// The `/skill <Tab>` tier above needs only names; the `$` popup shows what
/// each skill DOES, so it needs the description too.
static SKILL_MENTION_SNAPSHOT: std::sync::OnceLock<std::sync::RwLock<Vec<(String, String)>>> =
    std::sync::OnceLock::new();

/// Register the skills the composer can NAME — the invocable pool: the
/// operator's own skills PLUS the vendor roots (the installed integrations
/// and the plugin roots), which is exactly the pool the `$` / `/skill`
/// expansion resolves against (`build_skill_invocation_message_in` takes the
/// same roots).
///
/// One pool for both name tiers — `/skill <Tab>` and the `$` mention popup —
/// so the popup can never be a SUBSET of what the expander would run. A
/// vendor skill typed in full always resolved; before this it simply never
/// appeared in the popup, which read as "not installed".
pub fn register_invocable_skills(
    skills_dir: &std::path::Path,
    vendor_roots: &[std::path::PathBuf],
    bundle_names: Vec<String>,
) {
    let _guard = SKILL_SNAPSHOT_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    set_invocable_skills(skills_dir, vendor_roots, bundle_names);
}

/// The same registration over a RESOLVED root list.
///
/// The two-argument form above concatenates the pool and the vendor roots and
/// stops there, so it cannot see `[skills] extra_roots`; this is the form the
/// boot path uses, and it is the only one that makes the popup's name set
/// equal to the set the `$` expander will run.
pub fn register_invocable_skills_in(
    roots: &operant_core::skills::SkillRoots,
    bundle_names: Vec<String>,
) {
    let _guard = SKILL_SNAPSHOT_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let loadable = operant_core::skill_mentions::loadable_skills_in(roots);
    let mention_rows = loadable.clone();
    let names: Vec<String> = loadable.into_iter().map(|(name, _)| name).collect();
    set_typeahead_names(names, bundle_names);
    set_skill_mentions(mention_rows);
}

/// Lock-free inner writer used by `register_invocable_skills` and by the
/// typeahead test (which holds `SKILL_SNAPSHOT_LOCK` across its assertions).
pub(super) fn set_invocable_skills(
    skills_dir: &std::path::Path,
    vendor_roots: &[std::path::PathBuf],
    bundle_names: Vec<String>,
) {
    let loadable = operant_core::skill_mentions::loadable_skills(skills_dir, vendor_roots);
    let mention_rows = loadable.clone();
    let names: Vec<String> = loadable.into_iter().map(|(name, _)| name).collect();
    set_typeahead_names(names, bundle_names);
    set_skill_mentions(mention_rows);
}

/// Lock-free inner writer used by `register_skill_mentions` and by the
/// typeahead test (which holds `SKILL_SNAPSHOT_LOCK` across its assertions).
pub(super) fn set_skill_mentions(entries: Vec<(String, String)>) {
    let snapshot = SKILL_MENTION_SNAPSHOT.get_or_init(|| std::sync::RwLock::new(Vec::new()));
    let mut snapshot = snapshot.write().unwrap_or_else(|e| e.into_inner());
    snapshot.clear();
    snapshot.extend(entries);
}

/// Compute `$skill` mention completions for the live token under the cursor.
///
/// The grammar lives in `operant_core::skill_mentions` — one owner for what
/// counts as a mention — so the popup and the expander can never disagree
/// about where a token starts or ends.
fn compute_skill_mention_suggestions(input: &str) -> Vec<TypeaheadSuggestion> {
    let Some(token) = operant_core::skill_mentions::active_skill_token(input) else {
        return Vec::new();
    };
    let query = token.query.to_lowercase();
    let snapshot = SKILL_MENTION_SNAPSHOT.get_or_init(|| std::sync::RwLock::new(Vec::new()));
    let entries = snapshot.read().unwrap_or_else(|e| e.into_inner());
    entries
        .iter()
        .filter(|(name, _)| query.is_empty() || name.to_lowercase().contains(&query))
        .take(20)
        .map(|(name, description)| {
            TypeaheadSuggestion::skill_ref(
                format!("${name}"),
                if description.is_empty() {
                    "skill".to_string()
                } else {
                    description.clone()
                },
            )
        })
        .collect()
}

/// Compute typeahead suggestions for file references (e.g., `@src/main.rs`).
pub(super) fn compute_file_suggestions(
    input: &str,
    file_autocomplete_limit: usize,
    file_autocomplete_show_hidden: bool,
) -> Vec<TypeaheadSuggestion> {
    // Parse FIRST, then list the parsed path: the grammar (kind prefix,
    // opening quote) is not part of the path, and an accepted directory
    // chains as `@folder:<dir>/` — feeding that string to suggest_files
    // verbatim statted a literal path named `folder:…` and blanked the
    // popup (the chain was dead since the grammar shipped; the fuzzy tier
    // could not rescue it either, because an empty tail after the last `/`
    // disables the walk).
    match parse_file_ref_token(input) {
        Some(token) => suggest_files(
            token.path,
            file_autocomplete_limit,
            file_autocomplete_show_hidden,
        ),
        None => Vec::new(),
    }
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
/// The immediate tier is a single `read_dir` of one directory (fast); the
/// recursive fuzzy tier is requested separately via [`active_file_query`]
/// and arrives asynchronously (plan 2026-10-24 M3).
fn suggest_files(
    prefix: &str,
    max_suggestions: usize,
    show_hidden: bool,
) -> Vec<TypeaheadSuggestion> {
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

    // List files in the directory — via the shared discovery listing
    // (plan 2026-10-24 M2): prefix filter, hidden policy (dot-prefix
    // unlock), `.git` skip, symlink stat, dirs-first order all live there
    // now, one implementation for the interim tier and the M4 drill-down.
    for listed in crate::tui::file_discovery::list_dir(
        &search_dir,
        &partial_name,
        show_hidden,
        max_suggestions,
    ) {
        if suggestions.len() >= max_suggestions {
            break;
        }
        let name = listed.name;
        let is_dir = listed.is_dir;
        let full_path = search_dir.join(&name);

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

        let description = if listed.is_symlink {
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

        suggestions.push(TypeaheadSuggestion::file_ref(
            file_ref_value(&suggestion_text, is_dir),
            file_ref_display_label(&suggestion_text, is_dir),
            description,
        ));
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
