# RT-3 red-team: disabled-tool enforcement and content-install validation

Subject: remediation commit `ae9a27cf` (`fix(iter-2490)`), reviewed against current `HEAD` `81bcf22e`.

## Executive result

| Finding | Verdict | Severity / disposition |
|---|---|---|
| Z17: disabled tools were only hidden from listings | **PARTIALLY FIXED** | **HIGH**. Delete the bypass before claiming enforcement: the `tool_call` bridge checks `ToolRegistry::is_available()` first and then calls `execute()`; this is safe only because the bridge happens to pre-screen. The enforcement rule is not universal. Direct trait calls and at least one production caller remain outside the registry. Also, name/toolset matching is case-sensitive and whitespace-sensitive. |
| Z25: content install accepted an empty `SKILL.md` | **CONFIRMED FIXED for the stated 0-byte case** | **MEDIUM residual semantic gap**. A whitespace-only, comment-only, or frontmatter-only manifest still installs as a visually non-empty ruleset with no usable body. The original 0-byte file is rejected before the publish/swap. |

## Z17 — disabled tool enforcement

### Does the original defect stay closed?

Yes, for calls that enter the normal `ToolRegistry::execute()` route with the canonical registered name:

- Both sets are checked by the shared predicate: `crates/operant-core/src/tools.rs:839-848`.
- The normal schema paths use it: `get_schemas()` at `:854-869`, and `get_available_schemas_filtered()` at `:923-941`.
- The executor reads both sets before dispatch and returns a failed `ToolResult` without invoking the tool: `:1113-1186`. This includes unavailable tools, not only disabled ones.
- The agent's normal single, serial, and parallel branches all invoke this same registry entry point: `crates/operant-core/src/agent/stream.rs:1763`, `:1833`, `:1920`.
- The direct MCP server tool-call route also invokes the registry entry point: `crates/operant-cli/src/mcp_serve.rs:372-405`.
- The `tool_call` bridge delegates to the registry too, but only after its own availability gate: `crates/operant-core/src/tools/tool_search.rs:620-652`.

Therefore the specific regression (`registry.execute("terminal", ...)` while `terminal` is in `disabled_names`) is closed.

### Remaining execution bypasses

**HIGH — direct trait dispatch is not governed by the disable list.** `OperantTool::execute` remains a public trait method (`crates/operant-core/src/tools.rs:345`), and production code can instantiate or obtain a tool and call it without `ToolRegistry::execute`. The clearest production example is Telegram voice enrichment:

- `TranscriptionTool::new()` is called directly: `crates/operant-cli/src/gateway_runner/enrichment.rs:121-127`.
- `OperantTool::execute` is invoked directly at line 127.

`operant tools disable transcription` therefore cannot stop that path. The new audit claim says “the tool registry” or “dispatch boundary,” but this is not the registry boundary. Plugin tools, MCP tools, AFT tools, and ordinary tools are only covered when they go through a registry built with the same disabled sets and dispatched through `execute`; the trait itself has no enforcement. Direct uses in non-test source must be inventoried and either routed through a central enforcement wrapper or explicitly documented as intentionally outside operator tool bans.

### Alias / whitespace / case behavior

- Alias resolution happens before the disabled check: `crates/operant-core/src/tools.rs:1120-1126`. A disabled canonical name is caught because execution checks the resolved name at `:1162-1176`.
- Matching is exact `HashSet::contains`, not trimmed or normalized: `crates/operant-core/src/tools.rs:845-848`; mutation methods insert the caller string verbatim at `:751-764`.
- Matching is case-sensitive. A tool registered as `terminal`, disabled as `Terminal`, has a differently cased name, or has leading/trailing whitespace, will not match. No central normalization exists in the predicate or registration path (`register` uses `tool.name()` verbatim at `:679-687`).
- The name/toolset choice is correct in principle: a name match and a toolset match are ORed, so the deny condition wins inside the predicate.

**Impact:** ordinary model calls usually use the exact visible name, so this is not the main bypass, but operator-entered CLI/config values are not normalized and can silently fail to disable the intended tool. Treat this as part of the HIGH finding rather than a separate cosmetic issue.

### Interceptors, subagents, and TOCTOU

- Pre/post interceptors run only after the new disabled refusal in `execute`: `crates/operant-core/src/tools.rs:1155-1185`, then `:1200-1235`. A disabled tool is not sent to interceptors.
- Subagent registration separately excludes inherited disabled names/toolsets: `crates/operant-core/src/tools/sub_agent_tool.rs:1581-1597`; however, this does not cure direct trait dispatch in the child process/code.
- There is a real check-to-execute interval: both disabled sets are read at `:1155-1158`, and the executor runs after those guards are dropped. A concurrent enable/disable cannot change the already-captured decision. This is a TOCTOU window in the literal sense. Its effect is bounded (one in-flight call may use the old decision), but the schema and execution paths are separate calls and can observe different states even without mutation. No generation/version or per-call immutable policy snapshot exists.

