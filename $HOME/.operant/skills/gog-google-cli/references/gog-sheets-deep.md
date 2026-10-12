# gog sheets deep — Advanced Sheets Operations

Conditional formatting, charts, tables, named ranges, validation, batch updates.

## Exit Codes
| Code | Meaning |
|------|---------|
| 0 | Success |
| 3 | Empty results |
| 4 | Auth error |

## get — Read values with render options
```bash
gog sheets get "<sheetId>" "A1:D10" -a $GOG_ACCOUNT --render FORMATTED_VALUE
gog sheets get "<sheetId>" "A1:D10" -a $GOG_ACCOUNT --render FORMULA
gog sheets get "<sheetId>" "A1:Z100" -a $GOG_ACCOUNT --dimension COLUMNS
```
| Flag | Description |
|------|-------------|
| `--dimension=ROWS\|COLUMNS` | Row or column major |
| `--render=FORMATTED_VALUE\|UNFORMATTED_VALUE\|FORMULA` | Value render option |

## update — Write values
```bash
gog sheets update "<sheetId>" "A1:B2" -a $GOG_ACCOUNT   --input USER_ENTERED --values-json '[["Name","Score"],["Alice",95]]'
gog sheets update "<sheetId>" "A1:A10" -a $GOG_ACCOUNT   --values-json '[["v1"],["v2"]]' --copy-validation-from "B1:B10"
```
| Flag | Description |
|------|-------------|
| `--input=RAW\|USER_ENTERED` | How to parse input |
| `--values-json=STRING` | JSON array of values |
| `--copy-validation-from=STRING` | Copy data validation from range |
| `--fail-on-formula-error` | Fail if formula evaluates to error |

## batch-update — Submit raw Sheets API batch requests
```bash
gog sheets batch-update "<sheetId>" -a $GOG_ACCOUNT   --data-json '{"requests":[{"updateCells":{"start":{"sheetId":0,"rowIndex":0,"columnIndex":0},"rows":[{"values":[{"userEnteredValue":{"stringValue":"Hello"}}]}],"fields":"userEnteredValue"}}]}'
```
| Flag | Description |
|------|-------------|
| `--data-json=STRING` | Raw Sheets API BatchUpdateRequest JSON |

## append — Append values
```bash
gog sheets append "<sheetId>" "A:B" -a $GOG_ACCOUNT   --input USER_ENTERED --values-json '[["new","row"]]'
```
| Flag | Description |
|------|-------------|
| `--insert=OVERWRITE\|INSERT_ROWS` | Insert mode |

## delete-dimension — Delete rows or columns
```bash
gog sheets delete-dimension "<sheetId>" -a $GOG_ACCOUNT   --dimension ROWS --range-or-sheet "0:2"  # Delete rows 0-1
gog sheets delete-dimension "<sheetId>" -a $GOG_ACCOUNT   --dimension COLUMNS --range-or-sheet "Sheet1!A:B"
```
| Flag | Description |
|------|-------------|
| `--dimension=ROWS\|COLUMNS` | What to delete |
| `--range-or-sheet=STRING` | Range or sheet reference |

## format — Apply cell formatting
```bash
gog sheets format "<sheetId>" "A1:D1" -a $GOG_ACCOUNT   --format-json '{"backgroundColor":{"red":0.9,"green":0.9,"blue":0.9},"textFormat":{"bold":true}}'
```

## conditional-format — Manage conditional formatting
```bash
gog sheets conditional-format list "<sheetId>" -a $GOG_ACCOUNT --sheet 0
gog sheets conditional-format add "<sheetId>" -a $GOG_ACCOUNT   --type text-contains --expr "urgent"   --format-json '{"backgroundColor":{"red":1,"green":0.8,"blue":0.8}}'
gog sheets conditional-format add "<sheetId>" -a $GOG_ACCOUNT   --type number-gt --expr "100"   --format-json '{"textFormat":{"bold":true}}'
gog sheets conditional-format clear "<sheetId>" -a $GOG_ACCOUNT --sheet 0
```

## validation — Data validation rules
```bash
gog sheets validation list "<sheetId>" -a $GOG_ACCOUNT --sheet 0
gog sheets validation add "<sheetId>" -a $GOG_ACCOUNT   --range "A1:A100" --type list --values "Yes,No,Maybe"
gog sheets validation clear "<sheetId>" -a $GOG_ACCOUNT --sheet 0
```

## copy-paste — Copy/paste range operations
```bash
gog sheets copy-paste "<sheetId>" -a $GOG_ACCOUNT   --source "A1:D10" --dest "A20"
```

## chart — Manage charts
```bash
gog sheets chart list "<sheetId>" -a $GOG_ACCOUNT
gog sheets chart get "<sheetId>" "<chartId>" -a $GOG_ACCOUNT
gog sheets chart create "<sheetId>" -a $GOG_ACCOUNT   --spec-json '<chart_spec>' --sheet 0 --anchor "A1" --width 600 --height 400
gog sheets chart delete "<sheetId>" "<chartId>" -a $GOG_ACCOUNT
```

## table — Manage Sheets tables
```bash
gog sheets table list "<sheetId>" -a $GOG_ACCOUNT
gog sheets table create "<sheetId>" -a $GOG_ACCOUNT   --name "Sales" --columns-json '[{"column":"Name"},{"column":"Amount"}]'
gog sheets table append "<sheetId>" "<tableId>" -a $GOG_ACCOUNT   --values-json '[["Item",100]]'
gog sheets table clear "<sheetId>" "<tableId>" -a $GOG_ACCOUNT
gog sheets table delete "<sheetId>" "<tableId>" -a $GOG_ACCOUNT --discard-data
```

## named-ranges — Manage named ranges
```bash
gog sheets named-ranges list "<sheetId>" -a $GOG_ACCOUNT
gog sheets named-ranges add "<sheetId>" -a $GOG_ACCOUNT   --name "Scores" --range "Sheet1!A1:B10"
gog sheets named-ranges delete "<sheetId>" "<namedRangeId>" -a $GOG_ACCOUNT
```

## Other operations
- `gog sheets freeze <sheetId>` — Freeze rows/columns
- `gog sheets resize-columns <sheetId> <columns>` — Resize columns
- `gog sheets resize-rows <sheetId> <rows>` — Resize rows
- `gog sheets merge <sheetId> <range>` — Merge cells
- `gog sheets unmerge <sheetId> <range>` — Unmerge cells
- `gog sheets notes <sheetId> <range>` — Read cell notes
- `gog sheets update-note <sheetId> <range>` — Set/clear cell note
- `gog sheets links <sheetId> <range>` — Read hyperlinks
- `gog sheets find-replace <sheetId> <find> <replace>` — Find and replace
- `gog sheets banding <sheetId>` — Manage alternating colors
- `gog sheets number-format <sheetId> <range>` — Apply number format
- `gog sheets read-format <sheetId> <range>` — Read cell formatting

## Agent Pattern
```bash
# Read-only
gog sheets get "<sheetId>" "A1:Z100" --readonly -a $GOG_ACCOUNT -j --results-only

# Dry-run mutation
gog sheets update "<sheetId>" "A1:B1" --values-json '[["a","b"]]' --dry-run -a $GOG_ACCOUNT
