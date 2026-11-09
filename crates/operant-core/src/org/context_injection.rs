//! DM/feed context injection — phase 1 (the cron seam).
//!
//! Design: `docs/plan-2026-10-08-dm-feed-context-injection.md` — approved
//! 2026-10-08 with the owner's rulings: per-aspect character quotas (the
//! MEMORY.md budget model, applied to four aspect classes) and re-ranking
//! across the classes (recency + lexical affinity + author authority).
//!
//! DM/feed context injection — what the pipeline LIVEly injects is the
//! channel-sourced `dm` / `feed` classes only (RT-372).
//!
//! RETIRED SOURCES (G+1, `docs/agentic-os/`): the org-internal classes —
//! `Self` (the seat's own worklog rows), `Global` (premiere / chief-of-staff
//! rows) and `Dept` (same-department peers) — were RETIRED to Holon: the
//! collector returns `Ok(Vec::new())` for all three and no worklog table is
//! queried anywhere in this file. The header used to describe those three as
//! the live phase-1 sources ("everything here is data the organism already
//! persists (the worklog)") and to promise the seats' cross-visibility;
//! `crate::org`'s own module doc already records the retirement, and this
//! header now agrees with it. Only `Dm`/`Feed` can produce an item.
//!
//! Nonredundancy is the watermark backbone: a per-seat, per-class monotonic
//! cursor in `context_watermarks`. Items at/below the cursor are never
//! collected again; advancing happens on collection (the ranking's judgment
//! that a low-score item is not worth showing must not leave it pending
//! forever, crowding out later items — at-least-once semantics plus the
//! render-time content-hash dedup make a re-collect harmless).
//!
//! RT-425: the per-class query takes the OLDEST pending items (up to its
//! limit), not the newest. It used to take the newest, and the cursor then
//! jumped to their max — so a class with more pending items than the limit
//! left the OLDER ones below the cursor forever: never collected, never
//! rendered, never seen. Collecting oldest-first walks the cursor through
//! the backlog instead of over it, which is what the at-least-once promise
//! above requires.
//!
//! Failure posture: any store error degrades to "no injection" and the
//! prompt is returned byte-identical — the iter-666 contract. Injection
//! must never fail a run.
//!
//! ponytail: the pipeline reads the `context_items` table it owns rather
//! than copying the worklog into it (the design doc's §5 store is deferred to
//! phase 2, where pull-once platform reads genuinely need a durable landing).
//! Same pipeline, less code, no schema duplication of rows the worklog
//! already keeps. (The earlier note said "phase 1 reads the worklog
//! directly" — true of the RETIRED org classes, and corrected by RT-372.)

use crate::error::{Error, Result};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

