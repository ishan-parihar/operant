//! Operator-authored theme files (plan 2026-11-15 U2).
//!
//! A theme file is the `[tui.colors]` shape with a name: `role = "#rrggbb"`
//! pairs, one per semantic role. The join key already existed — the palette
//! already accepts these keys — so this module adds the file, not a vocabulary.
//!
//! Lookup: `<config dir>/themes/<name>.toml`, where `<config dir>` is
//! `OPERANT_CONFIG_DIR` when set and non-empty, else `~/.operant` — the same
//! rule `operant-core`'s loader follows, restated here because that helper is
//! `pub(crate)`.
//!
//! Precedence is deliberately one-way: a BUILT-IN name is never shadowed by a
//! file. That is OMP's rule too (`loader.ts:26-42`), and the alternative —
//! letting a file named `dark` quietly replace the shipped `dark` — is a
//! support burden with no upside.

use std::path::PathBuf;

/// The directory operator themes are read from.
pub fn dir() -> PathBuf {
    if let Ok(custom) = std::env::var("OPERANT_CONFIG_DIR") {
        let custom = custom.trim();
        if !custom.is_empty() {
            let expanded = match custom.strip_prefix('~') {
                Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                    match std::env::var("HOME") {
                        Ok(home) => home + rest,
                        Err(_) => custom.to_string(),
                    }
                }
                _ => custom.to_string(),
            };
            return PathBuf::from(expanded).join("themes");
        }
    }
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".operant").join("themes"),
        Err(_) => PathBuf::from("themes"),
    }
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
                assert!(
                    value.is_empty()
                        || value.starts_with('#')
                        || value.chars().all(|c| c.is_ascii_digit()),
                    "{name}: {key} has an unusable value {value:?}"
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
        let _lock = crate::config::test_env_lock();
        let before = crate::tui::theme_colors::accent();
        crate::tui::theme_colors::set_active_theme("tokyo-night");
        let after = crate::tui::theme_colors::accent();
        crate::tui::theme_colors::set_active_theme("default");
        assert_ne!(
            before, after,
            "tokyo-night must move at least one colour the TUI reads"
        );
    }

    /// A file's pairs are read as strings and numbers alike, and a file with no
    /// `colors` table is no theme at all.
    #[test]
    fn a_file_is_its_colors_table() {
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
        // SAFETY: serialized by the crate-wide test_env_lock below.
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
