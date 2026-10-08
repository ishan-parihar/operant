//! DM/feed context injection — phase 1 (the cron seam).
//!
//! Design: `docs/plan-2026-10-08-dm-feed-context-injection.md` — approved
//! 2026-10-08 with the owner's rulings: per-aspect character quotas (the
//! MEMORY.md budget model, applied to four aspect classes) and re-ranking
//! across the classes (recency + lexical affinity + author authority).
//!
//! Phase 1 sources are org-internal only — everything here is data the
//! organism already persists (the worklog), so there is no platform API
//! risk and the seats' cross-visibility goes live immediately:
//!
//! - `Self` — the seat's own recent worklog rows (its continuity thread).
//! - `Global` — rows authored by `premiere` / `chief-of-staff` (directives,
//!   digests) that the seat has not consumed.
//! - `Dept` — rows by same-department peers (empty until Wave 3 departments;
//!   the honest value, not a guess).
//! - `Dm` — reserved for phase 2 (platform read adapters) and the
//!   socialization sessions; the org layer stores no DM bodies today.
//!
//! Nonredundancy is the watermark backbone: a per-seat, per-class monotonic
//! cursor in `context_watermarks`. Items at/below the cursor are never
//! collected again; advancing happens on collection (the ranking's judgment
//! that a low-score item is not worth showing must not leave it pending
//! forever, crowding out later items — at-least-once semantics plus the
//! render-time content-hash dedup make a re-collect harmless).
//!
//! Failure posture: any store error degrades to "no injection" and the
//! prompt is returned byte-identical — the iter-666 contract. Injection
//! must never fail a run.
//!
//! ponytail: phase 1 reads the worklog directly rather than copying it into
//! a `context_items` table (the design doc's §5 store is deferred to phase
//! 2, where pull-once platform reads genuinely need a durable landing).
//! Same pipeline, less code, no schema duplication of rows the worklog
//! already keeps.

use crate::error::{Error, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

/// Per-seat, per-class read cursors — the nonredundancy backbone.
pub const CONTEXT_WATERMARKS_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS context_watermarks (
        seat_id TEXT NOT NULL,
        class   TEXT NOT NULL,   -- dm | global | dept | self
        last_ts INTEGER NOT NULL DEFAULT 0,  -- unix seconds
        PRIMARY KEY (seat_id, class)
    );
"#;

/// The four aspect classes (the owner's list, in render order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextClass {
    Dm,
    Global,
    Dept,
    Self_,
}

impl ContextClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::Global => "global",
            Self::Dept => "dept",
            Self::Self_ => "self",
        }
    }
}

/// One normalized item flowing through the pipeline.
#[derive(Debug, Clone)]
pub struct ContextItem {
    pub class: ContextClass,
    /// The authoring seat ("premiere", "dispatcher", … — the worklog's
    /// post-661 attribution).
    pub author: String,
    /// Unix seconds.
    pub ts: i64,
    pub text: String,
    /// Content hash over normalized text — the cross-class dedup key.
    pub hash: u64,
}

/// Upper bound on a single item's rendered chars. A rambling worklog row is
/// permanent, but one row must not eat a whole class quota.
///
/// ponytail: a constant, not config — raise deliberately when a real feed
/// needs longer items, not speculatively.
const MAX_ITEM_CHARS: usize = 400;

/// The seats whose outputs are org-global broadcasts by role.
const GLOBAL_AUTHORS: [&str; 2] = ["premiere", "chief-of-staff"];

/// ponytail: authority is a static precedence ladder for phase 1 (no Wave 3
/// department/authority model yet); the design doc's full ladder lands with
/// it. Premiere outranks the chief; everyone else is a peer.
fn authority_of(author: &str) -> f64 {
    match author {
        "premiere" => 1.0,
        "chief-of-staff" => 0.8,
        _ => 0.5,
    }
}

