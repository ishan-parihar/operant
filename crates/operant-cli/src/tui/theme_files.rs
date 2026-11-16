//! Operator-authored theme files (plan 2026-11-15 U2).
//!
//! A theme file is the `[tui.colors]` shape with a name: `role = "#rrggbb"`
//! pairs, one per semantic role. The join key already existed — the palette
//! already accepts these keys — so this module adds the file, not a vocabulary.
//!
//! Lookup: `<config dir>/themes/<name>.toml`, where `<config dir>` is
//! `OPERANT_CONFIG_DIR` when set and non-empty, else the RESOLVED operant home.
//!
//! THIS USED TO RESTATE THE RULE, AND THE RESTATEMENT WAS THE BUG. It spelled
//! the default as `env::var("HOME")/.operant`, which ignores BOTH `HERMES_HOME`
//! and the sticky active profile. A profile switched with `operant profile use`
//! therefore read the DEFAULT profile's `themes/` - the operator's own themes
//! vanished, and `one_home_resolver` could not see it because the guard only
//! looked for the `dirs::home_dir()` spelling (RT-Z43-F).
//!
//! The rule now comes from operant-core, in two calls, so there is nothing here
//! to drift: `operant_config_dir_override()` is the loader's own helper and
//! `operant_home()` is the one resolver.
//!
//! Precedence is deliberately one-way: a BUILT-IN name is never shadowed by a
//! file. That is OMP's rule too (`loader.ts:26-42`), and the alternative —
//! letting a file named `dark` quietly replace the shipped `dark` — is a
//! support burden with no upside.

use std::path::PathBuf;

/// The directory operator themes are read from.
pub fn dir() -> PathBuf {
    if let Some(custom) = operant_core::config::operant_config_dir_override() {
        return custom.join("themes");
    }
    operant_core::platform::operant_home().join("themes")
}

