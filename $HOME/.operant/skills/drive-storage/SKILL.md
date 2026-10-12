---
name: drive-storage
version: 1.0.0
category: productivity
description: Google Drive file operations: upload, download, search, organize, share. Use for storing files, retrieving shared files, managing folder structure, or sharing documents.
triggers:
  - When user asks about files in Drive
  - When uploading/downloading files to/from Drive
  - When searching Drive content
  - When creating folders or organizing Drive structure
  - When sharing files or managing permissions
metadata:
  operant:
    tags: [drive, files, upload, download, share]
---

# Drive Storage

Google Drive file operations via the `gog` CLI (binary at `gog`, v0.31.1).

**Account:** `$GOG_ACCOUNT`
**Auth diagnostics:** See `google-auth` skill if commands fail with exit code 4.

## Quick Start

```bash
# List root folder
gog drive ls -a $GOG_ACCOUNT --max 20

# Search by text (name + content)
gog drive search "report" -a $GOG_ACCOUNT --max 10

# Download by file ID
gog drive download 1AbCdEfGhIjKlMnOpQrStUvWxYz -a $GOG_ACCOUNT --out /tmp/file.pdf
```

## Default Flags

```bash
-a $GOG_ACCOUNT   # Explicit account
-j --results-only                    # JSON output
--no-input                           # Never prompt
```

**Subprocess env:** `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD`

## List Files

```bash
# Root folder
gog drive ls -a $GOG_ACCOUNT

# Specific folder by ID
gog drive ls --parent 1A2B3C4D5E6F7G8H9I0J -a $GOG_ACCOUNT

# All files (My Drive + shared + shared drives)
gog drive ls --all -a $GOG_ACCOUNT

# Filter by MIME type
gog drive ls --query "mimeType='application/pdf'" -a $GOG_ACCOUNT

# Recursive tree view
gog drive tree --parent 1A2B3C4D5E6F7G8H9I0J -a $GOG_ACCOUNT

# Disk usage for a folder
gog drive du --parent 1A2B3C4D5E6F7G8H9I0J -a $GOG_ACCOUNT

# Full inventory export to TSV
gog drive inventory --out ~/drive-inventory.tsv -a $GOG_ACCOUNT
```

## Search

```bash
# Text search (name + content)
gog drive search "invoice" -a $GOG_ACCOUNT --max 25

# Multi-word
gog drive search "Q3 2026 report" -a $GOG_ACCOUNT

# Drive query language (raw)
gog drive search "name contains 'budget'" --raw-query -a $GOG_ACCOUNT

# Search within a folder
gog drive search "report" --parent 1A2B3C4D5E6F7G8H9I0J -a $GOG_ACCOUNT

# Paginate
gog drive search "log" --max 100 --page NEXT_PAGE_TOKEN -a $GOG_ACCOUNT
```

Useful Drive query predicates: `name = 'X'`, `name contains 'X'`, `mimeType = '...'`, `modifiedTime > '2026-01-01T00:00:00Z'`, `trashed = true`, `starred = true`, `'folderId' in parents`, `fullText contains 'X'`.

## Upload

```bash
# Basic upload to root
gog drive upload ~/report.pdf -a $GOG_ACCOUNT

# Custom name
gog drive upload ~/report.pdf --name "Q3-2026-report.pdf" -a $GOG_ACCOUNT

# Into specific folder
gog drive upload ~/report.pdf --parent 1A2B3C4D5E6F7G8H9I0J -a $GOG_ACCOUNT

# Override MIME type
gog drive upload ~/data.bin --mime-type "application/json" -a $GOG_ACCOUNT

# Auto-convert to Google format (by extension)
gog drive upload ~/notes.md --convert -a $GOG_ACCOUNT

# Force specific Google format
gog drive upload ~/data.csv --convert-to sheets -a $GOG_ACCOUNT

# Markdown to Google Doc, keep YAML frontmatter
gog drive upload ~/notes.md --convert --keep-frontmatter -a $GOG_ACCOUNT

# Replace existing file (keeps ID, permissions, share link)
gog drive upload ~/report-v2.pdf --replace 1AbCdEfGhIjKlMnOpQrStUvWxYz -a $GOG_ACCOUNT
```

`--convert` auto-picks format by extension: `.md/.docx/.txt/.html` to Doc, `.csv/.xlsx` to Sheet, `.pptx` to Slides. `--convert-to <doc|sheets|slides>` forces format. Default (no flag) uploads as-is.

## Download

```bash
# Download to specific path
gog drive download 1AbCdEfGhIjKlMnOpQrStUvWxYz --out /tmp/report.pdf -a $GOG_ACCOUNT

# Export Google Doc as PDF
gog drive download 1AbCdEfGhIjKlMnOpQrStUvWxYz --format pdf --out /tmp/doc.pdf -a $GOG_ACCOUNT

# Export Google Sheet as CSV / XLSX
gog drive download 1AbCdEfGhIjKlMnOpQrStUvWxYz --format xlsx --out /tmp/sheet.xlsx -a $GOG_ACCOUNT
```

Supported `--format` values: `pdf`, `csv`, `xlsx`, `pptx`, `txt`, `png`, `docx`, `md`.

## Create Folder

```bash
# At root
gog drive mkdir "Q3-2026" -a $GOG_ACCOUNT

# Nested inside existing folder
gog drive mkdir "raw" --parent 1A2B3C4D5E6F7G8H9I0J -a $GOG_ACCOUNT
```

`mkdir` always creates a new folder. If a folder with the same name exists in the parent, you get a duplicate. Search first if you want to avoid duplicates.

