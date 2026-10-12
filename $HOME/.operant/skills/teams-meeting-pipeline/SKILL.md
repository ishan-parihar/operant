---
name: teams-meeting-pipeline
description: Teams meeting summaries, job replay, Graph subscriptions.
version: 1.1.0
author: Operant (adapted from hermes-agent + Teknium)
license: MIT
prerequisites:
  env_vars: [MSGRAPH_TENANT_ID, MSGRAPH_CLIENT_ID, MSGRAPH_CLIENT_SECRET]
  commands: [curl, jq]
metadata:
  operant:
    tags: [Teams, Microsoft Graph, Meetings, Productivity, Operations]
---

# Teams Meeting Pipeline

Use this skill whenever the user asks about Microsoft Teams meeting summaries, transcripts, recordings, action items, Graph subscriptions, or any operational question about the Teams meeting pipeline. Works in any language — the triggers below are examples, not an exhaustive list.

Everything operator-facing is the Microsoft Graph REST API driven via the `terminal` tool with `curl` (auth via `MSGRAPH_*` env vars -> OAuth2 client-credentials token). There are no new model tools for this pipeline — the REST API is the surface.

## When to use this skill

The user is asking to:
- summarize a Teams meeting / extract action items / pull meeting notes
- check pipeline status, inspect a stored meeting job, or see recent meetings
- replay / re-run a stored job that failed or needs a fresh summary
- validate Microsoft Graph setup after changing env or config
- troubleshoot "meeting summary never arrived" or "no new meetings are ingesting"
- manage Graph webhook subscriptions (create, renew, delete, inspect)
- set up automated subscription renewal (see pitfall below)

Multilingual trigger examples (not exhaustive):
- English: "summarize the Teams meeting", "pipeline status", "replay job X"
- Turkish: "Teams meeting özetle", "action item çıkar", "toplantı notu", "pipeline durumu", "replay job"

## Prerequisites

Before using the pipeline, verify these are set in `${HERMES_HOME:-~/.operant}/.env`:

```bash
MSGRAPH_TENANT_ID=...
MSGRAPH_CLIENT_ID=...
MSGRAPH_CLIENT_SECRET=...
```

If any are missing, direct the user to the Azure app registration guide at `/docs/guides/microsoft-graph-app-registration` — they need an Azure AD app registration with admin-consented Graph application permissions before the pipeline will work.

## Command reference

### Status and inspection (start here)

```bash
# Acquire a client-credentials token, then drive Graph REST:
TOKEN=$(curl -s -X POST https://login.microsoftonline.com/$MSGRAPH_TENANT_ID/oauth2/v2.0/token \
  -d "client_id=$MSGRAPH_CLIENT_ID" -d "client_secret=$MSGRAPH_CLIENT_SECRET" \
  -d "grant_type=client_credentials" -d "scope=https://graph.microsoft.com/.default" | jq -r .access_token)

# List recent meeting transcripts:
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://graph.microsoft.com/v1.0/users/me/onlineMeetings/getAllTranscripts"

# List current webhook subscriptions:
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://graph.microsoft.com/v1.0/subscriptions"
```

### Re-running / debugging

```bash
# Re-fetch a meeting transcript by ID:
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://graph.microsoft.com/v1.0/users/me/onlineMeetings/<meeting-id>/transcripts"

# By join URL: resolve via /onlineMeetings?joinWebUrl=..., then fetch transcripts.
```

### Subscription management

```bash
# Create a webhook subscription (expires in <= 72h):
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  "https://graph.microsoft.com/v1.0/subscriptions" -d \
  '{"changeType":"created","notificationUrl":"https://<your-public-host>/msgraph/webhook","resource":"communications/onlineMeetings/getAllTranscripts","clientState":"$MSGRAPH_WEBHOOK_CLIENT_STATE","expirationDateTime":"<iso-8601 +72h>"}'

# Renew (PATCH) or delete:
curl -s -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  "https://graph.microsoft.com/v1.0/subscriptions/<sub-id>" -d '{"expirationDateTime":"<iso-8601>"}'
curl -s -X DELETE -H "Authorization: Bearer $TOKEN" \
  "https://graph.microsoft.com/v1.0/subscriptions/<sub-id>"
```

## Decision tree for common asks

- User asks "why didn't I get a summary for today's meeting?" → start with `list --status failed`, then `show <job-id>` on the relevant row. If the job doesn't exist at all, check `subscriptions` — the webhook may have expired (see pitfall below).
- User asks "is setup working?" → `validate`, then `token-health`, then `subscriptions`. If all three pass, request a test meeting and check `list` for a fresh row.
- User asks "re-run summary for meeting X" → `list` to find the job ID, `run <job-id>` to replay. If it fails again, `show <job-id>` to inspect the error and `fetch --meeting-id` to dry-run the artifact resolution.
- User asks "add meeting X to the pipeline" → usually you don't — the pipeline is subscription-driven, not per-meeting. If they want a specific past meeting summarized, use `fetch` to pull transcript + `run` after a job is created.

## Critical pitfall: Graph subscriptions expire in 72 hours

Microsoft Graph caps webhook subscriptions at 72 hours and **will not auto-renew them**. If an automatic renewal job is not scheduled, meeting notifications silently stop arriving 3 days after any manual subscription creation.

When the user reports "the pipeline worked yesterday but nothing is arriving today":
1. Run the subscriptions listing above — if it's empty or all entries show `expirationDateTime` in the past, that's the cause.
2. Recreate with `subscribe` as shown above.
3. **Set up automated renewal immediately** via `operant cron create`, a systemd timer, or plain crontab. The operator runbook at `/docs/guides/operate-teams-meeting-pipeline#automating-subscription-renewal-required-for-production` has all three options. 12-hour interval is safe (6x headroom against the 72h limit).

## Other pitfalls

- **Transcript not available yet.** Teams takes some time after a meeting ends to generate the transcript artifact. `fetch --meeting-id` on a just-ended meeting may return empty. Wait 2-5 minutes and retry, or let the Graph webhook drive ingestion naturally.
- **Delivery mode mismatch.** If summaries are produced (`list` shows success) but nothing lands in Teams, check `platforms.teams.extra.delivery_mode` and the matching target config (`incoming_webhook_url` OR `chat_id` OR `team_id`+`channel_id`). The writer reads these from config.yaml or `TEAMS_*` env vars.
- **Graph app permissions.** A token acquires cleanly (`token-health` passes) but Graph API calls return 401/403 when permissions were added but admin consent wasn't re-granted. Have the user revisit the app registration in the Azure portal and click "Grant admin consent" again.

## Related docs

Point the user to these when they need more depth than this skill covers:
- Azure app registration walkthrough: `/docs/guides/microsoft-graph-app-registration`
- Full pipeline setup: `/docs/user-guide/messaging/teams-meetings`
- Operator runbook (renewal automation, troubleshooting, go-live checklist): `/docs/guides/operate-teams-meeting-pipeline`
- Webhook listener setup: `/docs/user-guide/messaging/msgraph-webhook`
