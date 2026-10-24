//! Cron schedule validation and next-run computation.
//!
//! Two schedule families, with different next-run semantics:
//!
//!   * **Cron expressions** (5-field `"0 9 * * *"` or 6-field
//!     `"0 0 9 * * *"`) are calendar rules evaluated in **UTC** — `0 9`
//!     fires at 09:00 UTC. 5-field input is canonicalized to the 6-field
//!     form the `cron` crate parses, and that canonical form is what gets
//!     persisted.
//!   * **Interval phrases** (`"every N<s|m|h|d|w>"`) are **durations**, not
//!     calendar rules: next run is exactly `period` after the previous
//!     computation. The phrase is persisted RAW — never rewritten into a
//!     cron day-of-month step. (ponytail audit 2026-10-24: the old
//!     expansion turned "every 48h" into `day-of-month */2`, a calendar
//!     step that fired Jan 31 then Feb 1, and "every 2w" grew a 3-day gap
//!     across February. The agent-side `cron_tool::compute_first_run`
//!     always had the duration semantics; this module now agrees with it.)
//!
//! Callers: `cmd_cron create`, `cmd_cron blueprint` and `suggestions
//! accept` validate/normalize before persisting; the scheduler and the DB
//! self-heal paths recompute through [`next_run_from_schedule`], so every
//! consumer applies the same rule.

use crate::error::Error;
use cron::Schedule;
use std::str::FromStr;

/// Validate a user-supplied schedule string and canonicalize it for
/// storage. Errors when nothing usable can be produced.
///
/// Returns the 6-field cron form for cron input, and the trimmed phrase
/// unchanged for interval input — the phrase IS the canonical form, because
/// rewriting it to a cron expression would change its meaning (see the
/// module doc).
pub fn normalize_schedule(input: &str) -> Result<String, Error> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(Error::Agent("empty cron schedule".to_string()));
    }
    // Already parseable (6-field or anything the cron crate accepts).
    if Schedule::from_str(trimmed).is_ok() {
        return Ok(trimmed.to_string());
    }
    // 5-field expression → prepend seconds.
    if let Ok(normalized) = five_to_six(trimmed)
        && Schedule::from_str(&normalized).is_ok()
    {
        return Ok(normalized);
    }
    // Interval phrase — a duration; kept raw (see module doc).
    if interval_seconds(trimmed).is_some() {
        return Ok(trimmed.to_string());
    }
    Err(Error::Agent(format!(
        "invalid cron schedule '{input}' — expected a 5/6-field expression \
         (e.g. \"0 9 * * *\") or an interval (e.g. \"every 6h\")"
    )))
}

/// Prepend a seconds field to a 5-field expression.
fn five_to_six(expr: &str) -> Result<String, Error> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() == 5 {
        Ok(format!("0 {}", fields.join(" ")))
    } else {
        Err(Error::Agent("not a 5-field expression".to_string()))
    }
}

/// The duration in seconds of an interval phrase (`every N<s|m|h|d|w>`,
/// case-insensitive, unit names or abbreviations). `None` when the input is
/// not an interval phrase or has a non-positive count.
///
/// Every accepted count is a valid duration — there are no calendar-shape
/// restrictions (the old mapping rejected "every 25h" and bent "every 30d"
/// into monthly-on-the-1st).
pub fn interval_seconds(input: &str) -> Option<i64> {
    let lower = input.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("every ")?.trim();
    let (num, unit) = split_num_unit(rest)?;
    if num == 0 {
        return None;
    }
    let secs = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => num as i64,
        "m" | "min" | "mins" | "minute" | "minutes" => num as i64 * 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => num as i64 * 3600,
        "d" | "day" | "days" => num as i64 * 86400,
        "w" | "wk" | "wks" | "week" | "weeks" => num as i64 * 604800,
        _ => return None,
    };
    Some(secs)
}

/// Compute the next fire time (RFC3339, UTC) for a schedule string.
/// `None` when the schedule cannot be parsed.
pub fn next_run_from_schedule(schedule: &str) -> Option<String> {
    next_run_from_schedule_at(schedule, chrono::Utc::now()).map(|t| t.to_rfc3339())
}

/// [`next_run_from_schedule`] with an injectable clock, so the boundary
/// behaviour (month ends — the bug the 2026-10-24 audit fixed) is testable.
fn next_run_from_schedule_at(
    schedule: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Some(secs) = interval_seconds(schedule) {
        return Some(now + chrono::Duration::seconds(secs));
    }
    let normalized = normalize_schedule(schedule).ok()?;
    let parsed = Schedule::from_str(&normalized).ok()?;
    parsed.after(&now).next()
}