/// Every operator theme name, sorted. A missing or unreadable directory is
/// empty, not an error: an operator with no `themes/` directory has eight
/// built-ins and needs nothing else.
pub fn available() -> Vec<String> {
    let mut names: Vec<String> = crate::tui::theme_bundled::BUNDLED
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    if let Ok(entries) = std::fs::read_dir(dir()) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|x| x == "toml")
                && let Some(stem) = entry.path().file_stem()
            {
                names.push(stem.to_string_lossy().to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Read one operator theme's `role = "#rrggbb"` pairs.
///
/// Returns `None` when the file is absent, unreadable, unparseable, or carries
/// no valid pair — an unusable theme is the same as no theme, and the caller
/// keeps the built-in palette rather than half-applying a broken file.
pub fn load(name: &str) -> Option<Vec<(String, String)>> {
    // A name is a file stem, never a path: reject separators so a theme name
    // cannot escape the themes directory.
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    // A bundled palette is a built-in for precedence purposes: it ships with the
    // binary, so an operator file of the same name is ignored.
    if let Some((_, pairs)) = crate::tui::theme_bundled::BUNDLED
        .iter()
        .find(|(bundled, _)| *bundled == name)
    {
        return Some(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        );
    }
    let path = dir().join(format!("{name}.toml"));
    let text = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = text.parse().ok()?;
    let colors = value.get("colors")?.as_table()?;
    let pairs: Vec<(String, String)> = colors
        .iter()
        .filter_map(|(key, value)| {
            let value = match value {
                toml::Value::String(s) => s.clone(),
                toml::Value::Integer(n) => n.to_string(),
                _ => return None,
            };
            Some((key.clone(), value))
        })
        .collect();
    (!pairs.is_empty()).then_some(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (RT-Z43-F) THE resolver test. `dir()` must follow `HERMES_HOME`.
    ///
    /// It used to spell the default as `env::var("HOME")/.operant`, which
    /// ignores BOTH `HERMES_HOME` and the sticky active profile. An operator
    /// who ran `operant profile use work` therefore got the DEFAULT profile's
    /// `themes/` - their own theme files vanished and nothing said why.
    ///
    /// Mutation-proven: restoring the `env::var("HOME")` spelling fails the
    /// second assertion, because that spelling cannot see `HERMES_HOME` at all.
    #[test]
    fn the_themes_directory_follows_hermes_home() {
        let _guard = crate::config::test_env_lock();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!(
            "operant-themes-home-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&base).expect("fixture");

        let previous = std::env::var("HERMES_HOME").ok();
        let previous_operant = std::env::var("OPERANT_CONFIG_DIR").ok();
        // SAFETY: serialised by the crate's environment lock.
        unsafe { std::env::set_var("HERMES_HOME", &base) };
        unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") };

        let resolved = dir();
        assert!(
            resolved.starts_with(&base),
            "the themes dir must live under HERMES_HOME ({base:?}), got {resolved:?}"
        );
        assert!(
            resolved.ends_with("themes"),
            "the themes dir must end in `themes`, got {resolved:?}"
        );

        // SAFETY: serialised by the crate's environment lock.
        match previous {
            Some(v) => unsafe { std::env::set_var("HERMES_HOME", v) },
            None => unsafe { std::env::remove_var("HERMES_HOME") },
        }
        match previous_operant {
            Some(v) => unsafe { std::env::set_var("OPERANT_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") },
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The themes directory must be scoped, never empty, and the name guard
    /// must reject anything that could name a path outside it.
    #[test]
    fn the_directory_is_scoped_and_the_name_guard_rejects_paths() {
        assert!(dir().ends_with("themes"), "themes dir is {:?}", dir());
        for bad in ["", "../escape", "a/b", "a\\b", "..", "with space", "x.toml"] {
            assert!(
                load(bad).is_none(),
                "{bad:?} must not resolve to a theme file"
            );
        }
    }

    /// Every bundled palette must be APPLYABLE: each pair names a real role key
    /// and carries a value `from_pairs` accepts. A generated table with one bad
    /// key would ship a palette that silently loses that role — the same
    /// writer-only failure U3 exists to prevent, one level up.
    #[test]
    fn every_bundled_palette_names_only_real_roles() {
        use crate::tui::vendor::style::palette::Role;
        for (name, pairs) in crate::tui::theme_bundled::BUNDLED {
            for (key, value) in pairs.iter() {
                assert!(
                    Role::from_key(key).is_some(),
                    "{name}: {key:?} is not a role key"
                );
                // Hex only. A 0..255 ANSI index would pass a digits-only check
                // and then be dropped by `parse_hex` at apply time, so the
                // generator resolves indices to hex and this rejects them.
                assert!(
                    crate::tui::vendor::style::palette::parse_hex(value).is_some(),
                    "{name}: {key} has an unparseable value {value:?}"
                );
            }
            assert!(
                pairs.len() >= 20,
                "{name} carries only {} pairs",
                pairs.len()
            );
        }
    }

    /// A bundled palette must actually change the rendered palette, or it is a
    /// row in the picker that applies to nothing.
    #[test]
    fn applying_a_bundled_palette_changes_the_palette() {
        // The process-global palette is shared with every other theming test, so
        // a bare "read, apply, read" races: the suite runs these in parallel and
        // a neighbouring `set_active_theme` between the two reads makes the guard
        // fail for a reason that has nothing to do with the file. `ACTIVE_LOCK`
        // is the lock `theme_colors`' own tests use for exactly this.
        let _lock = crate::tui::vendor::style::STYLE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = crate::tui::theme_colors::accent();
        crate::tui::theme_colors::set_active_theme("dark-tokyo-night");
        let after = crate::tui::theme_colors::accent();
        crate::tui::theme_colors::set_active_theme("default");
        assert_ne!(
            before, after,
            "dark-tokyo-night must move at least one colour the TUI reads"
        );
    }

    /// A file's pairs are read as strings and numbers alike, and a file with no
    /// `colors` table is no theme at all.
    #[test]
    fn a_file_is_its_colors_table() {
        // The comment below has always SAID "serialized by the crate-wide
        // test_env_lock" but the lock was never taken, so this test raced every
        // other one that touches OPERANT_CONFIG_DIR. It passed by luck until a
        // sibling test started setting HERMES_HOME. Taking the lock is what the
        // comment promised.
        let _guard = crate::config::test_env_lock();
        let root = std::env::temp_dir().join(format!("operant-theme-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("themes")).unwrap();
        std::fs::write(
            root.join("themes/probe.toml"),
            "[colors]\naccent = \"#febc38\"\ndim = 244\n",
        )
        .unwrap();
        std::fs::write(root.join("themes/broken.toml"), "name = \"broken\"\n").unwrap();

        let saved = std::env::var("OPERANT_CONFIG_DIR").ok();
        // SAFETY: serialized by the crate-wide test_env_lock above.
        unsafe {
            std::env::set_var("OPERANT_CONFIG_DIR", &root);
        }
        let pairs = load("probe").expect("probe.toml has a colors table");
        assert_eq!(
            pairs,
            vec![
                ("accent".to_string(), "#febc38".to_string()),
                ("dim".to_string(), "244".to_string())
            ]
        );
        // `available()` is the bundled palettes PLUS the directory, so the
        // assertion is that both operator files are present — not that they are
        // the whole list, which stopped being true when the palettes shipped.
        let names = available();
        assert!(names.contains(&"probe".to_string()), "{names:?}");
        assert!(names.contains(&"broken".to_string()), "{names:?}");
        assert!(
            names.len() > 100,
            "the bundled palettes are part of the list: {}",
            names.len()
        );
        assert!(
            load("broken").is_none(),
            "a file with no colors table is not a theme"
        );
        assert!(load("absent").is_none(), "an absent theme is no theme");
        // SAFETY: still serialized; restored below.
        unsafe {
            match saved {
                Some(value) => std::env::set_var("OPERANT_CONFIG_DIR", value),
                None => std::env::remove_var("OPERANT_CONFIG_DIR"),
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
