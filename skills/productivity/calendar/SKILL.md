---
name: calendar
version: 1.0.0
category: productivity
description: Google Calendar operations: list/create/update/delete events, check schedules, manage calendars, RSVP, propose times. Use for any scheduling task, calendar query, or event creation.
triggers:
  - When user asks about schedule/calendar/events/meetings
  - When creating/updating/deleting calendar events
  - When checking availability or free/busy
  - When listing meetings today/tomorrow/this week
  - When responding to event invitations
  - When proposing alternative meeting times
metadata:
  operant:
    tags: [calendar, events, scheduling, meetings, rsvp]
---

# Calendar

Google Calendar operations via the `gog` CLI (binary at `gog`, v0.31.1).

**Account:** `$GOG_ACCOUNT` (calendar default timezone: Asia/Kolkata / IST, UTC+5:30)
**Auth diagnostics:** See `google-auth` skill if commands fail with exit code 4.

## ⚠️ CRITICAL: Timezone Handling

**gog calendar interprets `--from` and `--to` in the calendar's default timezone (IST), NOT in the RFC3339 offset.**

The offset you provide (e.g., `-05:00`) is IGNORED. Always convert target time to IST before passing to gog.

**Conversion table:**
| Target TZ | Target Time | Calendar TZ (IST) | Event Time in IST |
|-----------|-------------|-------------------|-------------------|
| CDT (UTC-5) | 17:30 | UTC+5:30 | 04:00 next day |
| CST (UTC-6) | 17:30 | UTC+5:30 | 05:00 next day |
| EDT (UTC-4) | 17:30 | UTC+5:30 | 03:00 next day |
| EST (UTC-5) | 17:30 | UTC+5:30 | 04:00 next day |
| PDT (UTC-7) | 17:30 | UTC+5:30 | 06:00 next day |
| PST (UTC-8) | 17:30 | UTC+5:30 | 07:00 next day |

**Working example:** to create an event at 5:30 PM CDT (Chicago) on July 7, 2026:
```bash
# 17:30 CDT = 22:30 UTC = 04:00 IST next day (July 8)
gog calendar create primary   --summary "Event Name"   --from "2026-07-08T04:00:00+05:30"   --to "2026-07-08T05:30:00+05:30"   -a $GOG_ACCOUNT   --description "Event description"   --location "Location"
```

**After create/update, ALWAYS verify** with `gog calendar get primary <eventId>` and check `start-local` and `end-local` fields show the intended local time.

## Quick Start

```bash
# Health check
gog auth doctor --check -j --results-only

# List today's events
gog calendar events primary --today -a $GOG_ACCOUNT

# List next 7 days
gog calendar events primary --days 7 -a $GOG_ACCOUNT --max 20

# Get specific event
gog calendar event primary "<eventId>" -a $GOG_ACCOUNT
```

## Default Flags

```bash
-a $GOG_ACCOUNT   # Explicit account
-j --results-only                    # JSON output
--no-input                           # Never prompt
```

**Subprocess env:** `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD`

## List Events

```bash
# Today
gog calendar events primary --today -a $GOG_ACCOUNT

# Tomorrow
gog calendar events primary --tomorrow -a $GOG_ACCOUNT

# Next N days
gog calendar events primary --days 7 -a $GOG_ACCOUNT --max 50

# Specific date range
gog calendar events primary --from "2026-07-01T00:00:00+05:30" --to "2026-07-31T23:59:59+05:30" -a $GOG_ACCOUNT

# Free text search
gog calendar events primary --query "team meeting" --days 30 -a $GOG_ACCOUNT

# Multiple calendars (comma-separated)
gog calendar events --calendars "primary,work@group.calendar.google.com" --days 7 -a $GOG_ACCOUNT
```

## Create Events

### Basic (summary, from, to)

⚠️ Times MUST be in IST (UTC+5:30). See CRITICAL timezone section above.

```bash
gog calendar create primary   --summary "Meeting Title"   --from "2026-08-15T14:00:00+05:30"   --to "2026-08-15T15:00:00+05:30"   -a $GOG_ACCOUNT   --force
```

### With description, location, attendees

```bash
gog calendar create primary   --summary "Project Review"   --from "2026-08-15T14:00:00+05:30"   --to "2026-08-15T15:00:00+05:30"   --description "Quarterly review of project milestones"   --location "Conference Room A"   --attendees "alice@example.com, bob@example.com"   -a $GOG_ACCOUNT   --force
```

### With location search (Google Places)

```bash
gog calendar create primary   --summary "Client Meeting"   --from "2026-08-15T14:00:00+05:30"   --to "2026-08-15T15:00:00+05:30"   --location-search "Empire State Building, New York"   -a $GOG_ACCOUNT   --force
```

### With reminders

Format: `method:duration` (e.g., `email:1h`, `popup:10m`, `email:1d`)

```bash
gog calendar create primary   --summary "Important Meeting"   --from "2026-08-15T14:00:00+05:30"   --to "2026-08-15T15:00:00+05:30"   --reminder "email:1d,popup:10m,popup:1h"   -a $GOG_ACCOUNT   --force
```

### All-day event

```bash
gog calendar create primary   --summary "Vacation Day"   --from "2026-08-15"   --to "2026-08-16"   --all-day   -a $GOG_ACCOUNT   --force
```

### With Google Meet

