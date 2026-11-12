//! The Omarchy agent integration: register Operant as a selectable default agent.
//!
//! WHY THIS IS NOT A ONE-LINE CONFIG EDIT
//!
//! Omarchy's agent list is not data, it is a `case` statement inside
//! `/usr/bin/omarchy-default-agent`, a script the Omarchy package owns. Its
//! own header says it: the package reinstalls that file on update, so editing
//! it is a change with a scheduled deletion date. And the packaged launcher
//! has an explicit `*)` arm that refuses anything it does not know:
//!
//! ```text
//! *) echo "Unsupported default agent: $agent" >&2; exit 1 ;;
//! ```
//!
//! So "omarchy does not show the option to use operant as the default" is
//! really two problems. There is no menu ROW (nothing to see), and even if
//! there were one, selecting it would reach that `*)` arm and refuse.
//!
//! THE SHAPE OF THE FIX
//!
//! `~/.local/bin` precedes `/usr/bin` on the login shell's PATH, so a wrapper
//! there intercepts every caller — the menu action, the keybinding, and the
//! `omarchy default agent` route. The wrappers add the agents registered under
//! `~/.config/omarchy/defaults/extra-agents/` and hand everything else back to
//! the packaged script untouched. Registering an agent is then ONE FILE:
//!
//! ```text
//! ~/.config/omarchy/defaults/extra-agents/operant   ->  omarchy-operant
//! ```
//!
//! jcode was installed by hand-patching two wrappers that hard-coded the name.
//! Generalising the wrappers is what lets jcode and Operant coexist: this
//! installer registers `jcode` in the same registry before it replaces
//! anything, so the agent that used to work still works — and it is backed up
//! as well, so a failure to generalise is recoverable.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::herdr_plugin::make_executable;

/// The agents this installer can register.
///
/// `jcode` is here because the hand-written wrappers on this machine only
/// knew about it: generalising the wrappers would otherwise silently drop the
/// agent the operator was using. It is registered, never launched by us, and
/// only when its own launcher is already on PATH.
pub const REGISTRABLE: &[&str] = &["operant", "jcode"];

const OMARCHY_DEFAULT_AGENT: &str =
    include_str!("../../integrations/omarchy/omarchy-default-agent");
const OMARCHY_AGENT: &str = include_str!("../../integrations/omarchy/omarchy-agent");
const OMARCHY_OPERANT: &str = include_str!("../../integrations/omarchy/omarchy-operant");

/// The menu key for the Operant row.
pub const MENU_KEY: &str = "setup.default.agent.operant";

/// The menu row, as JSONC.
const MENU_ROW: &str = r#"  // Operant is not in the packaged agent list. The action resolves
  // omarchy-default-agent from PATH, where the user-level wrapper in
  // ~/.local/bin adds the extra agents and hands every other agent back to the
  // packaged script.
  "setup.default.agent.operant": {
    "icon": "󰚩",
    "label": "operant",
    "aliases": ["operant", "op"],
    "checked": "[[ \"$(omarchy-default-agent)\" == \"operant\" ]]",
    "action": "omarchy-default-agent operant"
  },"#;

/// Whether this machine runs Omarchy.
///
/// Decided by the packaged launcher, not by an `omarchy` binary: the launcher
/// is the file this integration has to cooperate with, and its absence means
/// there is nothing to cooperate with.
pub fn omarchy_available() -> bool {
    Path::new("/usr/bin/omarchy-default-agent").exists()
}

/// `~/.local/bin`
fn local_bin() -> PathBuf {
    home().join(".local").join("bin")
}

/// `~/.config/omarchy`
fn omarchy_config() -> PathBuf {
    home().join(".config").join("omarchy")
}

/// The extra-agent registry.
fn extra_agents_dir() -> PathBuf {
    omarchy_config().join("defaults").join("extra-agents")
}

/// The user menu extension the shell merges over its defaults.
fn menu_file() -> PathBuf {
    omarchy_config()
        .join("extensions")
        .join("omarchy-menu.jsonc")
}