/// FNV-1a over lowercased, whitespace-collapsed text — cheap, stable, and
/// only used for identity comparison, never security.
fn content_hash(text: &str) -> u64 {
    let normalized: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in normalized.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The ranking score: recency decay + lexical affinity + author authority.
/// Thread participation is the design doc's fourth component — phase 1 has
/// no thread refs in items, so the weight applies to zero until phase 2.
pub fn score_item(
    item: &ContextItem,
    affinity: Option<&str>,
    halflife_hours: f64,
    weight_recency: f64,
    weight_lexical: f64,
    weight_authority: f64,
    now_unix: i64,
) -> f64 {
    let age_hours = ((now_unix - item.ts).max(0)) as f64 / 3600.0;
    let recency = 0.5f64.powf(age_hours / halflife_hours.max(0.5));
    let lexical = affinity
        .map(|text| lexical_overlap(&item.text, text))
        .unwrap_or(0.0);
    weight_recency * recency
        + weight_lexical * lexical
        + weight_authority * authority_of(&item.author)
}

/// Token-set overlap between item text and the affinity corpus (the seat's
/// charter), normalized to [0, 1]. Boring on purpose: no embeddings, no
/// model calls — the same in-process philosophy as memory-wire's recall.
fn lexical_overlap(item_text: &str, affinity: &str) -> f64 {
    let stop = |w: &str| {
        matches!(
            w,
            "the"
                | "a"
                | "an"
                | "and"
                | "or"
                | "of"
                | "to"
                | "in"
                | "is"
                | "for"
                | "on"
                | "with"
                | "as"
                | "at"
                | "by"
                | "it"
                | "this"
                | "that"
        )
    };
    let words = |t: &str| {
        t.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2 && !stop(w))
            .map(|w| w.to_lowercase())
            .collect::<std::collections::HashSet<_>>()
    };
    let iw = words(item_text);
    if iw.is_empty() {
        return 0.0;
    }
    let aw = words(affinity);
    if aw.is_empty() {
        return 0.0;
    }
    iw.intersection(&aw).count() as f64 / iw.len().max(aw.len()) as f64
}

/// The phase-1 injector: reads the worklog, ranks, quotas, renders, and
/// advances the per-seat watermarks. All state lives in the app database
/// (`context_watermarks` — table-not-a-file, the seat_policies discipline).
pub struct ContextInjector {
    app_db: PathBuf,
    settings: crate::config::ContextInjectionSettings,
}

impl ContextInjector {
    /// Open (idempotently creating the watermark table) against the app db.
    pub fn open(app_db: &Path, settings: crate::config::ContextInjectionSettings) -> Result<Self> {
        let conn = Connection::open(app_db)
            .map_err(|e| Error::Agent(format!("context injection: open: {e}")))?;
        conn.execute_batch(CONTEXT_WATERMARKS_SCHEMA)
            .map_err(|e| Error::Agent(format!("context injection: schema: {e}")))?;
        Ok(Self {
            app_db: app_db.to_path_buf(),
            settings,
        })
    }

    /// Render the bounded context section for `seat_id` and prepend it to
    /// `prompt`. Empty pipeline, disabled, or any store error → `prompt`
    /// byte-identical (the iter-666 contract).
    ///
    /// `affinity` is the relevance corpus — the seat's charter — and may be
    /// `None` (lexical component scores zero).
    pub fn render_section(&self, seat_id: &str, affinity: Option<&str>, prompt: &str) -> String {
        if !self.settings.enabled {
            return prompt.to_string();
        }
        let Ok(conn) = Connection::open(&self.app_db) else {
            return prompt.to_string(); // fail-open: injection never fails a run
        };
        let items = match self.collect(&conn, seat_id) {
            Ok(items) => items,
            Err(e) => {
                tracing::warn!("context injection collect failed: {e} (fail-open)");
                return prompt.to_string();
            }
        };
        if items.is_empty() {
            return prompt.to_string();
        }
        // Watermarks advance on collection (see the module docs for why
        // excluded-by-quota items must not stay pending forever).
        if let Err(e) = self.advance_watermarks(&conn, seat_id, &items) {
            tracing::warn!("context injection watermark write failed: {e} (fail-open)");
        }
        let rendered = self.render(items, affinity);
        if rendered.is_empty() {
            return prompt.to_string();
        }
        format!("{rendered}\n\n{prompt}")
    }

