# gog docs deep — Advanced Docs Operations

Write, format, insert, find-replace, tables, headers/footers, and structural operations.

## Exit Codes
| Code | Meaning |
|------|---------|
| 0 | Success |
| 3 | Empty results |
| 4 | Auth error |

## write — Write/replace/append content
```bash
gog docs write "<docId>" -a $GOG_ACCOUNT --markdown --file /tmp/content.md
gog docs write "<docId>" -a $GOG_ACCOUNT --append --file /tmp/append.md
gog docs write "<docId>" -a $GOG_ACCOUNT --replace --text "New content"
gog docs write "<docId>" -a $GOG_ACCOUNT --tab "Tab Name" --text "Content"
```
| Flag | Description |
|------|-------------|
| `--text=STRING` | Inline text content |
| `--file=STRING` | Read from file (`-` for stdin) |
| `--replace` | Replace entire doc content |
| `--markdown` | Interpret as markdown |
| `--append` | Append to existing content |
| `--tab=STRING` | Target specific tab |
| `--batch` | Batch with other operations |
| `--pageless` | Pageless layout |
| **Formatting** | `--font-family`, `--font-size`, `--text-color`, `--bg-color`, `--bold/--no-bold`, `--italic/--no-italic`, `--underline/--no-underline`, `--strikethrough/--no-strikethrough`, `--alignment`, `--line-spacing`, `--heading-level`, `--bullets`, `--indent` |

## insert — Insert text at position
```bash
gog docs insert "<docId>" "New paragraph" -a $GOG_ACCOUNT --index 100
gog docs insert "<docId>" "NEW TEXT" -a $GOG_ACCOUNT --at "old text"
gog docs insert "<docId>" -a $GOG_ACCOUNT --markdown --file /tmp/insert.md --at "## Section"
```
| Flag | Description |
|------|-------------|
| `--index=1` | Character index (default 1) |
| `--at=STRING` | Anchor by text match |
| `--occurrence=1` | Which occurrence of anchor |
| `--match-case` | Case-sensitive anchor match |
| `--markdown` | Interpret as markdown |
| `--tab=STRING` | Target specific tab |
| `--segment=STRING` | Target segment (header/footer) |
| `--batch` | Batch with other operations |

## insert-table — Insert native table
```bash
gog docs insert-table "<docId>" -a $GOG_ACCOUNT --rows 3 --cols 4
gog docs insert-table "<docId>" -a $GOG_ACCOUNT --rows 5 --cols 3 --index 100
```
| Flag | Description |
|------|-------------|
| `--rows=INT` | Number of rows (required) |
| `--cols=INT` | Number of columns (required) |
| `--index=INT` | Character insertion index |
| `--tab=STRING` | Target specific tab |

## cell-update — Update table cell content
```bash
gog docs cell-update "<docId>" -a $GOG_ACCOUNT --row 0 --col 1 --text "New value"
gog docs cell-update "<docId>" -a $GOG_ACCOUNT --row 2 --col 0 --file /tmp/cell.txt
```
| Flag | Description |
|------|-------------|
| `--row=INT` | Row index (required) |
| `--col=INT` | Column index (required) |
| `--text=STRING` | Cell text |
| `--file=STRING` | Cell text from file |
| `--markdown` | Interpret as markdown |

## cell-style — Style table cell
```bash
gog docs cell-style "<docId>" -a $GOG_ACCOUNT   --row 0 --col 0 --bold --text-color "#FF0000" --bg-color "#FFFF00"
```
| Flag | Description |
|------|-------------|
| `--row=INT` | Row index (required) |
| `--col=INT` | Column index (required) |
| **Formatting** | `--font-family`, `--font-size`, `--text-color`, `--bg-color`, `--bold/--no-bold`, `--italic/--no-italic` |

## table-row — Insert/delete table rows
```bash
gog docs table-row insert "<docId>" -a $GOG_ACCOUNT --row 2
gog docs table-row delete "<docId>" -a $GOG_ACCOUNT --row 3
```

