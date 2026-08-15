# gog email management — Inbox Cleanup, Unsubscribe, and Label Automation

Patterns for bulk email management, unsubscribe extraction, and inbox organization.

## Extracting Unsubscribe Links

### Method 1: List-Unsubscribe Header (Most Reliable)
```python
import json, subprocess
r = subprocess.run([GOG, "gmail", "raw", msg_id, "-a", ACC], ...)
d = json.loads(r.stdout)
headers = {h["name"].lower(): h["value"] for h in d["payload"]["headers"]}
unsub = headers.get("list-unsubscribe", "")
urls = re.findall(r'<(https?://[^>]+)>', unsub)
```

### Method 2: Body HTML Links
```python
body_data = d["payload"]["body"]["data"]
body = base64.urlsafe_b64decode(body_data + "==").decode("utf-8", errors="replace")
links = re.findall(r'href="(https?://[^"]*)"', body)
unsub_links = [u for u in links if any(x in u.lower() 
    for x in ['unsubscribe', 'opt-out', 'remove', 'preference', 'manage'])]
```

## Hitting Unsubscribe URLs

```bash
curl -sL -o /dev/null -w "%{http_code}" --max-time 15 -A "Mozilla/5.0" "$URL"
```

## Inbox Organization

### Create Labels
```bash
gog gmail labels create "OTP-Verifications" -a ACC
gog gmail labels create "Delivery-Updates" -a ACC
gog gmail labels create "Stale-2025" -a ACC
gog gmail labels create "Promotions-Unsubscribed" -a ACC
```

### Move Emails (single line — no backslash continuation)
```bash
gog gmail batch modify "msgId1" "msgId2" -a ACC --add "Label" --remove INBOX
```

### Create Auto-Filters
```bash
gog gmail settings filters create -a ACC --from "sender.com" --add-label "Label" --archive
```

## Labels Reference

| ID | Name | Purpose |
|----|------|---------|
| Label_5 | OTP-Verifications | OTPs, email verifications |
| Label_6 | Delivery-Updates | Shipments, orders |
| Label_7 | Stale-2025 | Old events, expired |
| Label_8 | Promotions-Unsubscribed | Marketing (unsubscribed) |
