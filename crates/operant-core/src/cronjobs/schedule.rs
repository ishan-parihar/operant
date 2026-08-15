//! Cron schedule normalization.
//!
//! The scheduler parses schedules with `cron::Schedule::from_str`, which
//! requires a 6-field expression (seconds minutes hours day-of-month month
//! day-of-week). User-facing surfaces historically documented friendlier
//! formats ("0 9 * * *", "every 6h") that the crate silently rejects — jobs
//! created with those strings never computed a `next_run_at` and never
//! fired. This module normalizes all accepted forms down to a parseable
//! 6-field expression:
//!
//!   * 6-field cron expression   → passed through unchanged
//!   * 5-field cron expression   → seconds `0` prepended ("0 9 * * *" → "0 0 9 * * *")
//!   * "every N<s|m|h|d|w>"      → interval expansion (see [`every_interval`])
//!
//! Callers: `cmd_cron create` and `suggestions accept` validate/normalize
//! before persisting; the scheduler re-normalizes at compute time so
//! previously-created jobs self-heal on their next tick.

use crate::error::Error;
use cron::Schedule;
use std::str::FromStr;

/// Normalize a user-supplied schedule string to a 6-field cron expression
/// the `cron` crate can parse. Errors when nothing usable can be produced.
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
    // Natural language "every <n><unit>".
    if let Some(normalized) = every_interval(trimmed)
        && Schedule::from_str(&normalized).is_ok()
    {
        return Ok(normalized);
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

/// Expand "every N<unit>" into a 6-field expression. `None` when the
/// phrasing is not understood or the interval is out of range.
fn every_interval(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let rest = lower.strip_prefix("every ")?.trim();
    let (num, unit) = split_num_unit(rest)?;
    match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(format!("*/{} * * * * *", num)),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(format!("0 */{} * * * *", num)),
        "h" | "hr" | "hrs" | "hour" | "hours" => {
            if num == 24 {
                Some("0 0 0 * * *".to_string()) // daily at midnight
            } else if num < 24 {
                Some(format!("0 0 */{} * * *", num))
            } else if num % 24 == 0 {
                // e.g. every 48h → every 2 days
                Some(format!("0 0 0 */{} * * *", num / 24))
            } else {
                None
            }
        }
        "d" | "day" | "days" => match num {
            1 => Some("0 0 0 * * *".to_string()),  // daily at midnight
            7 => Some("0 0 0 * * 7".to_string()),  // weekly, Sunday (DOW 7)
            30 => Some("0 0 0 1 * *".to_string()), // monthly, 1st
            n if n < 31 => Some(format!("0 0 0 */{} * *", n)),
            _ => None,
        },
        "w" | "wk" | "wks" | "week" | "weeks" => match num {
            1 => Some("0 0 0 * * 7".to_string()), // weekly, Sunday (DOW 7)
            n if n < 5 => Some(format!("0 0 0 */{} * *", n * 7)),
            _ => None,
        },
        _ => None,
    }
}

/// Compute the next fire time (RFC3339) for a schedule string, normalizing
/// it first. `None` when the schedule cannot be normalized or parsed.
pub fn next_run_from_schedule(schedule: &str) -> Option<String> {
    let normalized = normalize_schedule(schedule).ok()?;
    let parsed = Schedule::from_str(&normalized).ok()?;
    parsed.upcoming(chrono::Utc).next().map(|t| t.to_rfc3339())
}

/// Split "<digits><unit>" (e.g. "6h", "30d") into (6, "h").
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
    fn every_hours_interval() {
        let out = normalize_schedule("every 6h").unwrap();
        assert_eq!(out, "0 0 */6 * * *");
        assert!(parses(&out));

        let daily = normalize_schedule("every 24h").unwrap();
        assert_eq!(daily, "0 0 0 * * *");
        assert!(parses(&daily));

        let weekly = normalize_schedule("every 168h").unwrap();
        assert_eq!(weekly, "0 0 0 */7 * * *");
        assert!(parses(&weekly));
    }

    #[test]
    fn every_days_interval() {
        let daily = normalize_schedule("every 1d").unwrap();
        assert_eq!(daily, "0 0 0 * * *");

        let weekly = normalize_schedule("every 7d").unwrap();
        assert_eq!(weekly, "0 0 0 * * 7");

        let monthly = normalize_schedule("every 30d").unwrap();
        assert_eq!(monthly, "0 0 0 1 * *");
    }

    #[test]
    fn every_minutes_and_seconds() {
        let m = normalize_schedule("every 5m").unwrap();
        assert_eq!(m, "0 */5 * * * *");

        let s = normalize_schedule("every 15s").unwrap();
        assert_eq!(s, "*/15 * * * * *");
    }

    #[test]
    fn case_and_whitespace_insensitive() {
        let out = normalize_schedule("  Every 6H  ").unwrap();
        assert_eq!(out, "0 0 */6 * * *");
    }

    #[test]
    fn rejects_garbage() {
        assert!(normalize_schedule("").is_err());
        assert!(normalize_schedule("banana").is_err());
        assert!(normalize_schedule("every 0h").is_err());
        assert!(normalize_schedule("every xh").is_err());
    }

    #[test]
    fn every_normalized_output_always_parses() {
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
            let out = normalize_schedule(input).unwrap();
            assert!(parses(&out), "normalized '{input}' -> '{out}' must parse");
        }
    }
}