/// Per-seat, per-class read cursors — the nonredundancy backbone.
pub const CONTEXT_WATERMARKS_SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS context_watermarks (
        seat_id TEXT NOT NULL,
        class   TEXT NOT NULL,   -- dm | feed (the live classes; global/dept/self are retired)
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
        class        TEXT NOT NULL,   -- dm | feed (both live)
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

/// How many pending items one class contributes per collection.
///
/// RT-425: a class with more pending items than this used to SKIP the
/// older ones permanently — the query took the NEWEST 20 and the watermark
/// then jumped to their max, so the rest fell below the cursor and were
/// never collected again. The query now takes the OLDEST 20, so the cursor
/// advances through the backlog and every pending item is collected exactly
/// once (the "at-least-once" the module doc promises).
const COLLECT_LIMIT: usize = 20;

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

/// The phase-1 injector: reads `context_items` (dm/feed), ranks, quotas,
/// renders, and advances the per-seat watermarks.
///
/// RT-436: this used to place the state in the app database. It is not
/// there: `open` resolves the ORG SIBLING
/// (`operant_kanban.db` beside the app database) — the "one sibling
/// kanban file" rule — and the field that holds the path was named
/// after the APP database, so the doc and the field name both pointed a
/// reader at the wrong file. That is the exact "invisible no-op" `open`'s
/// own doc recounts: a reader looking in the app database finds nothing.
///
/// The state is still table-not-a-file (the seat_policies discipline); it
/// is simply in the ORG database.
pub struct ContextInjector {
    /// The ORG sibling database (`operant_kanban.db`) — see `open`.
    db_path: PathBuf,
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
            db_path: org_sibling,
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
        let Ok(conn) = Connection::open(&self.db_path) else {
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
        let Ok(conn) = Connection::open(&self.db_path) else {
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
                    .prepare(&format!(
                        "SELECT author, ts, text FROM context_items \
                             WHERE class = 'dm' AND seat_hint = ?1 AND ts > ?2 \
                             ORDER BY ts ASC LIMIT {COLLECT_LIMIT}"
                    ))
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
                    .prepare(&format!(
                        "SELECT author, ts, text FROM context_items \
                             WHERE class = 'feed' AND seat_hint = ?1 AND ts > ?2 \
                             ORDER BY ts ASC LIMIT {COLLECT_LIMIT}"
                    ))
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
            let Some(new_ts) = items
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
                params![seat_id, class.as_str(), new_ts],
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
                // RT-424: the quotas are CHARACTERS (`total_char_cap` is
                // documented "Hard ceiling for the whole rendered section,
                // chars"; the per-aspect quotas are "char quotas"), but the
                // accounting charged `text.len()` — BYTES. Non-ASCII text was
                // charged up to 4x its character count, so a DM that fits the
                // 800-char quota in characters overflowed it in bytes and
                // was pooled or dropped.
                let text_chars = text.chars().count();
                let directive_exempt = class == ContextClass::Global && item.author == "premiere";
                if !directive_exempt && used_class + text_chars > quota_for(class) {
                    pool.push((score, item));
                    continue;
                }
                if used_total + text_chars > s.total_char_cap {
                    // `continue`, not `break`: items are ranked by SCORE, not
                    // length, so abandoning the rest of the class on one long
                    // item starved shorter items that still fit the remaining
                    // total budget (RT-327). The roll-over pool cannot
                    // recover them — only quota-overflow items are pooled,
                    // and an item that exceeds the total cap here exceeds it
                    // there too.
                    continue;
                }
                used_class += text_chars;
                used_total += text_chars;
                chosen.push(item);
            }
        }
        // Roll-over pool: fill leftover total budget by global score.
        pool.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        for (score, item) in pool {
            let text = truncate_chars(&item.text, MAX_ITEM_CHARS);
            let _ = score;
            // RT-424: chars, not bytes — the cap is documented in chars.
            let text_chars = text.chars().count();
            if used_total + text_chars > s.total_char_cap {
                break;
            }
            used_total += text_chars;
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

mod rt372_tests {
    /// RT-372: the module header described three org classes (`Self`,
    /// `Global`, `Dept`) as the live phase-1 sources reading the worklog, while
    /// the collector returns `Ok(Vec::new())` for all three ("retired to
    /// Holon") and `crate::org`'s own module doc records the retirement — the
    /// two docs contradicted. Source contract, needles concatenation-built.
    #[test]
    fn the_injection_header_names_the_retired_classes() {
        let src = include_str!("context_injection.rs");
        let retired = format!("RETIRED SOURCES (G+1, {}", "`docs/agentic-os/`)");
        assert!(
            src.contains(&retired),
            "the header must name the retired sources, not list them as live"
        );
        let stale_claim = format!("Phase 1 sources are org-internal {}only", "");
        assert!(
            !src.contains(&stale_claim),
            "the header must not claim the org classes are the live sources"
        );
        // The two module docs must now agree.
        let org_mod = include_str!("mod.rs");
        let retirement = format!("retired to {}Holon", "");
        assert!(
            org_mod.contains(&retirement) && src.contains(&retirement),
            "the sibling doc and this header must both carry the retirement"
        );
    }
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

    /// RT-327: an item that FITS the remaining total budget used to be
    /// silently dropped. The class loop `break`ed on the first item that
    /// exceeded the total cap, abandoning the rest of the class — and those
    /// items never reached the roll-over pool (only quota-overflow items are
    /// pooled), so the pool could not recover them. Items rank by SCORE, not
    /// length, so one long high-score item starved short items that fit.
    #[test]
    fn a_short_item_still_renders_when_a_longer_one_exceeds_the_total_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let injector = ContextInjector::open(
            &dir.path().join("app.sqlite"),
            crate::config::ContextInjectionSettings {
                // The long item truncates to MAX_ITEM_CHARS (400), so the
                // cap must sit below that for it to genuinely not fit.
                total_char_cap: 300,
                dm_quota: 5_000, // the class quota must not be the constraint
                ..settings()
            },
        )
        .expect("injector");
        let now = chrono::Utc::now().timestamp();
        let long_text = "L".repeat(600);
        let short_text = "S".repeat(100);
        let items = vec![
            // The LONG item ranks FIRST (a 24h halflife makes the newer item
            // score clearly higher), so a `break` on it starves the short one
            // even though 400 chars of total budget remain.
            ContextItem {
                class: ContextClass::Dm,
                author: "peer".into(),
                ts: now,
                text: long_text.clone(),
                hash: 1,
            },
            ContextItem {
                class: ContextClass::Dm,
                author: "peer".into(),
                ts: now - 86_400,
                text: short_text.clone(),
                hash: 2,
            },
        ];
        let rendered = injector.render(items, None);
        assert!(
            rendered.contains(&short_text),
            "the short item must render — the long one only breaks ITS OWN slot: {rendered}"
        );
        assert!(
            !rendered.contains(&long_text),
            "the long item genuinely cannot fit the total cap"
        );
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
        let conn = Connection::open(&injector.db_path).expect("conn");
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

#[cfg(test)]
mod rt437_tests {
    use super::*;

    /// RT-437: the two schema comments named class sets that were neither
    /// live nor retired-and-honest: the watermarks' comment listed
    /// `dm | global | dept | self` (the last three are RETIRED, and the live
    /// `feed` was missing) and the items' comment called `feed` a "future"
    /// source — it is live: the gateway taps feed posts into the table, the
    /// feed collector reads them, and a feed watermark row is written.
    ///
    /// BEHAVIORAL: a feed item and its watermark row are written today, and
    /// the comments name exactly the live classes.
    #[test]
    fn the_schema_comments_name_the_live_classes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let injector = ContextInjector::open(
            &dir.path().join("database.db"),
            crate::config::ContextInjectionSettings::default(),
        )
        .expect("injector");
        injector.record_feed_item("seat-1", "peer", "a feed post");
        let conn = Connection::open(dir.path().join("operant_kanban.db")).expect("open");
        let items: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM context_items WHERE class = 'feed'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(items, 1, "the feed class is live — a feed item is written");
    }

    #[test]
    fn the_schema_comments_do_not_call_the_feed_class_future() {
        let src = include_str!("context_injection.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        let future = concat!("future: feed", "");
        assert_eq!(
            prod.matches(future).count(),
            0,
            "the feed class is live -- the items schema must not call it future"
        );
        let retired_classes = concat!("dm | global | dept | self", "");
        assert_eq!(
            prod.matches(retired_classes).count(),
            0,
            "the watermarks schema must not list the retired classes as live"
        );
        // Both comments name the live pair.
        assert_eq!(
            prod.matches("dm | feed").count(),
            2,
            "both schema comments must name the live classes"
        );
    }
}

#[cfg(test)]
mod rt436_tests {
    use super::ContextInjector;

    /// RT-436: the injector's doc said "All state lives in the app
    /// database" — it lives in the ORG SIBLING (`operant_kanban.db` beside
    /// the app database, the "one sibling kanban file" rule). A reader
    /// looking in the app database finds nothing, which is the exact
    /// "invisible no-op" `open`'s own doc recounts.
    ///
    /// SOURCE contract: the location is not a behaviour to measure, so what
    /// is pinned is that the doc and the FIELD name both say where the
    /// state lives. The path itself is asserted behaviorally below.
    #[test]
    fn the_injector_doc_and_field_name_the_org_sibling() {
        let src = include_str!("context_injection.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        let app_db_claim = concat!("All state lives in the app", " database");
        assert_eq!(
            prod.matches(app_db_claim).count(),
            0,
            "the doc must not send a reader to the app database"
        );
        let struct_doc = concat!("the ORG SIBLING", "");
        assert_eq!(
            prod.matches(struct_doc).count(),
            1,
            "the struct's doc must name the ORG SIBLING"
        );
        let field_doc = concat!("The ORG sibling database", "");
        assert_eq!(
            prod.matches(field_doc).count(),
            1,
            "the field that holds the path must say which database it is"
        );
        // The field that holds the path must not be named after the app
        // database: that misnomer is what made the doc believable.
        assert_eq!(
            prod.matches("app_db").count(),
            0,
            "no field may carry the app-database misnomer"
        );
    }

    /// RT-436: and the path really is the org sibling — measured, not read
    /// off a comment.
    #[test]
    fn the_injector_opens_the_org_sibling_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_db = dir.path().join("database.db");
        let injector =
            ContextInjector::open(&app_db, crate::config::ContextInjectionSettings::default())
                .expect("injector");
        let org_sibling = dir.path().join("operant_kanban.db");
        assert!(
            org_sibling.exists(),
            "the org sibling must be the file the injector created"
        );
        assert!(
            !app_db.exists(),
            "the app database must NOT be the injector's file"
        );
    }
}

#[cfg(test)]
mod rt424_tests {
    use super::*;

    /// RT-424: the quotas are CHARACTERS — `total_char_cap` is documented
    /// "Hard ceiling for the whole rendered section, chars", the per-aspect
    /// quotas are "char quotas" — but the accounting charged `text.len()`,
    /// which is BYTES. Non-ASCII text was charged up to 4x its character
    /// count.
    ///
    /// BEHAVIORAL: a DM of 40 four-byte characters (160 bytes) fits a
    /// 60-character quota, so it must be rendered. Under the byte
    /// accounting it overflowed the class quota, went to the roll-over
    /// pool, and then overflowed the total cap too — so it was DROPPED and
    /// the section rendered EMPTY.
    ///
    /// What this pins: the COMBINED accounting. A partial revert (only the
    /// class-quota charge in bytes) is masked by the pool, which charges the
    /// same unit and would re-admit the item — the rendered output cannot
    /// tell "admitted by the class quota" from "admitted by the pool".
    #[test]
    fn a_non_ascii_dm_is_charged_in_characters_not_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = crate::config::ContextInjectionSettings {
            dm_quota: 60,
            total_char_cap: 60,
            ..crate::config::ContextInjectionSettings::default()
        };
        let injector =
            ContextInjector::open(&dir.path().join("app.sqlite"), settings).expect("injector");

        let text = "🙂".repeat(40);
        assert_eq!(text.chars().count(), 40, "the fixture is 40 CHARACTERS");
        assert_eq!(
            text.len(),
            160,
            "and 160 BYTES — the two must not be confused"
        );

        let item = ContextItem {
            class: ContextClass::Dm,
            author: "peer".into(),
            ts: chrono::Utc::now().timestamp(),
            text,
            hash: 1,
        };
        let rendered = injector.render(vec![item], None);
        assert!(
            rendered.contains("🙂"),
            "a 40-character DM fits a 60-character quota and must be rendered; \
             got: {rendered}"
        );
    }
}

#[cfg(test)]
mod rt425_tests {
    use super::*;

    /// RT-425: the collector's per-class LIMIT used to SKIP the older
    /// pending items permanently — the query took the newest 20 and the
    /// watermark then jumped to their max, so the rest fell below the cursor
    /// and were never collected again, despite the module doc's
    /// "at-least-once semantics".
    ///
    /// BEHAVIORAL: 25 DMs are recorded, and the collection is driven until
    /// every one of them has been seen. The pre-fix cursor jumps straight to
    /// the newest DM, so the older five are never collected.
    #[test]
    fn a_backlog_larger_than_the_limit_is_fully_collected_over_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = crate::config::ContextInjectionSettings {
            dm_quota: 100_000,
            total_char_cap: 100_000,
            ..crate::config::ContextInjectionSettings::default()
        };
        let injector =
            ContextInjector::open(&dir.path().join("app.sqlite"), settings).expect("injector");

        // 25 DMs with distinct timestamps, oldest first. The rows go in
        // directly (the capture tap stamps "now", so a test needs its own
        // timestamps) into the ORG SIBLING the injector opened.
        let base = chrono::Utc::now().timestamp() - 100;
        {
            let conn = Connection::open(dir.path().join("operant_kanban.db")).expect("open");
            for i in 0..25i64 {
                let text = format!("dm-{i}");
                conn.execute(
                    "INSERT INTO context_items (class, author, ts, content_hash, seat_hint, text) \
                     VALUES ('dm', 'peer', ?1, ?2, 'seat-1', ?3)",
                    rusqlite::params![base + i, content_hash(&text) as i64, text],
                )
                .expect("insert");
            }
        }

        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        // Drive the collection the way a prompt cycle does.
        for _ in 0..10 {
            let rendered = injector.render_section("seat-1", None, "PROMPT", &[]);
            for i in 0..25i64 {
                if rendered.contains(&format!("dm-{i}")) {
                    seen.insert(i);
                }
            }
            if seen.len() == 25 {
                break;
            }
        }
        assert_eq!(
            seen.len(),
            25,
            "every pending DM must be collected eventually — the older ones must \
             not fall below the cursor unseen (collected {} of 25)",
            seen.len()
        );
    }
}
