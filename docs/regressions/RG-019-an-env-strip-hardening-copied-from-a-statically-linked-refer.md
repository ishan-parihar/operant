---
ID: "RG-019"
Title: "an env-strip hardening copied from a statically-linked reference breaks a dynamically-linked binary"
Status: "Active"
Date: "2026-10-31"
Description: "an env-strip hardening copied from a statically-linked reference breaks a dynamically-linked binary"
Severity: "High"
---

# RG-019: an env-strip hardening copied from a statically-linked reference breaks a dynamically-linked binary

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

A hardening rule read as an unambiguous security win turns out to disable a designed default.
Writing the 2026-10-31 codex contrast audit, the `LD_*` environment strip from codex's
`process-hardening/src/lib.rs:61` was first recorded as **IMPORT-SMALL** — "strip `LD_*` so a
hostile library cannot be interposed". Measuring `ldd target/release/operant` showed:

```
libonnxruntime.so.1 => /home/ishanp/.local/lib/operant/libonnxruntime.so.1
```

The binary is dynamically linked and resolves its ONNX Runtime out of the user's own library
directory. `scripts/dev-env.sh` sets `LD_LIBRARY_PATH` for exactly this reason, and the repo's
documented design preference is `[tts] provider = "kokoro"` — Kokoro loads over that runtime.
Stripping `LD_*` would have broken the default voice engine in the name of hardening it.

## Root cause

The rule was copied without checking the premise that makes it safe in the source tree. codex's own
comment states the premise explicitly (`process-hardening/src/lib.rs:57-59`): *"Official Codex
releases are MUSL-linked, which means that variables such as LD_PRELOAD are ignored anyway, but
just to be sure, clear them here."* For codex the strip is belt-and-braces on an already-static
surface — it changes nothing and costs nothing. For a glibc, dynamically-linked binary whose
one exotic shared library is loaded by path, the same strip removes a runtime dependency.

The general failure class: **a hardening rule's safety is not a property of the rule. It is a
property of the rule plus the link model, the loader configuration, and the deployment of the
binary it is applied to.** Copying the rule and not the premise transfers a no-op into a defect.

## Law

Before importing any security hardening from a reference tree, measure the property it depends on
in **our** binary, and record the measurement in the same commit as the import. For environment
strips specifically: run `ldd` on the release binary, name every library that does not resolve
from a system path, and check the dev-env/provisioning scripts for the variables that make them
resolve. If stripping a variable breaks a documented default, the rule is DO-NOT here regardless of
how correct it is there.

Where a narrow version is genuinely safe (stripping only `LD_PRELOAD`, which does not affect
resolution paths), it is a **separate, separately-measured** row — not a concession granted to the
broad one.

## Detection

`ldd target/release/operant | grep -v '^\s'` and read the resolved paths; `grep -n LD_LIBRARY_PATH
scripts/dev-env.sh`. A stripping import whose measurement is missing from its own commit body is
unproven, not safe.

Recorded at `docs/audit/2026-10-31-codex-architecture-deepen/07-process-hardening.md`, where the
row itself carries the DO-NOT verdict and this reasoning rather than quietly dropping the finding.

## Orthogonality

AD-035 owns libclang path resolution and RG-013/RG-017 own gate methodology; neither covers importing a hardening rule whose safety depends on the reference's link model, which is a different failure class: the rule is correct in its own tree and destructive in ours (recon R103117512266c: LD_LIBRARY_PATH load-bearing dynamic link onnxruntime)

## Revisions

- 2026-10-31: created
