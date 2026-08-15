---
title: gog Calendar Timezone Patterns
version: 1.0.0
description: "Patterns for handling timezone correctly with gog calendar create/update commands"
---

# gog Calendar Timezone Patterns

## Critical Discovery: Timezone Handling in gog Calendar

**Problem**: `gog calendar create` and `gog calendar update` interpret `--from` and `--to` times in the **calendar's default timezone** (which for $GOG_ACCOUNT is Asia/Kolkata / IST), NOT in the timezone specified in the RFC3339 timestamp.

**Evidence**: 
- Created event with `--from "2026-07-07T17:30:00-05:00"` (5:30 PM CDT)
- Result: Event stored as 2026-07-08T04:00:00+05:30 (4:00 AM IST next day)
- The `-05:00` offset was **interpreted as IST**, not as CDT

## Correct Pattern: Use Target Timezone Time Expressed in Calendar's Timezone

To create an event at 5:30 PM CDT (Chicago time, UTC-5) on July 7, 2026:
1. Convert target time to calendar's timezone (IST = UTC+5:30)
2. 5:30 PM CDT = 5:30000 AM IST next day (July 8)
3. Use: `--from "2026-07-08T05:00:00+05:30" --to "2026-07-08T06:30:00+05:30"`

## Alternative: Create with --location and rely on Google Calendar's timezone handling

Google Calendar can handle timezone conversion if you set `--location` properly, but gog doesn't expose a `--timezone` flag for the event itself (only for output).

## Best Practice for This Account

**Calendar default timezone**: Asia/Kolkata (IST, UTC+5:30)

**Pattern for CDT events (Chicago time, UTC-5 standard / UTC-6 daylight)**:
- CDT (March-Nov): Target time + 10:30 = IST time next day
- CST (Nov-March): Target time + 11:30 = IST time next day

**Pattern for user-specified times**: Always convert to IST before passing to gog.

## Working Example from Session

```bash
# Goal: 5:30 PM CDT on July 7, 2026
# CDT = UTC-5, IST = UTC+5:30
# 17:30 CDT = 22:30 UTC = 04:00 IST next day (July 8)
gog calendar create $GOG_ACCOUNT \
  --summary "Michelle Lynn's Practitioner Spotlight" \
  --from "2026-07-08T04:00:00+05:30" \
  --to "2026-07-08T05:30:00+05:30" \
  --account $GOG_ACCOUNT \
  --description "Michelle Lynn's Practitioner Spotlight\n\nZoom: https://us06web.zoom.us/j/89323142845?pwd=zVL8HWBt9RH7Sgvcln5d2Ax6L3r19v.1" \
  --location "Online (Zoom)"
```

## Update Pattern

Same timezone conversion applies to `gog calendar update`:

```bash
gog calendar update $GOG_ACCOUNT <eventId> \
  --from "2026-07-08T04:00:00+05:30" \
  --to "2026-07-08T05:30:00+05:30" \
  --account $GOG_ACCOUNT
```

## Verification

After create/update, verify with:
```bash
gog calendar get $GOG_ACCOUNT <eventId> \
  --account $GOG_ACCOUNT
```

Check `start-local` and `end-local` fields show the intended local time in the event's timezone.

## Timezone Conversion Quick Reference

| Target TZ | Target Time | Calendar TZ (IST) | Event Time in IST |
|-----------|-------------|-------------------|-------------------|
| CDT (UTC-5) | 17:30 | UTC+5:30 | 04:00 next day |
| CST (UTC-6) | 17:30 | UTC+5:30 | 05:00 next day |
| EDT (UTC-4) | 17:30 | UTC+5:30 | 03:00 next day |
| EST (UTC-5) | 17:30 | UTC+5:30 | 04:00 next day |
| PDT (UTC-7) | 17:30 | UTC+5:30 | 06:00 next day |
| PST (UTC-8) | 17:30 | UTC+5:30 | 07:00 next day |

## Key Takeaway

**Never trust RFC3339 offset in --from/--to flags.** The gog CLI (or underlying Google Calendar API) interprets the timestamp in the calendar's default timezone, ignoring the offset. Always convert to calendar's timezone first.