/// `$HOME`, or an empty path when it is unset.
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The placeholder in the shipped wrapper's agent-list header.
///
/// The list is FILLED IN AT INSTALL TIME from the packaged script's own list,
/// so the wrapper never advertises a shorter agent list than Omarchy ships.
/// A hand-copied list was already out of date (ten agents against the
/// packaged script's twenty-four) and would have drifted again with the next
/// Omarchy release.
const AGENT_LIST_PLACEHOLDER: &str = "{{OMARCHY_AGENT_LIST}}";

/// The packaged script whose agent list the wrapper mirrors.
const PACKAGED_DEFAULT_AGENT: &str = "/usr/bin/omarchy-default-agent";

/// The agent list the installed wrapper advertises.
///
/// The packaged list, plus every extra agent this installer registers.
fn agent_list() -> String {
    let mut agents = packaged_agent_list();
    for extra in REGISTRABLE {
        if !agents.iter().any(|agent| agent == extra) {
            agents.push((*extra).to_string());
        }
    }
    agents.join("|")
}

/// The agent names the packaged script advertises, parsed from its header.
fn packaged_agent_list() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(PACKAGED_DEFAULT_AGENT) else {
        return Vec::new();
    };
    text.lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix("# omarchy:args=[")?;
            let list = rest.strip_suffix(']')?;
            Some(
                list.split('|')
                    .map(|agent| agent.trim().to_string())
                    .filter(|agent| !agent.is_empty())
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default()
}

/// The wrapper as it should be installed, with the agent list filled in.
fn wrapper_contents() -> String {
    OMARCHY_DEFAULT_AGENT.replace(AGENT_LIST_PLACEHOLDER, &agent_list())
}

/// The install state of the Omarchy integration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OmarchyStatus {
    /// Wrappers and launcher match the shipped copies.
    pub wrappers_current: bool,
    /// Files that are missing or differ from the shipped copy.
    pub stale: Vec<String>,
    /// `operant` is registered in the extra-agent registry.
    pub registered: bool,
    /// The menu extension carries the Operant row.
    pub menu_row: bool,
    /// The agent Omarchy currently launches, when one is chosen.
    pub default_agent: Option<String>,
    /// `jcode` is registered, so generalising the wrappers kept it working.
    pub jcode_preserved: bool,
}

/// Inspect the installed integration.
pub fn status() -> OmarchyStatus {
    // The default-agent wrapper's expected content is the template with the
    // agent list filled in, so "stale" means "the installed list no longer
    // matches what Omarchy ships" — not "the template changed".
    let expected_default_agent = wrapper_contents();
    let stale = [
        ("omarchy-default-agent", expected_default_agent.as_str()),
        ("omarchy-agent", OMARCHY_AGENT),
    ]
    .iter()
    .filter_map(|(name, shipped)| {
        let path = local_bin().join(name);
        let current = std::fs::read_to_string(&path)
            .map(|existing| existing == *shipped)
            .unwrap_or(false);
        (!current).then(|| path.display().to_string())
    })
    .chain(
        std::fs::read_to_string(local_bin().join("omarchy-operant"))
            .map(|existing| existing != OMARCHY_OPERANT)
            .unwrap_or(true)
            .then(|| local_bin().join("omarchy-operant").display().to_string()),
    )
    .collect::<Vec<_>>();

    OmarchyStatus {
        wrappers_current: stale.is_empty(),
        stale,
        registered: extra_agents_dir().join("operant").is_file(),
        menu_row: std::fs::read_to_string(menu_file())
            .map(|text| text.contains(MENU_KEY))
            .unwrap_or(false),
        default_agent: std::fs::read_to_string(omarchy_config().join("defaults").join("agent"))
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()),
        jcode_preserved: extra_agents_dir().join("jcode").is_file(),
    }
}