## table-column — Insert/delete table columns
```bash
gog docs table-column insert "<docId>" -a $GOG_ACCOUNT --col 1
gog docs table-column delete "<docId>" -a $GOG_ACCOUNT --col 2
```

## find-replace — Find and replace text
```bash
gog docs find-replace "<docId>" "old" "new" -a $GOG_ACCOUNT
gog docs find-replace "<docId>" "placeholder" -a $GOG_ACCOUNT   --format markdown --content-file /tmp/replacement.md
gog docs find-replace "<docId>" "Word" "Replacement" -a $GOG_ACCOUNT   --match-case --first
```
| Flag | Description |
|------|-------------|
| `--content-file=STRING` | Replacement from file |
| `--match-case` | Case-sensitive |
| `--format=plain\|markdown` | Content format |
| `--first` | Replace first occurrence only |
| `--tab=STRING` | Target specific tab |

## sed — Regex find/replace (sed-style)
```bash
gog docs sed "<docId>" "s/old/new/g" -a $GOG_ACCOUNT
gog docs sed "<docId>" -e "s/foo/bar/g" -e "s/baz/qux/g" -a $GOG_ACCOUNT
gog docs sed "<docId>" -f /tmp/expressions.txt -a $GOG_ACCOUNT
```

## format — Apply formatting to existing text
```bash
gog docs format "<docId>" -a $GOG_ACCOUNT   --match "Important" --bold --text-color "#FF0000"
gog docs format "<docId>" -a $GOG_ACCOUNT   --match-all --match-case --font-size 14 --alignment CENTER
```
| Flag | Description |
|------|-------------|
| `--match=STRING` | Text to match |
| `--match-all` | Match all occurrences |
| `--match-case` | Case-sensitive |
| `--link=URL` | Add hyperlink |
| `--segment=STRING` | Target segment |
| **Formatting** | Same as `write` |

## structure — Show document structure
```bash
gog docs structure "<docId>" -a $GOG_ACCOUNT
gog docs structure "<docId>" -a $GOG_ACCOUNT --tab "Tab Name"
```

## header — Manage document headers
```bash
gog docs header list "<docId>" -a $GOG_ACCOUNT
gog docs header create "<docId>" -a $GOG_ACCOUNT --text "Header text"
gog docs header delete "<docId>" -a $GOG_ACCOUNT
```

## footer — Manage document footers
```bash
gog docs footer list "<docId>" -a $GOG_ACCOUNT
gog docs footer create "<docId>" -a $GOG_ACCOUNT --text "Footer text"
gog docs footer delete "<docId>" -a $GOG_ACCOUNT
```

## comments — Manage comments
```bash
gog docs comments list "<docId>" -a $GOG_ACCOUNT
gog docs comments create "<docId>" -a $GOG_ACCOUNT --text "Comment text"
```

## tabs — Manage tabs
```bash
gog docs tabs list "<docId>" -a $GOG_ACCOUNT
gog docs add-tab "<docId>" -a $GOG_ACCOUNT --title "New Tab"
gog docs rename-tab "<docId>" -a $GOG_ACCOUNT --title "Renamed"
gog docs delete-tab "<docId>" -a $GOG_ACCOUNT
gog docs list-tabs "<docId>" -a $GOG_ACCOUNT
```

## Agent Pattern
```bash
# Read-only
gog docs cat "<docId>" --readonly -a $GOG_ACCOUNT

# Dry-run mutation
gog docs write "<docId>" --replace --text "New" --dry-run -a $GOG_ACCOUNT

# Batch workflow
gog docs write "<docId>" --replace --text "Header" --batch -a $GOG_ACCOUNT
gog docs insert "<docId>" "Body" --at "Header" --batch -a $GOG_ACCOUNT
gog docs insert "<docId>" "Footer" --index 999 --batch -a $GOG_ACCOUNT
# Submit batch with: gog batch end <batchId>
```