/// Split "`<digits><unit>`" (e.g. "6h", "30d") into (6, "h").
fn split_num_unit(rest: &str) -> Option<(u32, &str)> {
    let idx = rest.find(|c: char| !c.is_ascii_digit())?;
    let num: u32 = rest[..idx].parse().ok()?;
    if num == 0 {
        return None;
    }
    Some((num, &rest[idx..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parses(s: &str) -> bool {
        Schedule::from_str(s).is_ok()
    }

    #[test]
    fn passes_through_valid_six_field() {
        let out = normalize_schedule("0 0 9 * * *").unwrap();
        assert_eq!(out, "0 0 9 * * *");
        assert!(parses(&out));
    }

    #[test]
    fn five_field_prepends_seconds() {
        let out = normalize_schedule("0 9 * * *").unwrap();
        assert_eq!(out, "0 0 9 * * *");
        assert!(parses(&out));
    }

    #[test]
    fn every_phrases_stay_raw_they_are_durations() {
        // The phrase is the canonical form: rewriting it to a cron
        // day-of-month step is what made "every 48h" fire Jan 31 then
        // Feb 1 (audit finding 4).
        for input in ["every 6h", "every 24h", "every 168h", "every 30d"] {
            let out = normalize_schedule(input).unwrap();
            assert_eq!(out, input, "interval phrases must not be rewritten");
        }
    }

    #[test]
    fn interval_seconds_is_the_true_duration() {
        assert_eq!(interval_seconds("every 15s"), Some(15));
        assert_eq!(interval_seconds("every 5m"), Some(300));
        assert_eq!(interval_seconds("every 6h"), Some(21_600));
        assert_eq!(interval_seconds("every 48h"), Some(172_800));
        assert_eq!(interval_seconds("every 1d"), Some(86_400));
        assert_eq!(interval_seconds("every 30d"), Some(2_592_000));
        assert_eq!(interval_seconds("every 2w"), Some(1_209_600));
        // Counts the old calendar mapping rejected are plain durations now.
        assert_eq!(interval_seconds("every 25h"), Some(90_000));
        // Not interval phrases at all.
        assert_eq!(interval_seconds("0 9 * * *"), None);
        assert_eq!(interval_seconds("banana"), None);
    }

    #[test]
    fn case_and_whitespace_insensitive() {
        let out = normalize_schedule("  Every 6H  ").unwrap();
        assert_eq!(out, "Every 6H");
        assert_eq!(interval_seconds(&out), Some(21_600));
    }

    #[test]
    fn rejects_garbage() {
        assert!(normalize_schedule("").is_err());
        assert!(normalize_schedule("banana").is_err());
        assert!(normalize_schedule("every 0h").is_err());
        assert!(normalize_schedule("every xh").is_err());
    }

    #[test]
    fn every_normalized_output_always_resolves() {
        for input in [
            "0 0 9 * * *",
            "0 9 * * *",
            "*/5 * * * *",
            "every 6h",
            "every 24h",
            "every 168h",
            "every 30d",
            "every 7d",
            "every 5m",
            "every 15s",
            "every 2w",
        ] {
            let out = normalize_schedule(input)
                .unwrap_or_else(|e| panic!("'{input}' must normalize: {e}"));
            assert!(
                parses(&out) || interval_seconds(&out).is_some(),
                "normalized '{input}' -> '{out}' must resolve as cron or duration"
            );
        }
    }

    /// The audit's concrete case: a fire at Jan 31 23:00 UTC under the old
    /// day-of-month expansion (`0 0 0 */2 * * *`) scheduled the next run for
    /// Feb 1 00:00 — ONE hour later, and the February cadence then bunched
    /// against the month end. A duration schedules Feb 2 23:00.
    #[test]
    fn every_48h_crossing_a_month_end_is_48_hours() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-31T23:00:00Z")
            .expect("fixed instant")
            .with_timezone(&chrono::Utc);
        let next = next_run_from_schedule_at("every 48h", now).expect("resolves");
        assert_eq!(next.to_rfc3339(), "2026-02-02T23:00:00+00:00");
    }

    /// The sibling case: "every 2w" under `day-of-month */14` fired Jan 29
    /// then Feb 1 — three days apart. A duration is fourteen days.
    #[test]
    fn every_2w_crossing_february_is_fourteen_days() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-29T08:00:00Z")
            .expect("fixed instant")
            .with_timezone(&chrono::Utc);
        let next = next_run_from_schedule_at("every 2w", now).expect("resolves");
        assert_eq!(next.to_rfc3339(), "2026-02-12T08:00:00+00:00");
    }

    /// Cron expressions keep their calendar semantics: 09:00 UTC daily
    /// means the next 09:00 UTC after `now`, whatever the month boundary.
    #[test]
    fn cron_forms_stay_calendar_rules() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-31T23:30:00Z")
            .expect("fixed instant")
            .with_timezone(&chrono::Utc);
        let next = next_run_from_schedule_at("0 9 * * *", now).expect("resolves");
        assert_eq!(next.to_rfc3339(), "2026-02-01T09:00:00+00:00");
    }
}
