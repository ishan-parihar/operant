# gog slides deep — Advanced Slides Operations

Template creation, markdown import, thumbnails, text manipulation, tables, and slide management.

## Exit Codes
| Code | Meaning |
|------|---------|
| 0 | Success |
| 3 | Empty results |
| 4 | Auth error |

## create-from-markdown — Create from markdown
```bash
gog slides create-from-markdown "My Deck" -a $GOG_ACCOUNT   --content "# Slide 1\n\nBullets\n\n---\n\n# Slide 2"
gog slides create-from-markdown "My Deck" -a $GOG_ACCOUNT   --content-file /tmp/slides.md
gog slides create-from-markdown "Arch" -a $GOG_ACCOUNT   --content-file /tmp/slides.md --mmdc /usr/local/bin/mmdc
```
| Flag | Description |
|------|-------------|
| `--content=STRING` | Inline markdown |
| `--content-file=STRING` | Read from file |
| `--parent=STRING` | Destination folder ID |
| `--mmdc=STRING` | Mermaid CLI path |
| `--strict` | Strict parsing |
| `--no-notes` | Don't generate speaker notes |

## create-from-template — Create from template
```bash
gog slides create-from-template "<templateId>" "My Deck" -a $GOG_ACCOUNT   --replace "COMPANY,Acme" --replace "DATE,June 2026"
gog slides create-from-template "<templateId>" "My Deck" -a $GOG_ACCOUNT   --replacements /tmp/replacements.json
```
| Flag | Description |
|------|-------------|
| `--replace=REPLACE,...` | key=value replacements (repeatable) |
| `--replacements=STRING` | JSON file of replacements |
| `--exact` | Exact string matching |

## new-slide — Create native themed slide
```bash
gog slides new-slide "<presentationId>" -a $GOG_ACCOUNT
gog slides new-slide "<presentationId>" -a $GOG_ACCOUNT --layout "TITLE"
```

## duplicate-slide — Duplicate a slide
```bash
gog slides duplicate-slide "<presentationId>" "<slideId>" -a $GOG_ACCOUNT
```

## move-slide — Reorder slides
```bash
gog slides move-slide "<presentationId>" "<slideId>" -a $GOG_ACCOUNT --to-index 2
```
| Flag | Description |
|------|-------------|
| `--to-index=INT` | Zero-based insertion index |

## add-slide — Add slide with image
```bash
gog slides add-slide "<presentationId>" /tmp/image.png -a $GOG_ACCOUNT   --notes "Speaker notes"
gog slides add-slide "<presentationId>" /tmp/image.png -a $GOG_ACCOUNT   --before "<slideId>"
```
| Flag | Description |
|------|-------------|
| `--notes=STRING` | Speaker notes |
| `--notes-file=STRING` | Notes from file |
| `--before=STRING` | Insert before this slide ID |

## insert-image — Insert image at position/size
```bash
gog slides insert-image "<presentationId>" "<slideId>" /tmp/image.png   -a $GOG_ACCOUNT --width 400
gog slides insert-image "<presentationId>" "<slideId>" "https://example.com/img.png"   -a $GOG_ACCOUNT --width 300
```
| Flag | Description |
|------|-------------|
| `--width=FLOAT` | Image width in points |
| `--url=STRING` | Image URL (alternative to file arg) |

## thumbnail — Get/download slide thumbnail
```bash
gog slides thumbnail "<presentationId>" "<slideId>" -a $GOG_ACCOUNT --size large
gog slides thumbnail "<presentationId>" "<slideId>" -a $GOG_ACCOUNT   --out /tmp/slide.png --size large --format png
```
| Flag | Description |
|------|-------------|
| `--size=small\|medium\|large` | Thumbnail size |
| `--format=png\|jpeg` | Image format |
| `--out=STRING` | Output file path |
| `--overwrite` | Overwrite existing file |

## insert-text — Insert text into shape/table
```bash
gog slides insert-text "<presentationId>" "<objectId>" "New text"   -a $GOG_ACCOUNT --insertion-index 0
gog slides insert-text "<presentationId>" "<objectId>" "Replacement"   -a $GOG_ACCOUNT --replace
gog slides insert-text "<presentationId>" "<objectId>" "Cell value"   -a $GOG_ACCOUNT --row 0 --col 1
```
| Flag | Description |
|------|-------------|
| `--insertion-index=0` | Zero-based insertion index |
| `--replace` | Clear existing text first |
| `--row=INT` | Table row |
| `--col=INT` | Table column |

## replace-slide — Replace slide image
```bash
gog slides replace-slide "<presentationId>" "<slideId>" /tmp/new.png   -a $GOG_ACCOUNT --notes "Updated"
gog slides replace-slide "<presentationId>" "<slideId>" --url "https://example.com/img.png"   -a $GOG_ACCOUNT
```

## replace-text — Find-and-replace across presentation
```bash
gog slides replace-text "<presentationId>" "old" "new" -a $GOG_ACCOUNT
gog slides replace-text "<presentationId>" "Acme" "NewCorp" -a $GOG_ACCOUNT   --match-case --page "slide1,slide3"
```
| Flag | Description |
|------|-------------|
| `--match-case` | Case-sensitive |
| `--page=PAGE,...` | Restrict to specific slide IDs |
| `--object=STRING` | Specific shape objectId |
| `--all` | Replace across entire presentation |

## locate — Find text in shapes (returns object IDs + ranges)
```bash
gog slides locate "<presentationId>" "Target text" -a $GOG_ACCOUNT
# Returns: objectId, startIndex, endIndex (UTF-16)
```
Use this to get objectIds for `insert-text` operations.

## table — Create/update native tables
```bash
gog slides table create "<presentationId>" -a $GOG_ACCOUNT   --rows 3 --cols 4
gog slides table update "<presentationId>" "<tableId>" -a $GOG_ACCOUNT   --row 0 --col 1 --text "Cell value"
```

## read-slide — Read slide content
```bash
gog slides read-slide "<presentationId>" "<slideId>" -a $GOG_ACCOUNT
```

## list-slides — List all slides
```bash
gog slides list-slides "<presentationId>" -a $GOG_ACCOUNT -j
```

## delete-slide — Delete a slide
```bash
gog slides delete-slide "<presentationId>" "<slideId>" -a $GOG_ACCOUNT
```

## Agent Pattern
```bash
# Read-only
gog slides list-slides "<presentationId>" --readonly -a $GOG_ACCOUNT -j

# Find text location before modifying
gog slides locate "<presentationId>" "Old Text" -a $GOG_ACCOUNT -j
# Then use returned objectId for insert-text
```