    /// Collect unseen items for the seat, one class at a time, then drop
    /// cross-class duplicates (DM form wins — the addressing context is
    /// the valuable part; then Global > Dept > Self).
    fn collect(&self, conn: &Connection, seat_id: &str) -> Result<Vec<ContextItem>> {
        let mut items: Vec<ContextItem> = Vec::new();
        for class in [
            ContextClass::Dm,
            ContextClass::Global,
            ContextClass::Dept,
            ContextClass::Self_,
        ] {
            let watermark = self.watermark(conn, seat_id, class)?;
            let rows = self.collect_class(conn, seat_id, class, watermark)?;
            items.extend(rows);
        }
        // Cross-class dedup, first-writer-wins (collection order above IS
        // the class priority, so a DM form survives over a feed form). A
        // hash claimed by one class drops only its OTHER-class copies —
        // identical rows within the SAME class are distinct events at
        // distinct timestamps (a repeated directive re-issued at 09:00 and
        // 10:00 is two events) and stay.
        let mut claimed_by: std::collections::HashMap<u64, ContextClass> =
            std::collections::HashMap::new();
        items.retain(|item| match claimed_by.get(&item.hash) {
            Some(owner) => *owner == item.class,
            None => {
                claimed_by.insert(item.hash, item.class);
                true
            }
        });
        Ok(items)
    }

    fn collect_class(
        &self,
        conn: &Connection,
        seat_id: &str,
        class: ContextClass,
        watermark: i64,
    ) -> Result<Vec<ContextItem>> {
        match class {
            // Phase 1: the org layer stores no DM bodies — reserved for the
            // phase-2 platform read adapters and the socialization sessions.
            ContextClass::Dm => Ok(Vec::new()),
            ContextClass::Global => {
                let mut stmt = conn
                    .prepare(
                        "SELECT employee, ts, what_done FROM worklog \
                         WHERE ts > ?1 AND employee IN (?2, ?3) AND employee != ?4 \
                         ORDER BY ts DESC LIMIT 20",
                    )
                    .map_err(|e| Error::Agent(format!("context injection: stmt: {e}")))?;
                let rows = stmt
                    .query_map(
                        params![watermark, GLOBAL_AUTHORS[0], GLOBAL_AUTHORS[1], seat_id],
                        |row| {
                            Ok(ContextItem {
                                class,
                                author: row.get(0)?,
                                ts: row.get(1)?,
                                text: row.get(2)?,
                                hash: 0,
                            })
                        },
                    )
                    .map_err(|e| Error::Agent(format!("context injection: query: {e}")))?;
                Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            }
            ContextClass::Dept => {
                // Wave 3 has not landed departments; a seat with no
                // department has no peers to read. The honest empty set.
                let dept: Option<String> = conn
                    .query_row(
                        "SELECT department FROM employees WHERE employee_id = ?1",
                        params![seat_id],
                        |row| row.get(0),
                    )
                    .ok();
                let Some(dept) = dept else {
                    return Ok(Vec::new());
                };
                let mut stmt = conn
                    .prepare(
                        "SELECT employee, ts, what_done FROM worklog \
                         WHERE ts > ?1 AND department = ?2 AND employee != ?3 \
                         ORDER BY ts DESC LIMIT 20",
                    )
                    .map_err(|e| Error::Agent(format!("context injection: stmt: {e}")))?;
                let rows = stmt
                    .query_map(params![watermark, dept, seat_id], |row| {
                        Ok(ContextItem {
                            class,
                            author: row.get(0)?,
                            ts: row.get(1)?,
                            text: row.get(2)?,
                            hash: 0,
                        })
                    })
                    .map_err(|e| Error::Agent(format!("context injection: query: {e}")))?;
                Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            }
            ContextClass::Self_ => {
                let mut stmt = conn
                    .prepare(
                        "SELECT employee, ts, what_done FROM worklog \
                         WHERE ts > ?1 AND employee = ?2 \
                         ORDER BY ts DESC LIMIT 20",
                    )
                    .map_err(|e| Error::Agent(format!("context injection: stmt: {e}")))?;
                let rows = stmt
                    .query_map(params![watermark, seat_id], |row| {
                        Ok(ContextItem {
                            class,
                            author: row.get(0)?,
                            ts: row.get(1)?,
                            text: row.get(2)?,
                            hash: 0,
                        })
                    })
                    .map_err(|e| Error::Agent(format!("context injection: query: {e}")))?;
                Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            }
        }
        .map(|items: Vec<ContextItem>| {
            items
                .into_iter()
                .map(|mut item| {
                    item.hash = content_hash(&item.text);
                    item
                })
                .collect()
        })
    }

