---
ID: "AD-030"
Title: "The Telegram token filter is covered for URL-embedded tokens; the historic log is not rewritten"
Status: "Active"
Date: "2026-10-19"
Description: "Telegram bot tokens in bot URLs are redacted by a tested rule; the old gateway.log lines are not rewritten, and token rotation stays the owner's call"
---

# AD-030: The Telegram token filter is covered for URL-embedded tokens; the historic log is not rewritten

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

BUGS R42-8: two live Telegram bot tokens appeared unredacted in `~/.operant/logs/gateway.log` (46 MB), in URL-embedded form. Some lines already showed the redaction placeholder. The owner declined rotation.

## Decision

1. The filter covers the URL form. `redacts_telegram_token_in_bot_url` (operant-core `redaction.rs`) passes in iteration 973, and the leading `([^0-9])` guard handles `bot<TOKEN>` in a URL.
2. The historic log lines are not rewritten. Editing a log the owner keeps is a change to their data beyond this request.
3. Rotation is not done. The owner declined it, and it is a human step.

## Consequences

- Tokens logged from now on in a bot URL are redacted, as the test shows. Tokens already in the log stay there.
- The two live tokens remain valid until the owner rotates them. The owner should decide that.

## Orthogonality

AD-021 (stored knowledge is redacted) uses the same redaction module.

## Revisions

- 2026-10-19: created