### Regression-test mutation quality

The added tests are strong for the registry route but not sufficient for the whole finding:

- `a_tool_disabled_by_name_is_refused_by_execute` checks both schema removal and failed execution: `crates/operant-core/src/tools.rs:1440-1478`.
- `a_tool_disabled_by_toolset_is_refused_by_execute` covers toolset matching at `:1480-1510`.
- Enabled and empty-set controls are at `:1512-1560`.
- Reverting only the `execute()` refusal should fail the first two tests, so there is good mutation coverage for that narrow change.
- No test invokes `ToolCallTool`, an interceptor, a concurrent mutation, a direct `OperantTool::execute`, whitespace/case variants, or a non-registry production caller. Therefore the suite would pass even with the bypass documented above.

## Z25 — content-install manifest validation

### Does the original 0-byte defect stay closed?

Yes.

- The manifest is checked with `symlink_metadata`, including regular-file and zero-length checks: `crates/operant-core/src/binary_updater.rs:853-897`.
- It is called for every extracted skill directory before the guard scan: `:804-826`.
- Extraction and scanning finish before the staging tree is swapped: `:662-668`.
- On any validation error, staging is removed and the function returns before touching the old version: `:665-667`.
- The Unix swap moves the old version aside and restores it if the publish rename fails: `:690-713`.
- The regression test creates a real old version, supplies a zero-byte new `SKILL.md`, asserts the old bytes remain, and asserts no version/staging leftovers: `:1246-1330` (line range from the current test block).

### Residual semantic gaps

**MEDIUM — “non-empty” means byte length only.** `meta.len() == 0` is the sole content test at `crates/operant-core/src/binary_updater.rs:887-895`. It does not trim or parse the manifest. A file containing only spaces/newlines, only comments, or only YAML frontmatter with no body is accepted and can still produce an empty rendered ruleset. This is not the original zero-byte finding, but it leaves the same operator-visible outcome for a slightly modified payload.

**Scope is correct for the implemented content kind, but narrow by design.** The loop only checks directories under the extracted content subdirectory (`:809-826`) and only for `InstallKind::Content` (`:626-631`). It is not a universal skill-store validator and does not validate arbitrary pre-existing skills or non-content installation paths. No defect is proved for those paths by this remediation.

**Partial extraction:** the extractor writes each file as it goes (`:779-783`), and the validator runs only after extraction returns (`:663-665`). A process killed mid-extraction can leave a staging directory, but the deterministic staging path is removed on the next install (`:656-661`), and it is not swapped into the live version directory. A partial live install is therefore not shown by this code. The stated “old version really survives” property holds for validation errors; abrupt process termination during the final swap is a separate atomicity question and is not covered by a crash-injection test.

### Mutation quality

The tests cover the important original case:

- zero-byte manifest refuses and old content survives;
- missing manifest refuses;
- non-empty manifest installs;
- manifest check precedes dangerous-content scan.

Reverting `require_nonempty_skill_md` should fail the first two tests. They do **not** fail for whitespace-only, comments-only, or frontmatter-only manifests; those mutations would pass. No test simulates a kill between extraction and swap or a failing final rename.

## Required closure work

1. **HIGH:** make the disabled policy mandatory at every execution route, including direct `OperantTool::execute` callers, or remove/redefine the public unrestricted trait entry point. Add a production-path test using the Telegram transcription path and a direct plugin/tool call.
2. **HIGH:** normalize tool name/toolset policy once and test case and surrounding-whitespace variants. Decide and document whether this is intentional exact-match behavior; do not leave it as an implicit trap.
3. **MEDIUM:** replace the Z25 byte-length check with a defined manifest-content predicate (at minimum trim-whitespace; preferably require a non-empty body after supported frontmatter) and add those mutation tests.
4. **MEDIUM:** add a crash/fault test around staging-to-live swap if the claim includes preservation under abrupt termination, not only ordinary validation failure.

## Verification limits

The remediation commit and current source were read. Scoped test execution was attempted for both areas, but test compilation is blocked by unrelated current-tree errors in `crates/operant-core/src/gateway/mod.rs:3096` and `crates/operant-core/src/harness_seams_r3.rs:448` (`handle_update` is not a member of `PlatformAdapter`). No workspace build was run. Claims about runtime outcomes beyond the inspected code and test bodies are marked above as residual or unverified where appropriate.