    fn watermark(&self, conn: &Connection, seat_id: &str, class: ContextClass) -> Result<i64> {
        let ts: Option<i64> = conn
            .query_row(
                "SELECT last_ts FROM context_watermarks WHERE seat_id = ?1 AND class = ?2",
                params![seat_id, class.as_str()],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| Error::Agent(format!("context injection: watermark read: {e}")))?;
        Ok(ts.unwrap_or(0))
    }

    fn advance_watermarks(
        &self,
        conn: &Connection,
        seat_id: &str,
        items: &[ContextItem],
    ) -> Result<()> {
        for class in [
            ContextClass::Dm,
            ContextClass::Global,
            ContextClass::Dept,
            ContextClass::Self_,
        ] {
            let Some(max_ts) = items
                .iter()
                .filter(|item| item.class == class)
                .map(|item| item.ts)
                .max()
            else {
                continue;
            };
            conn.execute(
                "INSERT INTO context_watermarks (seat_id, class, last_ts)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(seat_id, class) DO UPDATE SET
                     last_ts = MAX(last_ts, excluded.last_ts)",
                params![seat_id, class.as_str(), max_ts],
            )
            .map_err(|e| Error::Agent(format!("context injection: watermark write: {e}")))?;
        }
        Ok(())
    }

    /// Rank within each class, fill the class quota, roll leftovers into a
    /// shared pool, and render under the total cap. Premiere-authored
    /// global items are directive-class: exempt from the class quota
    /// (subject only to the total cap) — the owner's directive slice.
    fn render(&self, mut items: Vec<ContextItem>, affinity: Option<&str>) -> String {
        let now = chrono::Utc::now().timestamp();
        let s = &self.settings;
        let quota_for = |class: ContextClass| match class {
            ContextClass::Dm => s.dm_quota,
            ContextClass::Global => s.global_quota,
            ContextClass::Dept => s.dept_quota,
            ContextClass::Self_ => s.self_quota,
        };
        let scored: Vec<(f64, ContextItem)> = items
            .into_iter()
            .map(|item| {
                let halflife = match item.class {
                    ContextClass::Dm => s.recency_halflife_hours_dm,
                    _ => s.recency_halflife_hours_feed,
                };
                let score = score_item(
                    &item,
                    affinity,
                    halflife,
                    s.weight_recency,
                    s.weight_lexical,
                    s.weight_authority,
                    now,
                );
                (score, item)
            })
            .collect();

        let mut chosen: Vec<ContextItem> = Vec::new();
        let mut pool: Vec<(f64, ContextItem)> = Vec::new();
        let mut used_total = 0usize;
        for class in [
            ContextClass::Dm,
            ContextClass::Global,
            ContextClass::Dept,
            ContextClass::Self_,
        ] {
            let mut class_items: Vec<(f64, ContextItem)> = scored
                .iter()
                .filter(|(_, item)| item.class == class)
                .cloned()
                .collect();
            class_items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut used_class = 0usize;
            for (score, item) in class_items {
                let text = truncate_chars(&item.text, MAX_ITEM_CHARS);
                let directive_exempt = class == ContextClass::Global && item.author == "premiere";
                if !directive_exempt && used_class + text.len() > quota_for(class) {
                    pool.push((score, item));
                    continue;
                }
                if used_total + text.len() > s.total_char_cap {
                    break;
                }
                used_class += text.len();
                used_total += text.len();
                chosen.push(item);
            }
        }
        // Roll-over pool: fill leftover total budget by global score.
        let mut pool = pool;
        pool.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        for (score, item) in pool {
            let text = truncate_chars(&item.text, MAX_ITEM_CHARS);
            let _ = score;
            if used_total + text.len() > s.total_char_cap {
                break;
            }
            used_total += text.len();
            chosen.push(item);
        }
        if chosen.is_empty() {
            return String::new();
        }
        chosen.sort_by_key(|item| (item.class as u8, -item.ts));

        let mut out = String::from("## Injected context (org feeds + DMs since your last cycle)");
        let mut current_class: Option<ContextClass> = None;
        for item in &chosen {
            if current_class != Some(item.class) {
                current_class = Some(item.class);
                out.push_str(&format!("\n### {}\n", class_header(item.class)));
            }
            out.push_str(&format!(
                "- [{}:{}] {}\n",
                item.author,
                chrono::DateTime::from_timestamp(item.ts, 0)
                    .map(|t| t.format("%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| item.ts.to_string()),
                truncate_chars(&item.text, MAX_ITEM_CHARS)
            ));
        }
        out
    }
}

fn class_header(class: ContextClass) -> &'static str {
    match class {
        ContextClass::Dm => "DMs addressed to you",
        ContextClass::Global => "Global feed (directives + org broadcasts)",
        ContextClass::Dept => "Department feed",
        ContextClass::Self_ => "Your previous cycles",
    }
}

/// Cut on a char boundary at `limit` chars.
fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.char_indices()
        .nth(limit)
        .map(|(i, _)| format!("{}…", &text[..i]))
        .unwrap_or_else(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> crate::config::ContextInjectionSettings {
        crate::config::ContextInjectionSettings::default()
    }

    fn db_with_worklog() -> (tempfile::TempDir, ContextInjector) {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = Connection::open(dir.path().join("app.sqlite")).expect("open");
        conn.execute_batch(
            "CREATE TABLE worklog (
                id TEXT PRIMARY KEY, ts INTEGER NOT NULL, ts_iso TEXT NOT NULL,
                employee TEXT NOT NULL, department TEXT, job_id TEXT,
                what_done TEXT NOT NULL, outcome TEXT NOT NULL
            );
            CREATE TABLE employees (employee_id TEXT PRIMARY KEY, department TEXT);",
        )
        .expect("schema");
        drop(conn);
        let injector = ContextInjector::open(
            &dir.path().join("app.sqlite"),
            crate::config::ContextInjectionSettings {
                self_quota: 300,
                ..settings()
            },
        )
        .expect("injector");
        (dir, injector)
    }