## Move / Rename / Copy

```bash
# Move to different folder
gog drive move 1AbCdEfGhIjKlMnOpQrStUvWxYz --parent 1A2B3C4D5E6F7G8H9I0J -a $GOG_ACCOUNT

# Rename
gog drive rename 1AbCdEfGhIjKlMnOpQrStUvWxYz "final-report.pdf" -a $GOG_ACCOUNT

# Copy (must supply new name)
gog drive copy 1AbCdEfGhIjKlMnOpQrStUvWxYz "report-copy.pdf" -a $GOG_ACCOUNT
```

`move` only changes parent. To move AND rename, do two separate calls.

## Delete

```bash
# Move to trash (recoverable, default)
gog drive delete 1AbCdEfGhIjKlMnOpQrStUvWxYz -a $GOG_ACCOUNT

# Permanent delete (NOT recoverable)
gog drive delete 1AbCdEfGhIjKlMnOpQrStUvWxYz --permanent --force -a $GOG_ACCOUNT

# Dry-run first
gog drive delete 1AbCdEfGhIjKlMnOpQrStUvWxYz --dry-run --permanent -a $GOG_ACCOUNT
```

Delete moves to TRASH by default (recoverable from Drive Trash UI for ~30 days). `--permanent` bypasses trash entirely. No CLI restore-from-trash command exists; recovery is via Drive web UI only.

## Share / Permissions

```bash
# Share with specific user as reader (default)
gog drive share 1AbCdEfGhIjKlMnOpQrStUvWxYz --to user --email collaborator@example.com -a $GOG_ACCOUNT

# Writer (edit) access
gog drive share 1AbCdEfGhIjKlMnOpQrStUvWxYz --to user --email collaborator@example.com --role writer -a $GOG_ACCOUNT

# Commenter access with notification email
gog drive share 1AbCdEfGhIjKlMnOpQrStUvWxYz --to user --email reviewer@example.com --role commenter --notify -a $GOG_ACCOUNT

# Anyone with link can read
gog drive share 1AbCdEfGhIjKlMnOpQrStUvWxYz --to anyone --role reader -a $GOG_ACCOUNT

# Share with entire domain
gog drive share 1AbCdEfGhIjKlMnOpQrStUvWxYz --to domain --domain example.com --role reader -a $GOG_ACCOUNT

# List current permissions
gog drive permissions 1AbCdEfGhIjKlMnOpQrStUvWxYz -a $GOG_ACCOUNT

# Unshare (need permission ID, not email)
gog drive unshare 1AbCdEfGhIjKlMnOpQrStUvWxYz <permissionId> -a $GOG_ACCOUNT
```

Roles: `reader` (view), `commenter` (view + comment), `writer` (edit). `--to` values: `anyone`, `user`, `domain`. To find a permission ID for unshare, run `permissions` on the file and pick the `.id` whose `.emailAddress` matches.

## Get Metadata

```bash
# Full metadata
gog drive get 1AbCdEfGhIjKlMnOpQrStUvWxYz -a $GOG_ACCOUNT

# Select specific fields
gog drive get 1AbCdEfGhIjKlMnOpQrStUvWxYz --select id,name,mimeType,modifiedTime,size,parents -a $GOG_ACCOUNT
```

Useful fields: `id`, `name`, `mimeType`, `parents[]`, `modifiedTime`, `createdTime`, `size`, `webViewLink`, `webContentLink`, `owners[]`, `permissions[]`, `trashed`.

## Shortcuts

```bash
# Create shortcut to a file (in a specific folder)
gog drive shortcut create 1AbCdEfGhIjKlMnOpQrStUvWxYz --parent 1A2B3C4D5E6F7G8H9I0J -a $GOG_ACCOUNT

# With custom name
gog drive shortcut create 1AbCdEfGhIjKlMnOpQrStUvWxYz --parent 1A2B3C4D5E6F7G8H9I0J --name "Q3-report-shortcut" -a $GOG_ACCOUNT
```

Shortcuts are real Drive items with their own ID. Deleting a shortcut does NOT delete the target; deleting the target leaves a broken shortcut.

## Pitfalls

⚠️ **File IDs are long alphanumeric strings**, NOT file names. Get them from `search` or `ls` output (`.id` field in JSON).

⚠️ **`delete` moves to TRASH by default** (recoverable for ~30 days). Use `--permanent --force` for hard delete. No CLI restore; use Drive web UI.

⚠️ **`--parent` takes a folder ID**, not a folder name. Look up the ID with `ls` or `search` first.

⚠️ **Unshare requires permission ID**, not email. Run `permissions <fileId>` to find the `.id` whose `.emailAddress` matches.

⚠️ **`--convert` vs `--convert-to`.** `--convert` auto-picks Google format by extension. `--convert-to <doc|sheets|slides>` forces a specific format. `--keep-frontmatter` preserves YAML when converting Markdown to Doc.

⚠️ **Large file uploads may time out.** No resumable upload flag exists; just re-run if it fails.

⚠️ **Exit codes:** 0 = success, 3 = empty, 4 = auth error (see google-auth skill), 5 = not_found, 6 = denied.

⚠️ **Always use `-a $GOG_ACCOUNT` explicitly.**

⚠️ **Subprocess env:** Set `GOG_KEYRING_PASSWORD=$GOG_KEYRING_PASSWORD` in env dict.

⚠️ **Pagination:** Use `--page <nextPageToken>` to fetch next page. Drop `--results-only` to see the token in JSON output.