```bash
gog calendar create primary   --summary "Remote Meeting"   --from "2026-08-15T14:00:00+05:30"   --to "2026-08-15T15:00:00+05:30"   --with-meet   -a $GOG_ACCOUNT   --force
```

### With guest permissions

```bash
gog calendar create primary   --summary "Open Meeting"   --from "2026-08-15T14:00:00+05:30"   --to "2026-08-15T15:00:00+05:30"   --guests-can-invite --guests-can-modify --guests-can-see-others   -a $GOG_ACCOUNT   --force
```

### Recurring events (⚠️ --rrule is BROKEN)

⚠️ **`gog calendar create --rrule` is BROKEN in v0.31.1.** Use individual events instead, or create via Google Calendar UI.

## Update Events

```bash
# Update time only (remember: IST timezone!)
gog calendar update primary "<eventId>"   --from "2026-08-15T15:00:00+05:30"   --to "2026-08-15T16:00:00+05:30"   -a $GOG_ACCOUNT   --force

# Update summary
gog calendar update primary "<eventId>"   --summary "New Meeting Title"   -a $GOG_ACCOUNT   --force

# Move to different calendar
gog calendar move primary "<eventId>" "work@group.calendar.google.com" -a $GOG_ACCOUNT
```

⚠️ Same timezone bug applies to `update`: `--from` and `--to` are interpreted in IST, not in the offset you provide.

## Delete Events

```bash
gog calendar delete primary "<eventId>" -a $GOG_ACCOUNT --force
```

## Get Event Details

```bash
# Standard
gog calendar event primary "<eventId>" -a $GOG_ACCOUNT

# Raw API dump (for debugging)
gog calendar raw primary "<eventId>" -a $GOG_ACCOUNT
```

## RSVP / Respond to Invitations

```bash
gog calendar respond primary "<eventId>"   --response accepted   -a $GOG_ACCOUNT   --force
```

Response options: `accepted`, `declined`, `tentative`, `needsAction`

## Availability (Free/Busy)

```bash
# Query free/busy across calendars
gog calendar freebusy "primary,work@group.calendar.google.com"   --from "2026-08-15T00:00:00+05:30"   --to "2026-08-15T23:59:59+05:30"   -a $GOG_ACCOUNT
```

## Propose New Time

```bash
gog calendar propose-time primary "<eventId>"   --proposed-times "2026-08-16T14:00:00+05:30,2026-08-17T14:00:00+05:30"   -a $GOG_ACCOUNT   --force
```

## Multiple Calendars

```bash
# List all calendars
gog calendar calendars -a $GOG_ACCOUNT -j --results-only

# Subscribe to public calendar
gog calendar subscribe "en.indian#holiday@group.v.calendar.google.com" -a $GOG_ACCOUNT

# Create new calendar
gog calendar create-calendar "Personal" -a $GOG_ACCOUNT

# Get calendar colors (for visual reference)
gog calendar colors -a $GOG_ACCOUNT -j --results-only
```

## Detect Changes (incremental sync)

```bash
# List events modified since timestamp
gog calendar changed primary --since 24h -a $GOG_ACCOUNT

# Since specific RFC3339 timestamp
gog calendar changed primary --since "2026-07-01T00:00:00+05:30" -a $GOG_ACCOUNT
```

## Conflicts

```bash
gog calendar conflicts -a $GOG_ACCOUNT
```

## Permissions / ACL

```bash
# List ACL on calendar
gog calendar acl primary -a $GOG_ACCOUNT
```

## Pitfalls

⚠️ **NEVER trust RFC3339 offset in --from/--to flags.** The gog CLI (or underlying API) interprets the timestamp in the calendar's default timezone (IST), ignoring the offset. Always convert to IST first. See CRITICAL section at top.

⚠️ **`--rrule` is BROKEN** (confirmed through v0.31.1). Use individual events for recurring schedules, or create via UI.

⚠️ **`--from` and `--to` are required for create** (unless `--all-day` with date-only format).

⚠️ **`--reminder` format is `method:duration`.** Methods: `email`, `popup`. Durations: `10m`, `1h`, `1d`. Repeat flag with commas.

⚠️ **`--attendees` comma-separated emails.** Modifiers: `*required`, `*optional` (e.g., `--attendees "a@x.com *required, b@y.com *optional"`).

⚠️ **`--location-search`** resolves Google Places text search; `--location` is plain text.

⚠️ **`--with-meet` creates Google Meet link.** Use for remote meetings.

⚠️ **Always verify event creation** with `gog calendar event primary <eventId>` and check `start-local` / `end-local` fields.

⚠️ **Exit codes:**
- 0 = success
- 3 = empty (no results)
- 4 = auth error (see google-auth skill)
- 7 = rate limited

⚠️ **Always use `-a $GOG_ACCOUNT` explicitly.**

⚠️ **Subprocess env:** Set `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD` in env dict.

## Quick Reference

```bash
# List today's events
gog calendar events primary --today -a $GOG_ACCOUNT

# Create event (remember IST!)
gog calendar create primary --summary "Title" --from "2026-08-15T14:00:00+05:30" --to "2026-08-15T15:00:00+05:30" -a $GOG_ACCOUNT --force

# Verify
gog calendar event primary "<eventId>" -a $GOG_ACCOUNT

# Delete
gog calendar delete primary "<eventId>" -a $GOG_ACCOUNT --force
```
