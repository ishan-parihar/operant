//! Which tab, and which heading inside it, one settings row belongs to.
//!
//! `state::all_entries` walks `operant_core::config_registry::SETTING_ROWS`
//! (385 rows, MEASURED 2026-11-16) and builds one `SettingsEntry` for every
//! drawable `live` row. A flat list of that size is the operator's actual
//! complaint about this pane, and the fix needs two facts per row that the
//! registry does not carry: a tab, and a heading inside that tab.
//!
//! Both are DERIVED from the key's own prefix instead of being kept in a
//! second hand-written table. A hand-kept table is exactly the drift this
//! file exists to avoid: `config_registry` is generated from the config
//! structs, so the key set moves whenever a field is added, while a parallel
//! list of "which tab does X live under" is only as current as the last
//! person who remembered it. Prefix matching cannot drift that way — a key
//! added under `[gateway]` is a Gateway row on the day it is generated, with
//! no second edit.
//!
//! A key that maps to `Advanced` is the HONEST fallback, not a silent bucket.
//! `Advanced` is where a prefix this file does not claim lands, it is drawn
//! like every other tab, and it is where the bulk of the registry actually
//! lives today (173 of 385 keys — `[auxiliary_models]`, `[advisor]`,
//! `[client]`, `[autonomous]`, …). That is the real shape of a config that
//! grew a table per subsystem, and the tab says so instead of pretending the
//! row was classified. The alternative is a tab that swallows whatever nobody
//! sorted, rendered collapsed where nobody opens it — the phantom surface
//! `live: false` exists to keep out of the panel. Every row here keeps its own
//! key as its label, so an `Advanced` row is still findable by search and
//! still says which file key it writes.
//!
//! (plan 2026-11-16 S2) OMP orders its tabs explicitly and lists each tab's
//! groups (`settings-defs.ts:22-32`, `settings-defs.ts:36-97`); the guards at
//! the bottom of this file are the same discipline — no registry key may reach
//! the panel without a tab and a heading.

use super::SettingsTab;

/// The dotted prefixes this file claims, in match order.
///
/// Order matters only in that the first match wins; the entries are disjoint
/// today (`plugin.` does not match `plugins.foo`, because the sixth character
/// is `s`, not `.`), and the two spellings of context injection are both
/// listed because the config has carried both.
const TAB_PREFIXES: &[(&str, SettingsTab)] = &[
    ("tui.", SettingsTab::Appearance),
    ("agent.", SettingsTab::Model),
    ("context.", SettingsTab::Context),
    ("context_injection.", SettingsTab::Context),
    ("contextInjection.", SettingsTab::Context),
    ("tools.", SettingsTab::Tools),
    ("browser.", SettingsTab::Tools),
    ("web.", SettingsTab::Tools),
    ("skills.", SettingsTab::Skills),
    ("mcp.", SettingsTab::Skills),
    ("plugin.", SettingsTab::Skills),
    ("plugins.", SettingsTab::Skills),
    ("memory.", SettingsTab::Memory),
    ("gateway.", SettingsTab::Gateway),
];

/// The tab one config key's row renders under.
///
/// By PREFIX, so a key added to the registry tomorrow needs no edit here. A
/// key no prefix claims is `Advanced` — see the module note for why that is
/// the honest answer rather than a bucket to hide rows in.
pub(crate) fn tab_for(key: &str) -> SettingsTab {
    TAB_PREFIXES
        .iter()
        .find_map(|(prefix, tab)| key.starts_with(prefix).then_some(*tab))
        .unwrap_or(SettingsTab::Advanced)
}

/// The heading one config key's row renders under, inside its tab.
///
/// Always `Some`: the panel draws a heading row per group, and a row with no
/// heading would render at the very top of its tab, above the first group,
/// which is where a reader looks for the settings they did NOT come for. The
/// default is the tab's own "General" (or, for Appearance, "Display"), so an
/// unclassified key still sits under a heading a reader can name.
pub(crate) fn group_for(key: &str) -> Option<&'static str> {
    let group = match tab_for(key) {
        SettingsTab::Appearance => {
            if contains_any(key, &["color", "colour", "theme"]) {
                "Colours"
            } else if key.contains("bar") {
                "Status Line"
            } else if key.contains("composer") || key.contains("input") {
                "Composer"
            } else {
                "Display"
            }
        }
        SettingsTab::Model => {
            if key.contains("context_window") || key.contains("context") {
                "Context Window"
            } else if key.contains("effort") {
                "Reasoning Effort"
            } else if key.contains("max") || key.contains("token") {
                "Limits"
            } else {
                "General"
            }
        }
        SettingsTab::Context => {
            if key.contains("compact") {
                "Compaction"
            } else if key.contains("inject") {
                "Injection"
            } else {
                "General"
            }
        }
        SettingsTab::Tools => {
            if key.contains("limit") || key.contains("max") {
                "Limits"
            } else if key.contains("browser") || key.contains("web") {
                "Browser"
            } else {
                "General"
            }
        }
        SettingsTab::Skills => {
            if key.contains("mcp") {
                "MCP"
            } else if key.contains("skill") {
                "Skills"
            } else {
                "Integrations"
            }
        }
        SettingsTab::Memory => "General",
        SettingsTab::Gateway => {
            if key.contains("port") && !key.contains("transport") {
                "Ports"
            } else if key.contains("posture") {
                "Posture"
            } else {
                "General"
            }
        }
        SettingsTab::Advanced => "General",
    };
    Some(group)
}