/// Install the integration.
///
/// Ordered so that nothing the operator relies on is lost if a later step
/// fails: the existing wrappers are backed up and the existing registrations
/// are carried over BEFORE the generalized wrappers replace anything.
pub fn install() -> Result<Vec<String>> {
    if !omarchy_available() {
        anyhow::bail!("Omarchy is not installed (no /usr/bin/omarchy-default-agent)");
    }
    let mut done = Vec::new();

    // 1. Registration, before replacement: whatever the hand-written wrappers
    //    knew about is written into the registry first, so the generalized
    //    wrappers inherit it rather than dropping it.
    std::fs::create_dir_all(extra_agents_dir())
        .with_context(|| format!("creating {}", extra_agents_dir().display()))?;
    let mut registered = false;
    for agent in REGISTRABLE {
        let launcher = format!("omarchy-{agent}");
        // The hand-written wrapper this replaces tested the AGENT's own binary
        // (`~/.local/bin/jcode`); the launcher is a separate file. Either one
        // means the agent is installed, so either one earns the registration —
        // and registering it gives the operator this integration's own
        // "not installed" message instead of the packaged script's usage text.
        let installed = launcher_on_path(&launcher) || launcher_on_path(agent);
        if agent != &"operant" && !installed {
            continue;
        }
        let path = extra_agents_dir().join(agent);
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() != Some(&format!("{launcher}\n")) {
            std::fs::write(&path, format!("{launcher}\n"))
                .with_context(|| format!("registering {agent}"))?;
            registered = true;
        }
    }
    // Only a CHANGE is reported, so the doctor's "N fixed" count counts fixes
    // and not steps.
    if registered {
        done.push(format!(
            "registered extra agents in {}",
            extra_agents_dir().display()
        ));
    }

    // 2. The wrappers and Operant's launcher.
    //
    // A wrapper is BACKED UP only when it is actually about to be replaced.
    // Backing up unconditionally left a copy of this integration's own
    // wrapper behind on every later run, which is noise in ~/.local/bin and
    // makes the install non-idempotent: the second run changed the home
    // even though it had nothing to change.
    std::fs::create_dir_all(local_bin())
        .with_context(|| format!("creating {}", local_bin().display()))?;
    let mut backups = Vec::new();
    for (name, contents, executable) in [
        ("omarchy-default-agent", wrapper_contents().as_str(), true),
        ("omarchy-agent", OMARCHY_AGENT, true),
        ("omarchy-operant", OMARCHY_OPERANT, true),
    ] {
        let path = local_bin().join(name);
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() != Some(contents) {
            // Only the two wrappers the operator might have written by hand
            // are worth keeping a copy of; the launcher is ours.
            if name != "omarchy-operant"
                && let Some(backup) = backup_once(&path)?
            {
                backups.push(backup);
            }
            std::fs::write(&path, contents)
                .with_context(|| format!("writing {}", path.display()))?;
            done.push(format!("wrote {}", path.display()));
        }
        if executable {
            make_executable(&path)?;
        }
    }
    if !backups.is_empty() {
        done.push(format!(
            "backed up the previous wrapper(s): {}",
            backups.join(", ")
        ));
    }

    // 4. The menu row, so the agent is selectable at all.
    match ensure_menu_row()? {
        MenuRow::Added => done.push(format!(
            "added the Operant row to {}",
            menu_file().display()
        )),
        MenuRow::Present => {}
    }

    Ok(done)
}

/// What [`ensure_menu_row`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuRow {
    Added,
    Present,
}

/// Insert the Operant row into the user menu extension.
///
/// Omarchy's shell watches exactly one user file
/// (`~/.config/omarchy/extensions/omarchy-menu.jsonc`) and merges it over its
/// defaults, so there is no second file to drop a fragment into. The row is
/// inserted at the top of the object, after the opening brace, which keeps the
/// edit small and works whether or not the file already has content.
fn ensure_menu_row() -> Result<MenuRow> {
    ensure_menu_row_at(&menu_file())
}

/// Insert the Operant row into a menu extension file.
///
/// The path is a parameter so the insertion can be proven on a real JSONC file
/// in a test, rather than only on the operator's live menu.
fn ensure_menu_row_at(path: &Path) -> Result<MenuRow> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            let seeded = format!("{{\n{MENU_ROW}\n}}\n");
            std::fs::write(path, seeded).with_context(|| format!("writing {}", path.display()))?;
            return Ok(MenuRow::Added);
        }
    };
    if existing.contains(MENU_KEY) {
        return Ok(MenuRow::Present);
    }
    let Some(brace) = existing.find('{') else {
        anyhow::bail!(
            "{} is not a JSON object — refusing to edit it",
            path.display()
        );
    };
    let insertion = brace + 1;
    let mut updated = String::with_capacity(existing.len() + MENU_ROW.len() + 2);
    updated.push_str(&existing[..insertion]);
    updated.push('\n');
    updated.push_str(MENU_ROW);
    updated.push_str(&existing[insertion..]);
    std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(MenuRow::Added)
}

