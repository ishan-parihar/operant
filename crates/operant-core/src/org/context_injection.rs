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
use rusqlite::{Connection, params};
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

/// The durable landing for CAPTURED inbound items (phase 2, design doc §5).
/// The Telegram Bot API has no history-read for bots — a DM can be seen
/// exactly once, at the moment the gateway receives it — so phase 2's DM
/// class is capture-at-inbound, not pull: the gateway taps each inbound
/// message into this table, and the Dm collector reads it like any other
/// source. Append-only; the watermarks govern re-reads.
pub const CONTEXT_ITEMS_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS context_items (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        class        TEXT NOT NULL,   -- dm (future: feed sources)
        author       TEXT NOT NULL,   -- platform username / sender label
        ts           INTEGER NOT NULL,  -- unix seconds
        content_hash INTEGER NOT NULL,   -- cross-class dedup key
        seat_hint    TEXT NOT NULL,   -- the seat the item is addressed to
        text         TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_context_items_seat_class_ts
        ON context_items(seat_hint, class, ts);
    -- iter-720: the per-capture retention prune is `DELETE ... WHERE ts < ?`;
    -- without a bare-ts index that is a full table scan on every inbound
    -- message (quadratic on an active channel).
    CREATE INDEX IF NOT EXISTS idx_context_items_ts
        ON context_items(ts);
"#;

/// The four aspect classes (the owner's list, in render order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextClass {
    Dm,
    /// Channel/group posts captured at route time (gap 5 phase 2's feed
    /// sources — the schema's long-reserved non-DM class).
    Feed,
    Global,
    Dept,
    Self_,
}

impl ContextClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::Feed => "feed",
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