/// Whether `key` contains any of `needles`.
///
/// A tiny helper rather than a chain of `||`: the group rules are a table of
/// alternatives and reading them as a list is what makes the list reviewable.
fn contains_any(key: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| key.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::config_registry::SETTING_ROWS;

    /// Every registry key gets a tab and a heading, and the tab is one of the
    /// eight the strip renders.
    ///
    /// This is the guard that replaces a hand-kept key list: a key added under
    /// a prefix nobody classified still lands in `Advanced` with "General",
    /// which is drawn and searchable, so the assertion is about the SHAPE
    /// (nothing panics, nothing is left without a heading) rather than about
    /// anyone having remembered the new prefix.
    #[test]
    fn every_registry_key_maps_to_a_tab_and_group() {
        let mut keys = 0usize;
        for row in SETTING_ROWS {
            keys += 1;
            let tab = tab_for(row.key);
            assert!(
                SettingsTab::ALL.contains(&tab),
                "`{}` mapped to a tab the strip does not render",
                row.key
            );
            let group = group_for(row.key);
            assert!(
                group.is_some(),
                "`{}` has no heading, so it would render above the first group",
                row.key
            );
            // A heading with no text would draw an empty band the operator
            // reads as a rendering bug.
            assert!(
                group.is_some_and(|name| !name.is_empty()),
                "`{}` mapped to an empty heading",
                row.key
            );
        }
        // Guards the guard: a registry read that silently yielded nothing
        // would pass every assertion above.
        assert!(keys > 0, "SETTING_ROWS yielded no rows");
    }

    /// The representative mappings, spelled out.
    ///
    /// The derivation above proves every key has SOME tab; this proves the
    /// prefixes mean what the tab strip's labels claim. A prefix table with a
    /// typo (`"atent."`) still passes the shape guard — every key would simply
    /// land in `Advanced` — so the mapping itself needs an assertion that names
    /// the keys.
    #[test]
    fn the_tab_for_a_key_is_stable_under_its_prefix() {
        assert_eq!(tab_for("tui.bar_style"), SettingsTab::Appearance);
        assert_eq!(tab_for("agent.context_window"), SettingsTab::Model);
        assert_eq!(tab_for("context.auto_compact"), SettingsTab::Context);
        assert_eq!(tab_for("context_injection.enabled"), SettingsTab::Context);
        assert_eq!(tab_for("contextInjection.enabled"), SettingsTab::Context);
        assert_eq!(tab_for("tools.browser.enabled"), SettingsTab::Tools);
        assert_eq!(tab_for("web.search_provider"), SettingsTab::Tools);
        assert_eq!(tab_for("skills.enabled"), SettingsTab::Skills);
        assert_eq!(tab_for("mcp.servers"), SettingsTab::Skills);
        assert_eq!(tab_for("plugins.enabled"), SettingsTab::Skills);
        assert_eq!(tab_for("memory.enabled"), SettingsTab::Memory);
        assert_eq!(tab_for("gateway.port"), SettingsTab::Gateway);
        // The honest fallback, and the one the module note is about.
        assert_eq!(tab_for("something.unmapped"), SettingsTab::Advanced);
    }

    /// A key with no specific group still gets one.
    ///
    /// `group_for` returns `Option` because `SettingsEntry.group` is an
    /// `Option` — the panel's own rows may render ungrouped at the top of a
    /// tab. A REGISTRY row never should: it arrives from a generated table
    /// nobody has looked at, and "no heading" would put it above every heading
    /// in the tab. Asserting `Some` here is what keeps that from regressing.
    #[test]
    fn a_key_with_no_specific_group_still_gets_one() {
        assert!(group_for("something.unmapped").is_some());
        assert_eq!(group_for("something.unmapped"), Some("General"));
    }

    /// No tab is a label with nothing behind it.
    ///
    /// An empty tab is worse than no tab: the strip draws eight labels and one
    /// of them opens onto nothing, which reads as a broken panel rather than
    /// as an absent feature. MEASURED 2026-11-16 with this mapping: every one
    /// of the eight owns at least one registry key — Appearance 18, Model 65,
    /// Context 13, Tools 60, Skills 18, Memory 8, Gateway 30, Advanced 173 —
    /// so there is nothing to exempt by name. If a future prefix sweep empties
    /// one, exempt THAT tab here with the reason instead of weakening this
    /// assertion into a `>= 1` over all tabs.
    #[test]
    fn every_tab_has_at_least_one_key_in_the_registry() {
        // Indexed by `SettingsTab::ALL`, which is also the strip's order, so
        // the printed line and the drawn tab are the same row.
        let mut counts = [0usize; SettingsTab::ALL.len()];
        for row in SETTING_ROWS {
            let tab = tab_for(row.key);
            let index = SettingsTab::ALL
                .iter()
                .position(|candidate| *candidate == tab)
                .expect("tab_for returned a tab outside SettingsTab::ALL");
            counts[index] += 1;
        }
        // The counts, printed: the panel's tab sizes are a property of
        // `config_registry`, and the natural mistake is to reason about them
        // from this file instead of measuring them (same trap as the row
        // count, iter-2451).
        for (tab, count) in SettingsTab::ALL.into_iter().zip(counts) {
            eprintln!("SETTINGS TAB: {:<10} {count} registry keys", tab.label());
        }
        for (tab, count) in SettingsTab::ALL.into_iter().zip(counts) {
            assert!(
                count > 0,
                "the {} tab owns no registry key, so the strip draws a label that opens onto nothing",
                tab.label()
            );
        }
    }
}