/// Whether a command resolves on PATH.
fn launcher_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(name);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

/// Copy `path` to a timestamped sibling, once per day.
///
/// Returns the backup path when one was written. Backing up only once per day
/// keeps a repeated `--fix` from littering the operator's `~/.local/bin` with
/// identical copies while still preserving the pre-Operant state.
fn backup_once(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    let backup = path.with_file_name(format!(
        "{}.operant-backup.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("wrapper"),
        stamp
    ));
    if backup.exists() {
        return Ok(None);
    }
    std::fs::copy(path, &backup).with_context(|| format!("backing up {}", path.display()))?;
    Ok(Some(backup.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The row must be a key the shell can merge: a bare `setup.default.agent.<name>`
    /// entry whose action routes through the same wrapper the keybinding uses.
    #[test]
    fn the_menu_row_uses_the_wrapper_route() {
        assert!(MENU_ROW.contains(MENU_KEY));
        assert!(
            MENU_ROW.contains(r#""action": "omarchy-default-agent operant""#),
            "the row must route through PATH, where the wrapper lives"
        );
        assert!(
            MENU_ROW.contains(r#""checked""#),
            "without a checked guard the menu cannot show which agent is active"
        );
    }

    /// The generated row has to be insertable into a live JSON object, so it
    /// must carry its own trailing comma and no enclosing braces.
    #[test]
    fn the_menu_row_is_a_fragment_not_a_document() {
        let trimmed = MENU_ROW.trim();
        assert!(
            !trimmed.starts_with('{'),
            "the row is spliced into an object"
        );
        assert!(trimmed.ends_with(','), "the row needs its own separator");
        // Balanced braces: the object opened in the string is closed in it.
        let opens = MENU_ROW.matches('{').count();
        let closes = MENU_ROW.matches('}').count();
        assert_eq!(opens, closes, "the row's object literal is balanced");
    }

    #[test]
    fn operant_is_registrable_and_so_is_jcode() {
        assert!(REGISTRABLE.contains(&"operant"));
        // The wrappers this replaces only knew about jcode; dropping it would
        // be a regression for the agent the operator was already using.
        assert!(REGISTRABLE.contains(&"jcode"));
    }

    /// The wrapper's advertised agent list must be the PACKAGED list plus the
    /// extras — never a hand-copied subset.
    ///
    /// A hand-written list was already out of date: it named ten agents while
    /// the packaged script named twenty-four. The header drives `omarchy
    /// --help`, so a short list misinforms the operator about what they can
    /// choose.
    #[test]
    fn the_advertised_agent_list_covers_the_packaged_agents_and_the_extras() {
        let list = agent_list();
        let packaged = packaged_agent_list();
        if packaged.is_empty() {
            // No packaged script on this machine (the test host may not have
            // Omarchy); the extras still have to be advertised.
            assert!(list.contains("operant"));
            return;
        }
        for agent in &packaged {
            assert!(
                list.split('|').any(|entry| entry == agent),
                "the packaged agent {agent} must stay advertised"
            );
        }
        for extra in REGISTRABLE {
            assert!(
                list.split('|').any(|entry| entry == *extra),
                "the extra agent {extra} must be advertised"
            );
        }
    }

    /// The placeholder must never reach the installed file: a literal
    /// `{{OMARCHY_AGENT_LIST}}` in the header would be shown to the operator
    /// by `omarchy --help`.
    #[test]
    fn the_installed_wrapper_has_no_placeholder_left() {
        let contents = wrapper_contents();
        assert!(
            !contents.contains(AGENT_LIST_PLACEHOLDER),
            "the agent list must be filled in"
        );
        assert!(contents.contains("# omarchy:args=["));
    }

    #[test]
    fn the_wrappers_delegate_unknown_agents_to_the_packaged_script() {
        for wrapper in [OMARCHY_DEFAULT_AGENT, OMARCHY_AGENT] {
            assert!(
                wrapper.contains(r#"packaged="/usr/bin/omarchy-"#),
                "the wrapper must name the packaged script it defers to"
            );
            assert!(
                wrapper.contains(r#"exec "$packaged""#),
                "an unknown agent must be handed back untouched"
            );
            assert!(
                wrapper.contains("extra-agents"),
                "the registry is what makes this data, not another case arm"
            );
        }
    }

    /// The launcher must never silently swallow a prompt: it puts it on the
    /// clipboard, which is the shape the packaged launcher uses for the agents
    /// that cannot take a starting prompt.
    #[test]
    fn the_operant_launcher_preserves_a_prompt() {
        assert!(OMARCHY_OPERANT.contains("wl-copy"));
        assert!(OMARCHY_OPERANT.contains("--prompt"));
        assert!(
            OMARCHY_OPERANT.contains("org.omarchy.agent"),
            "every agent window shares one app-id for window rules"
        );
    }

    /// A menu file shaped like the live one: comments, several rows, and a
    /// trailing comment block.
    const MENU_FIXTURE: &str = r#"{
  // >>> a plugin block (managed by someone) >>>
  "style.bar.barber": {
    "icon": "x",
    "label": "Barber"
  },
  // <<< a plugin block <<<
  "setup.default.agent.jcode": {
    "icon": "y",
    "label": "jcode"
  }
}
"#;

    /// The insertion must not cost the operator a row they already had.
    ///
    /// The live menu file is the operator's own: it carries the jcode row and
    /// whatever plugin blocks they installed. An edit that drops one of them
    /// would silently break an agent they use, which is the worst outcome an
    /// integration installer can have.
    #[test]
    fn the_menu_row_is_inserted_without_losing_the_existing_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("omarchy-menu.jsonc");
        std::fs::write(&path, MENU_FIXTURE).expect("write");

        assert_eq!(ensure_menu_row_at(&path).expect("insert"), MenuRow::Added);

        let updated = std::fs::read_to_string(&path).expect("read");
        assert!(updated.contains(MENU_KEY), "the operant row is there");
        assert!(
            updated.contains("style.bar.barber"),
            "the plugin row survives"
        );
        assert!(
            updated.contains("setup.default.agent.jcode"),
            "the jcode row survives — it is the agent the operator already used"
        );
        assert!(
            updated.contains(">>> a plugin block (managed by someone) >>>"),
            "the comments survive: the file is JSONC and the shell reads them"
        );
        // The inserted fragment carries its own separator, so the object stays
        // balanced.
        let opens = updated.matches('{').count();
        let closes = updated.matches('}').count();
        assert_eq!(opens, closes, "the object literal is still balanced");
    }

    /// A second run must be a no-op: the row is added once, not twice.
    #[test]
    fn the_menu_row_is_added_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("omarchy-menu.jsonc");
        std::fs::write(&path, MENU_FIXTURE).expect("write");

        ensure_menu_row_at(&path).expect("first insert");
        let after_first = std::fs::read_to_string(&path).expect("read");
        assert_eq!(
            ensure_menu_row_at(&path).expect("second insert"),
            MenuRow::Present
        );
        let after_second = std::fs::read_to_string(&path).expect("read");

        assert_eq!(after_first, after_second, "the second run changed nothing");
        assert_eq!(
            after_second.matches(MENU_KEY).count(),
            1,
            "the row is present exactly once"
        );
    }

    /// A missing menu file is seeded with the row, so a machine that never
    /// customised the menu still gets the option.
    #[test]
    fn a_missing_menu_file_is_seeded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("omarchy-menu.jsonc");

        assert_eq!(ensure_menu_row_at(&path).expect("seed"), MenuRow::Added);
        let seeded = std::fs::read_to_string(&path).expect("read");
        assert!(seeded.contains(MENU_KEY));
        let opens = seeded.matches('{').count();
        let closes = seeded.matches('}').count();
        assert_eq!(opens, closes, "the seeded file is a balanced object");
    }
}
