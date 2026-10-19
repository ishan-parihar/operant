---
ID: "AD-039"
Title: "PDF text extraction moves to lopdf 0.45, and the decompression bomb is bounded"
Status: "Active"
Date: "2026-10-19"
Description: "pdf-extract 0.12 pins lopdf 0.42, which reaches ttf-parser (RUSTSEC-2026-0192); pdf_read is ported to lopdf 0.45, which has no ttf-parser and bounds each page's decompressed content"
---

# AD-039: PDF text extraction moves to lopdf 0.45, and the decompression bomb is bounded

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

RT-27 was filed as: "`ttf-parser` is reached through `pdf-extract` 0.12.1 →
`lopdf` 0.42.0. Only the `rag-pdf` feature reaches it. The fix is a port of
`pdf_read` to `lopdf` 0.45, which is a text-extraction rewrite, or an
upstream `pdf-extract` release."

The upstream route is closed: `pdf-extract` 0.12.1 is the latest release
and it pins `lopdf` 0.42. So the port was the only path, and it was
tractable: `pdf_read` has exactly ONE call site
(`pdf_extract::extract_text_from_mem(&bytes)`), and `lopdf` 0.45 has an
equivalent API.

`lopdf` 0.45 with `default-features = false` pulls no `ttf-parser` at all.
Its `font_embedding` feature is the alternative it offers (it uses
`skrifa`), and this tool does not embed fonts, so that feature is not
enabled either.

Two things came with the swap.

**The decompression bomb is now bounded.** A PDF page's content stream is
a compressed blob whose decompressed size the file chooses.
`pdf-extract` did not bound it, so a small PDF could expand into gigabytes
inside a `spawn_blocking` task. `lopdf` does bound it, through
`extract_text_with_limit(pages, max_decompressed_size)`. The bound is
explicit: 64 MiB per page.

**The test fixture was broken.** The hand-built PDF in the test module
declared a 41-byte stream as `/Length 44`, and its xref entries pointed
past their objects (9, 58, 115, 274, 370 for objects at 9, 52, 101, 211,
298). `pdf-extract` reconstructed the cross-reference table and was lenient
about the mismatch, so the fixture passed. `lopdf` TRUSTS the table, so the
same bytes now yield no pages and no text.

That second point matters beyond the fixture. The tool-level test
`extracts_text_from_valid_pdf` accepted "no extractable text" as a valid
outcome, because a hand-crafted PDF may not parse perfectly. That is
reasonable for a malformed fixture — but it means the test would not have
noticed an extractor that extracted NOTHING from a perfectly good PDF.

## Decision

**Port `pdf_read` to `lopdf` 0.45.** `default-features = false`: neither
`chrono-clock` nor `rayon` is needed for text extraction, and
`font_embedding` is not needed at all.

**The bound is explicit.** `MAX_DECOMPRESSED_PAGE_BYTES` is 64 MiB per
page, and the extraction goes through `extract_text_with_limit`.

**The pages are passed explicitly.** `lopdf` extracts by page, not over
the whole document, so the page numbers come from `doc.get_pages()`. A
document with no pages yields empty text and is NOT an error — an empty
or image-only document is a valid thing to read.

**The fixture computes its own offsets.** The xref entries and the stream
length are derived from the bytes rather than written down, so the fixture
cannot drift when the body changes.

**The extraction gets its own test.** `the_extractor_reads_the_text_the_pdf_carries`
asserts the extracted string contains "Hello PDF". That is the test the
tool-level test could not be, because the tool-level test has to tolerate
a malformed fixture.

## Evidence

- `cargo tree -p operant-tools --features rag-pdf`: zero `ttf-parser`
  occurrences.
- `cargo audit --json`: 0 vulnerabilities, 2 unmaintained advisories
  (`bincode` and `unsafe-libyaml`, both owner-decided in AD-033) — down
  from 3.
- `scripts/supply-chain-gate.sh`: `seen=2 allowlist=2 new=0 stale=0`. The
  RT-27 entry was removed from `.ci/advisory-allowlist.txt`, so the gate
  would FAIL if the advisory came back.
- `cargo test -p operant-tools --features rag-pdf --lib -- pdf_read`: 16
  passed, 0 failed — including three new ones:
  - `the_extractor_reads_the_text_the_pdf_carries` — the extracted text
    carries the PDF's text.
  - `a_document_with_no_pages_yields_no_text` — no pages is not an error.
  - `a_file_that_is_not_a_pdf_is_an_error` — a non-PDF is an error, not a
    panic and not empty text.

## Consequences

- `ttf-parser` is out of the dependency graph. The `rag-pdf` feature is
  still off by default, so the default `operant-cli` graph is unchanged.
- A PDF decompression bomb is bounded at 64 MiB per page instead of
  unbounded.
- The PDF fixture is self-consistent, and the extractor is actually
  verified to extract text.

## Open

- The `rag-pdf` feature is still not in the default build, so the PDF tool
  is registered but reports a clear error unless the feature is built. That
  is the existing design, unchanged.
- The `lopdf` 0.45 text extraction differs from `pdf-extract`'s in the
  details (line breaks, font encodings). The tool's contract is "returns
  all readable text", which both satisfy; a PDF that relied on
  `pdf-extract`'s exact line breaking would render slightly differently.