    fn log(conn: &Connection, employee: &str, ts: i64, text: &str) {
        conn.execute(
            "INSERT INTO worklog (id, ts, ts_iso, employee, what_done, outcome)
             VALUES (?1, ?2, 'x', ?3, ?4, 'success')",
            params![format!("id-{employee}-{ts}"), ts, employee, text],
        )
        .expect("insert worklog");
    }

    #[test]
    fn watermark_never_reinjects_a_consumed_row() {
        let (_dir, injector) = db_with_worklog();
        let conn = Connection::open(&injector.app_db).expect("conn");
        log(&conn, "dispatcher", 100, "audited the queue");
        drop(conn);

        let first = injector.render_section("dispatcher", None, "PROMPT");
        assert!(
            first.contains("audited the queue"),
            "first render must include it"
        );
        assert!(
            first.ends_with("PROMPT"),
            "prompt must ride under the section"
        );

        let second = injector.render_section("dispatcher", None, "PROMPT");
        assert_eq!(
            second, "PROMPT",
            "a consumed row must never re-inject — byte-identical"
        );
    }

    #[test]
    fn global_feed_reaches_other_seats_but_not_the_author() {
        let (_dir, injector) = db_with_worklog();
        let conn = Connection::open(&injector.app_db).expect("conn");
        log(&conn, "premiere", 100, "org directive: ship the wave");
        drop(conn);

        let warden = injector.render_section("identity-warden", None, "PROMPT");
        assert!(
            warden.contains("org directive"),
            "the global feed must reach other seats: {warden}"
        );
        let premiere_self = injector.render_section("premiere", None, "PROMPT");
        assert!(
            premiere_self.contains("org directive"),
            "for the author the same row is Self-class continuity (never a broadcast to itself), and it still renders: {premiere_self}"
        );
        // And the author's own render must label it under "Your previous
        // cycles", not the global feed — the class is a property of the
        // (reader, row) pair.
        assert!(
            premiere_self.contains("Your previous cycles"),
            "the author sees its own row as Self, not Global: {premiere_self}"
        );
    }