/// Capture retention (days). `context_items` is append-only by design
/// (watermarks are the read cursor, not a TTL) — but an active group
/// writes a row per member message, so each capture prunes rows older
/// than this. 7 days covers the feed recency halflife (24h) with the
/// seat's whole week reviewable. KNOWN LOSS (iter-720 review): the
/// watermark does NOT protect an offline seat — a seat that does not
/// run for more than these 7 days has its queued DMs/feed rows pruned
/// before its watermark ever advances past them; they are deleted
/// unread. Raise (or split per class, longer for dm) when a real seat
/// demonstrably needs that offline depth.
/// ponytail: constant, not config — raise when a seat demonstrably needs
/// a longer memory, not speculatively.
const CAPTURE_RETENTION_DAYS: i64 = 7;

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
/// only used for identity comparison, never security. Public so a caller
/// can pre-compute exclusion hashes (the gateway's transcript check:
/// never echo the inbound message back as an injected item).
pub fn content_hash(text: &str) -> u64 {
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
    /// The effective settings (feed seat routing reads this).
    pub fn settings(&self) -> &crate::config::ContextInjectionSettings {
        &self.settings
    }

    /// Open (idempotently creating the watermark table) against the app
    /// db's ORG SIBLING — `worklog_db_path`/`org_db_path`'s "one sibling
    /// kanban file" rule: the worklog, the employees registry, and the
    /// `context_watermarks` cursors are all org-family rows in
    /// `operant_kanban.db`, NOT the app `database.db`. (Found in live
    /// verification 2026-10-08: the first deployment opened the app db,
    /// the worklog query failed, and fail-open correctly produced
    /// byte-identical prompts — an invisible no-op. This is the fix.)
    pub fn open(
        database_path: &Path,
        settings: crate::config::ContextInjectionSettings,
    ) -> Result<Self> {
        // G+1: the org-sibling rule inlined (worklog_db retired): the
        // context tables live in operant_kanban.db beside the app db.
        let org_sibling = database_path
            .parent()
            .map(|p| p.join("operant_kanban.db"))
            .unwrap_or_else(|| std::path::PathBuf::from("operant_kanban.db"));
        let conn = Connection::open(&org_sibling)
            .map_err(|e| Error::Agent(format!("context injection: open: {e}")))?;
        conn.execute_batch(CONTEXT_WATERMARKS_SCHEMA)
            .map_err(|e| Error::Agent(format!("context injection: schema: {e}")))?;
        conn.execute_batch(CONTEXT_ITEMS_SCHEMA)
            .map_err(|e| Error::Agent(format!("context injection: items schema: {e}")))?;
        Ok(Self {
            app_db: org_sibling,
            settings,
        })
    }

    /// Render the bounded context section for `seat_id` and prepend it to
    /// `prompt`. Empty pipeline, disabled, or any store error → `prompt`
    /// byte-identical (the iter-666 contract).
    ///
    /// `affinity` is the relevance corpus — the seat's charter — and may be
    /// `None` (lexical component scores zero). `exclude` is the transcript
    /// check: content hashes of messages the live conversation ALREADY
    /// carries (the inbound DM chiefly); matching items never render.
    pub fn render_section(
        &self,
        seat_id: &str,
        affinity: Option<&str>,
        prompt: &str,
        exclude: &[u64],
    ) -> String {
        if !self.settings.enabled {
            return prompt.to_string();
        }
        let Ok(conn) = Connection::open(&self.app_db) else {
            return prompt.to_string(); // fail-open: injection never fails a run
        };
        let items = match self.collect(&conn, seat_id, exclude) {
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

    /// G+1: the notice board retired to Holon (`api_messages`). The
    /// block is gone; callers of the old surface get an empty string —
    /// prompt byte-identical.
    #[expect(dead_code)]
    pub fn pending_notices_block(&self, _employee_id: &str) -> String {
        String::new()
    }

    /// Collect unseen items for the seat, one class at a time, drop the
    /// transcript-check exclusions, then drop cross-class duplicates (DM
    /// form wins — the addressing context is the valuable part; then
    /// Global > Dept > Self).
    fn collect(
        &self,
        conn: &Connection,
        seat_id: &str,
        exclude: &[u64],
    ) -> Result<Vec<ContextItem>> {
        let mut items: Vec<ContextItem> = Vec::new();
        for class in [
            ContextClass::Dm,
            ContextClass::Feed,
            ContextClass::Global,
            ContextClass::Dept,
            ContextClass::Self_,
        ] {
            let watermark = self.watermark(conn, seat_id, class)?;
            let rows = self.collect_class(conn, seat_id, class, watermark)?;
            items.extend(rows);
        }
        // Transcript check: the live conversation already carries these
        // items (the inbound message chiefly) — never echo them back.
        if !exclude.is_empty() {
            items.retain(|item| !exclude.contains(&item.hash));
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

    /// Phase 2 (design doc §5, capture-at-inbound realization): record an
    /// inbound DM addressed to a seat — the gateway taps every inbound
    /// message here. Fail-open: a failed capture never fails the turn.
    pub fn record_inbound_dm(&self, seat_id: &str, author: &str, text: &str) {
        self.record_item(ContextClass::Dm, seat_id, author, text, "inbound DM");
    }

    /// Phase 2 feed sources (plan-2026-10-09 §3): record a channel/group
    /// post routed to a seat — the gateway taps these at route time, before
    /// any turn or admin gate. Same fail-open contract as the DM tap.
    pub fn record_feed_item(&self, seat_id: &str, author: &str, text: &str) {
        self.record_item(ContextClass::Feed, seat_id, author, text, "feed post");
    }

    /// Shared capture body: one INSERT into `context_items`, fail-open on
    /// every error — a failed capture must never fail a turn or a route.
    fn record_item(
        &self,
        class: ContextClass,
        seat_id: &str,
        author: &str,
        text: &str,
        what: &str,
    ) {
        if !self.settings.enabled {
            return;
        }
        let Ok(conn) = Connection::open(&self.app_db) else {
            return;
        };
        let text = crate::agent::safe_truncate_str(text.trim(), 800);
        if text.is_empty() {
            return;
        }
        if let Err(e) = conn.execute(
            "INSERT INTO context_items (class, author, ts, content_hash, seat_hint, text)\n             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                class.as_str(),
                author,
                chrono::Utc::now().timestamp(),
                content_hash(text) as i64,
                seat_id,
                text
            ],
        ) {
            tracing::warn!("context injection: {what} capture failed: {e} (fail-open)");
            return;
        }
        // Write-through prune (review finding 2026-10-09): the table is
        // append-only and nothing else ever deletes — an active group
        // writes a row per member message, which on this storage-watched
        // box is an unbounded table. One best-effort DELETE per capture
        // bounds it to CAPTURE_RETENTION_DAYS; the capture itself already
        // succeeded, so a prune failure is logged and swallowed.
        let cutoff = chrono::Utc::now().timestamp() - CAPTURE_RETENTION_DAYS * 86_400;
        if let Err(e) = conn.execute("DELETE FROM context_items WHERE ts < ?1", [cutoff]) {
            tracing::warn!("context injection: retention prune failed: {e} (fail-open)");
        }
    }

    fn collect_class(
        &self,
        conn: &Connection,
        seat_id: &str,
        class: ContextClass,
        watermark: i64,
    ) -> Result<Vec<ContextItem>> {
        match class {
            // Phase 2 (capture-at-inbound): the durable store the gateway
            // taps inbound DMs into — the design doc's §5 landing, realized
            // here because the Bot API gives a bot exactly one look at a DM.
            ContextClass::Dm => {
                let mut stmt = conn
                    .prepare(
                        "SELECT author, ts, text FROM context_items \
                         WHERE class = 'dm' AND seat_hint = ?1 AND ts > ?2 \
                         ORDER BY ts DESC LIMIT 20",
                    )
                    .map_err(|e| Error::Agent(format!("context injection: stmt: {e}")))?;
                let rows = stmt
                    .query_map(params![seat_id, watermark], |row| {
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
            // Phase 2 feed sources: channel/group posts captured at route
            // time (plan-2026-10-09 §3). Same durable landing as DMs — the
            // Bot API gives the poll one look at each post.
            ContextClass::Feed => {
                let mut stmt = conn
                    .prepare(
                        "SELECT author, ts, text FROM context_items \
                         WHERE class = 'feed' AND seat_hint = ?1 AND ts > ?2 \
                         ORDER BY ts DESC LIMIT 20",
                    )
                    .map_err(|e| Error::Agent(format!("context injection: stmt: {e}")))?;
                let rows = stmt
                    .query_map(params![seat_id, watermark], |row| {
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
            ContextClass::Global | ContextClass::Dept | ContextClass::Self_ => {
                // G+1 (docs/agentic-os/): the worklog/employee/notice sources
                // retired to Holon — the brief and Holon memory carry org
                // context in a Holon run. Channel-sourced aspects (dm, feed)
                // are the whole pipeline in personal mode too.
                Ok(Vec::new())
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
            ContextClass::Feed,
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
    fn render(&self, items: Vec<ContextItem>, affinity: Option<&str>) -> String {
        let now = chrono::Utc::now().timestamp();
        let s = &self.settings;
        let quota_for = |class: ContextClass| match class {
            ContextClass::Dm => s.dm_quota,
            ContextClass::Feed => s.feed_quota,
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
            ContextClass::Feed,
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
        ContextClass::Feed => "Channel feed",
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
    fn injector_db() -> (tempfile::TempDir, ContextInjector) {
        let dir = tempfile::tempdir().expect("tempdir");
        // G+1: the injector opens (and creates) its own context tables in
        // the org-sibling file; the fixture only supplies the paths.
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

    #[test]
    fn disabled_or_empty_is_byte_identical() {
        let (dir, _injector) = injector_db();
        let injector = ContextInjector::open(
            &dir.path().join("app.sqlite"),
            crate::config::ContextInjectionSettings {
                enabled: false,
                ..settings()
            },
        )
        .expect("injector");
        assert_eq!(
            injector.render_section("dispatcher", None, "PROMPT", &[]),
            "PROMPT",
            "disabled must be byte-identical"
        );

        let (_d2, injector2) = injector_db(); // empty worklog
        assert_eq!(
            injector2.render_section("dispatcher", None, "PROMPT", &[]),
            "PROMPT",
            "empty pipeline must be byte-identical"
        );
    }

    #[test]
    fn captured_inbound_dm_reaches_the_seat_and_not_others() {
        let (_dir, injector) = injector_db();
        injector.record_inbound_dm(
            "identity-warden",
            "ishan",
            "Please check the registry binding tonight",
        );
        let warden = injector.render_section("identity-warden", None, "PROMPT", &[]);
        assert!(
            warden.contains("registry binding"),
            "the captured DM must reach its seat: {warden}"
        );
        assert!(
            warden.contains("DMs addressed to you"),
            "it renders under the Dm class: {warden}"
        );
        assert!(
            !injector
                .render_section("dispatcher", None, "PROMPT", &[])
                .contains("registry binding"),
            "another seat's DM must not leak"
        );
        // The watermark holds: a consumed DM never re-renders.
        assert_eq!(
            injector.render_section("identity-warden", None, "PROMPT", &[]),
            "PROMPT"
        );
    }

    #[test]
    fn capture_prunes_rows_past_the_retention_window() {
        // Review finding 2026-10-09: context_items is append-only and a
        // group writes a row per member message — the write-through prune
        // in record_item is the only bound. A row older than the retention
        // window must disappear on the NEXT capture; a fresh row survives.
        let (_dir, injector) = injector_db();
        let conn = Connection::open(&injector.app_db).expect("conn");
        let ancient = chrono::Utc::now().timestamp() - 30 * 86_400;
        conn.execute(
            "INSERT INTO context_items (class, author, ts, content_hash, seat_hint, text)\n             VALUES ('feed', 'old-timer', ?1, 1, 'premiere', 'stale group chatter')",
            [ancient],
        )
        .expect("seed ancient row");
        // The next capture (any class) prunes it.
        injector.record_inbound_dm("premiere", "ishan", "fresh dm");
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM context_items WHERE text = 'stale group chatter'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(remaining, 0, "the ancient row must be pruned on capture");
        let fresh: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM context_items WHERE text = 'fresh dm'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(fresh, 1, "the fresh capture must survive its own prune");
    }

    #[test]
    fn captured_feed_post_reaches_its_seat_and_not_others() {
        let (_dir, injector) = injector_db();
        injector.record_feed_item(
            "premiere",
            "Engineering Notes",
            "channel post: v0.2.2 release cut is live",
        );
        let premiere = injector.render_section("premiere", None, "PROMPT", &[]);
        assert!(
            premiere.contains("v0.2.2 release cut"),
            "the captured feed post must reach its seat: {premiere}"
        );
        assert!(
            premiere.contains("Channel feed"),
            "it renders under the Feed class: {premiere}"
        );
        assert!(
            !injector
                .render_section("dispatcher", None, "PROMPT", &[])
                .contains("v0.2.2 release cut"),
            "another seat's feed must not leak"
        );
        // The watermark holds: a consumed feed post never re-renders.
        assert_eq!(
            injector.render_section("premiere", None, "PROMPT", &[]),
            "PROMPT"
        );
    }
}
