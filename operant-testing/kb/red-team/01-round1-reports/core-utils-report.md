# Test Report — Core Utilities

**Date:** 2026-11-05T00:34:00Z  
**Workspace:** ./operant-testing/  
**Category:** Core Utilities  
**Test Count:** 8  
**Passed:** 8 | **Failed:** 0 | **Degraded:** 0

## Test Results

| # | Tool | Test | Input | Result | Status |
|---|---|---|---|---|---|
| 1 | `echo` | Connectivity smoke test | "operant-testing core utility smoke test" | Returned input unchanged | ✅ |
| 2 | `datetime` | Date/time formatting | format=%Y-%m-%dT%H:%M:%SZ | 2026-11-05T00:34:09Z, unix 1793838849 | ✅ |
| 3 | `timestamp` | Unix timestamp | unit=seconds | 1793838849 | ✅ |
| 4 | `debug_system` | System info | — | x86_64, 24 CPU, 46 GB RAM, linux, rustc 1.98.0 | ✅ |
| 5 | `debug_env` | Environment variables | filter=PATH | 8 variables returned (names only) | ✅ |
| 6 | `process` | Process registry | action=list | running_count=0 | ✅ |
| 7 | `env_probe` | Secret exposure audit | — | 5 secret-looking vars named (values hidden) | ✅ |
| 8 | `checkpoint` | Checkpoint listing | action=list | 11 checkpoints found | ✅ |

## Notes

- All core utility tools are fully operational.
- `env_probe` correctly masks values (names only, never values).
- `debug_system` confirms Rust 1.98.0 on Linux x86_64 with 24 CPUs and 46 GB RAM.