    #[test]
    fn premiere_directives_are_quota_exempt_subject_to_the_total() {
        let (_dir, injector) = db_with_worklog();
        let conn = Connection::open(&injector.app_db).expect("conn");
        // A directive longer than the whole global class quota (500).
        let long = "directive: ".to_string() + &"prioritise the rollout ".repeat(40);
        log(&conn, "premiere", 100, &long);
        drop(conn);

        let rendered = injector.render_section("identity-warden", None, "PROMPT");
        assert!(
            rendered.contains("prioritise the rollout"),
            "a premiere directive must not be dropped by the class quota"
        );
    }

    #[test]
    fn quota_fills_by_rank_and_rolls_leftovers_into_the_pool() {
        let (_dir, injector) = db_with_worklog();
        let conn = Connection::open(&injector.app_db).expect("conn");
        // Self quota is 300: three 400-char rows cannot all fit the class,
        // but the pool may take what the total cap (2000) still allows.
        let filler = "row ".to_string() + &"x".repeat(390);
        log(&conn, "dispatcher", 100, &filler);
        log(&conn, "dispatcher", 200, &filler);
        log(&conn, "dispatcher", 300, &filler);
        drop(conn);

        let rendered = injector.render_section("dispatcher", None, "PROMPT");
        let count = rendered.matches("row xxx").count();
        assert!(
            count >= 2 && count <= 5,
            "class quota holds ~1, the pool takes the rest under the total: got {count}"
        );
    }

    #[test]
    fn disabled_or_empty_is_byte_identical() {
        let (dir, _injector) = db_with_worklog();
        let injector = ContextInjector::open(
            &dir.path().join("app.sqlite"),
            crate::config::ContextInjectionSettings {
                enabled: false,
                ..settings()
            },
        )
        .expect("injector");
        assert_eq!(
            injector.render_section("dispatcher", None, "PROMPT"),
            "PROMPT",
            "disabled must be byte-identical"
        );

        let (_d2, injector2) = db_with_worklog(); // empty worklog
        assert_eq!(
            injector2.render_section("dispatcher", None, "PROMPT"),
            "PROMPT",
            "empty pipeline must be byte-identical"
        );
    }

    #[test]
    fn lexical_affinity_prefers_relevant_items_under_tie() {
        let now = chrono::Utc::now().timestamp();
        let relevant = ContextItem {
            class: ContextClass::Global,
            author: "chief-of-staff".into(),
            ts: now,
            text: "review the dispatcher queue audit charter".into(),
            hash: 0,
        };
        let irrelevant = ContextItem {
            class: ContextClass::Global,
            author: "chief-of-staff".into(),
            ts: now,
            text: "water the plants and rotate the snacks".into(),
            hash: 0,
        };
        let charter = "the dispatcher maintains the queue audit charter";
        let r = score_item(&relevant, Some(charter), 24.0, 1.0, 1.0, 0.5, now);
        let i = score_item(&irrelevant, Some(charter), 24.0, 1.0, 1.0, 0.5, now);
        assert!(
            r > i,
            "affinity must rank the relevant item higher: {r} vs {i}"
        );
    }

    #[test]
    fn content_hash_matches_whitespace_and_case_variants_only() {
        assert_eq!(
            content_hash("Ship the  Wave"),
            content_hash("ship the wave"),
            "case and whitespace are normalized"
        );
        assert_ne!(
            content_hash("ship the wave"),
            content_hash("ship different orders"),
            "different content hashes differently"
        );
    }

    #[test]
    fn truncate_cuts_on_a_char_boundary() {
        let cut = truncate_chars("héllo wörld", 5);
        assert_eq!(cut, "héllo…");
        assert_eq!(truncate_chars("short", 10), "short");
    }
}
