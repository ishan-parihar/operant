---
ID: "AD-041"
Title: "The PDF decompression bound covers the load path and the whole document, so AD-039's claim is now true"
Status: "Active"
Date: "2026-10-19"
Description: "AD-039 published 'the decompression bomb is now bounded', but only the extraction path was bounded: Document::load_mem uses LoadOptions::default(), whose max_decompressed_size is None, so the object and xref streams the loader decodes eagerly were unbounded; the load is now bounded too, and the per-page budget is divided across the document"
---

# AD-041: The PDF decompression bound covers the load path and the whole document

Immutable once numbered: a correction is a new record that supersedes this
one.

## Context

AD-039 (iter-987) ported `pdf_read` from `pdf-extract` 0.12 to `lopdf`
0.45 — the point being that `pdf-extract` pins `lopdf` 0.42, which reaches
`ttf-parser` (RUSTSEC-2026-0192). The record also claimed, in its title and
in its body:

> **The decompression bomb is now bounded.** A PDF page's content stream is

and

> A PDF decompression bomb is bounded at 64 MiB per page instead of
> unbounded.

**That claim was overstated when it was published.** The bound was applied
to the EXTRACT path only — `doc.extract_text_with_limit(&pages, 64 MiB)`.
The LOAD path was not bounded at all:

```rust
let doc = lopdf::Document::load_mem(bytes)?;   // LoadOptions::default()
```

`LoadOptions::default()` has `max_decompressed_size: None`
(lopdf-0.45.0/src/load_options.rs:51). The loader decodes the object
streams and the cross-reference streams EAGERLY, when the document is
read — so the largest decompression in the whole PDF happened before the
per-page limit was ever consulted. The vector the port was meant to close
was still open.

There was a second half: the per-page limit is a limit PER PAGE. A
document with 1,000 pages still costs 1,000 x 64 MiB, so a "bounded"
extraction could still expand to 64 GB.

## Decision

**Bound both paths that decompress, and give the document its own budget.**

1. **The load is bounded.** The document is read with
   `Document::load_mem_with_options(bytes, LoadOptions::with_max_decompressed_size(MAX_DECOMPRESSED_PAGE_BYTES))`,
   so the object and xref streams are bounded by the same 64 MiB.

2. **The document has a budget, not just the page.** The per-page limit is
   divided across the pages actually read:
   `MAX_DECOMPRESSED_PAGE_BYTES / pages.len()`, floored at 1 MiB so a
   document with many pages still gets a usable per-page budget. A 1,000
   page document now costs at most 64 MiB in total, not 64 GiB.

3. `MAX_PAGES_PER_DOCUMENT` (256) names the page ceiling the budget
   assumes.

## Why this is a new record and not an edit to AD-039

AD-039 is immutable once numbered. Its claim was wrong when it was
published, and the honest thing is to record the correction rather than to
quietly make the old record true by editing it.

**After this record, AD-039's claim is accurate**: the decompression bomb
IS bounded, on both paths, for the whole document.

## Evidence

The PDF tests in `crates/operant-tools/src/pdf_read.rs`:
`the_extractor_reads_the_text_the_pdf_carries` (the unit level),
`extracts_text_from_valid_pdf` (the tool level — strengthened in iter-1012
so it can no longer pass on an error), plus the no-pages, image-only,
truncation and not-a-PDF cases. `cargo test --workspace --all-features`
— 7201 passed, 0 failed.

## Consequences

- A PDF whose object or xref stream decompresses to more than 64 MiB is
  now refused at load time with a clear error, instead of being expanded
  in the process's memory.
- A document with many pages cannot multiply the budget.
- The honest limit is now written down: 64 MiB of decompression per
  document, divided across its pages.

## Open

- The 64 MiB per document is a constant, not a configuration key. A
  legitimate 500-page report would be refused. It should be a key.
