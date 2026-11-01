//! Patch tool
//!
//! A file patch tool that does targeted find-and-replace.
//! Matches Python's patch tool from file_tools.py.
//! Supports exact matching with a fuzzy fallback that normalizes
//! whitespace and line endings.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::schema::ToolSchema;
use crate::tools::file_tools::{atomic_write, holon_write_fence, validate_path};
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Arguments for the patch tool
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchArgs {
    /// Path to the file to patch
    path: String,
    /// Exact string to find in the file
    find: String,
    /// Replacement string
    replace: String,
    /// Which occurrence to edit when `find` matches more than once.
    ///
    /// Codex's contract: an ambiguous find is REFUSED, not guessed. Supply
    /// `0..count` to name one; leave it out and the tool names the count
    /// instead of picking for you.
    #[serde(default)]
    occurrence: Option<usize>,
}

/// Tool for targeted find-and-replace patching of files
pub struct PatchTool;

/// Normalize a string for fuzzy matching: trim each line, collapse
/// runs of whitespace, and normalize line endings to \n.
fn normalize_whitespace(s: &str) -> String {
    s.replace("\r\n", "\n")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Attempt a fuzzy replacement. Normalizes both the file content and
/// the `find` string, locates the match in the normalized content,
/// then maps back to the original content using a line-based approach.
/// Fuzzy (whitespace-insensitive) replacement.
///
/// `occurrence` names which matched block to rewrite. It MUST be honoured:
/// the caller gates on the match count and then trusts this function to rewrite
/// only the block it was asked for. Before, the parameter did not exist and an
/// ambiguous fuzzy match rewrote EVERY block -- `occurrence: 1` on two matches
/// rewrote both, which is the exact failure the occurrence gate exists to
/// prevent, surviving on the fuzzy tier because the gate only checked the count
/// and this function did what it liked afterwards.
fn fuzzy_replace(
    content: &str,
    find: &str,
    replace: &str,
    occurrence: Option<usize>,
) -> Option<(String, usize)> {
    let normalized_content = normalize_whitespace(content);
    let normalized_find = normalize_whitespace(find);

    if normalized_find.is_empty() {
        return None;
    }

    // Count occurrences in normalized form
    let count = normalized_content.matches(&normalized_find).count();
    if count == 0 {
        return None;
    }

    // Work line-by-line: find which normalized lines match the normalized find lines
    let content_lines: Vec<&str> = content.lines().collect();
    let find_lines: Vec<&str> = normalized_find.lines().collect();

    if find_lines.is_empty() {
        return None;
    }

    let mut replacements = 0;
    let mut result_lines: Vec<String> = Vec::new();
    let mut i = 0;
    // The match count the GATE must see: every match in the file, whether or
    // not this run rewrites it. Counting only the rewritten ones would hand the
    // gate a smaller number, so `occurrence: 1` over two matches would be judged
    // `Ambiguous` and refused -- which is safe, but it silently turns a
    // perfectly legal "edit the second one" into a dead end.
    let mut seen_matches = 0usize;

    while i < content_lines.len() {
        // Check if lines starting at `i` match the normalized find lines
        if i + find_lines.len() <= content_lines.len() {
            let mut matched = true;
            for (j, find_line) in find_lines.iter().enumerate() {
                let content_normalized = content_lines[i + j]
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                if content_normalized != *find_line {
                    matched = false;
                    break;
                }
            }

            if matched {
                // When a specific occurrence was named, every OTHER matched
                // block is passed through untouched -- rewriting them all was
                // the defect. Only the named one is replaced.
                //
                // TWO counters, deliberately: `seen_matches` is what the gate
                // judges (how many places match at all), `replacements` is what
                // the tool reports (how many places this run actually changed).
                let is_target = match occurrence {
                    Some(picked) => seen_matches == picked,
                    None => true,
                };
                if is_target {
                    // Only the named block is rewritten. `lines()` yields nothing
                    // at all for an empty `replace`, which is exactly the
                    // delete-a-line case.
                    for line in replace.lines() {
                        result_lines.push(line.to_string());
                    }
                    replacements += 1;
                } else {
                    // Pass the block through untouched, exactly as the exact
                    // tier leaves its non-picked occurrences alone.
                    //
                    // Per the Rust standards, avoid cloning in the loop: this is
                    // a cold path (one patch call), so the iterator is over an
                    // index range and each line is copied exactly once.
                    result_lines
                        .extend((i..i + find_lines.len()).map(|k| content_lines[k].to_string()));
                }
                seen_matches += 1;
                i += find_lines.len();
                continue;
            }

            result_lines.push(content_lines[i].to_string());
            i += 1;
        }
    }

    if replacements > 0 {
        // Preserve the file's line endings, not just its newlines.
        //
        // The old writer joined with "\n" unconditionally, which converted
        // every CRLF (and lone-CR) line in the file to LF on any fuzzy patch.
        // Row 4 shipped `SourceFile`/`LineEnding` in tools/patch/mod.rs to fix
        // exactly this, but that module was never wired to this tool -- so the
        // fidelity the port exists for did not reach the operator. Mirrored
        // here at the one place it matters.
        //
        // The trailing-ending branch is what makes the LAST line faithful: it
        // appends the file's own ending, not a hard-coded "\n", so a CRLF
        // file's final line stays CRLF (the old code appended "\n" and
        // silently produced a mixed-ending file).
        let ending = dominant_line_ending(content);
        let mut result = result_lines.join(ending);
        if (content.ends_with('\n') || content.ends_with('\r'))
            && !result.is_empty()
            && !result.ends_with(ending)
        {
            result.push_str(ending);
        }
        // The GATE count, not the applied count: the caller must be able to
        // distinguish "one place matched" from "two matched and you named one".
        // Returning `replacements` alone would hand the gate a 1 and make a
        // legal "edit the second one" impossible.
        Some((result, seen_matches))
    } else {
        None
    }
}

/// The line ending a file predominantly uses, so a rewrite can keep it.
///
/// Mirrors `crate::tools::patch::SourceFile`'s rule: the first ending found
/// wins, and a file with none at all defaults to LF. `normalize_whitespace`
/// already normalized CRLF to LF for MATCHING, which is correct and intended;
/// this is about what gets WRITTEN back.
fn dominant_line_ending(content: &str) -> &'static str {
    let mut first: Option<&'static str> = None;
    let bytes = content.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            let ending = if bytes.get(i + 1) == Some(&b'\n') {
                i += 2;
                "\r\n"
            } else {
                i += 1;
                "\r"
            };
            first.get_or_insert(ending);
            continue;
        }
        if bytes[i] == b'\n' {
            first.get_or_insert("\n");
        }
        i += 1;
    }
    first.unwrap_or("\n")
}

/// Delete every line whose trimmed text equals `find`, keeping endings.
///
/// An empty `replace` on a whole-line `find` means "remove that line". Using
/// `str::replace` for it leaves a blank line behind, which is how the old
/// `patch` handled a delete: the text vanished but the line stayed, so the tool
/// could blank a line and never remove one.
///
/// Only whole-line matches count. A `find` that is a fragment of a longer line
/// falls through to the caller's `str::replace`, which deletes just the
/// fragment -- the behaviour a substring delete should have.
fn delete_lines_matching(content: &str, find: &str) -> String {
    let find_trimmed = find.trim();
    if find_trimmed.is_empty() {
        return content.to_string();
    }
    let ending = dominant_line_ending(content);
    let mut kept: Vec<&str> = Vec::new();
    let mut removed = 0usize;
    for line in content.lines() {
        if line.trim() == find_trimmed {
            removed += 1;
            continue;
        }
        kept.push(line);
    }
    // Nothing matched as a whole line, so this was a fragment delete: the
    // caller's plain replace is the right behaviour.
    if removed == 0 {
        return content.replace(find, "");
    }
    let mut out = kept.join(ending);
    if (content.ends_with('\n') || content.ends_with('\r')) && !out.is_empty() {
        out.push_str(ending);
    }
    out
}

#[async_trait]
impl OperantTool for PatchTool {
    fn name(&self) -> &str {
        "patch"
    }

    fn description(&self) -> &str {
        "Apply a targeted find-and-replace patch to a file. Finds an exact string \
        in the file and replaces it. Falls back to fuzzy matching (normalized whitespace \
        and line endings) if an exact match is not found."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<PatchArgs>("patch", "Find and replace in a file")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: PatchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("patch", format!("Invalid arguments: {}", e)),
        };

        // Path safety check — the same gate every other file tool applies
        // (file_read/write/search/list). Without it, `patch` was the only
        // file tool that could write through `..` traversal or into
        // sensitive files (SSH keys, .aws/credentials, /etc/shadow) that
        // file_write rejects.
        // O-18: the Holon policy-roots fence (writes only)
        if let Err(e) = holon_write_fence(&args.path) {
            return ToolResult::error("patch", e);
        }
        let path = match validate_path(&args.path) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("patch", e),
        };

        if !path.exists() {
            return ToolResult::error("patch", format!("File not found: {}", args.path));
        }

        if !path.is_file() {
            return ToolResult::error("patch", format!("Path is not a file: {}", args.path));
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => return ToolResult::error("patch", format!("Failed to read file: {}", e)),
        };

        // Try exact match first
        let exact_count = content.matches(&args.find).count();

        // OCCURRENCE GATE (D1): codex refuses an ambiguous find, and so does
        // the ported engine in `crate::tools::patch::OccurrenceRule` -- but that engine was
        // never called from here, so the registered tool kept the old
        // replace-every-occurrence behaviour and reported it as success. The
        // gate is applied to BOTH tiers so the fuzzy path cannot be a
        // quieter way to rewrite more than one place.
        let (new_content, replacements, match_type) = if exact_count > 0 {
            match crate::tools::patch::OccurrenceRule::resolve(exact_count, args.occurrence) {
                rule if rule.is_allowed() => {
                    // How many edits this run will ACTUALLY make. Reporting the
                    // raw match count here is a lie: a named occurrence rewrites
                    // exactly one place, so the tool once answered
                    // `"replacements": 3` for a single edit. The model reads this
                    // number to decide what it did, so it must be the truth.
                    let mut applied = exact_count;
                    let replaced =
                        if let crate::tools::patch::OccurrenceRule::Chosen { picked, .. } = rule {
                            // The caller named which occurrence: replace only that
                            // one, leaving the others untouched.
                            //
                            // Exactly ONE edit, whatever the match count is --
                            // this is what `applied` reports.
                            applied = 1;
                            let mut seen = 0usize;
                            let mut out = String::with_capacity(content.len());
                            let mut rest = content.as_str();
                            while let Some(at) = rest.find(&args.find) {
                                if seen == picked {
                                    out.push_str(&rest[..at]);
                                    out.push_str(&args.replace);
                                    rest = &rest[at + args.find.len()..];
                                    break;
                                }
                                let end = at + args.find.len();
                                out.push_str(&rest[..end]);
                                rest = &rest[end..];
                                seen += 1;
                            }
                            out.push_str(rest);
                            out
                        } else {
                            // A single occurrence: replace it.
                            //
                            // An EMPTY replacement removes the matched text,
                            // and when what matched was a whole line that must
                            // remove the LINE, not leave a blank one behind --
                            // otherwise `patch` cannot delete anything, it only
                            // blanks it. Deleting a mid-line substring still
                            // deletes just the substring, which is correct.
                            if args.replace.is_empty() {
                                delete_lines_matching(&content, &args.find)
                            } else {
                                content.replace(&args.find, &args.replace)
                            }
                        };
                    (replaced, applied, "exact")
                }
                rule => {
                    let why = rule
                        .refusal_message(&args.find)
                        .unwrap_or_else(|| "ambiguous find".to_string());
                    return ToolResult::error("patch", format!("{} (file '{}')", why, args.path));
                }
            }
        } else {
            // Fuzzy fallback
            match fuzzy_replace(&content, &args.find, &args.replace, args.occurrence) {
                Some((replaced, gate_count)) => {
                    // Same gate on the fuzzy tier: an ambiguous fuzzy match is
                    // just as ambiguous as an exact one. `gate_count` is how
                    // many places matched at all, which is what the gate must
                    // judge -- the APPLIED count is 1 when an occurrence was
                    // named, and reporting that as the gate's input would make
                    // a legal "edit the second one" unselectable.
                    let applied = match crate::tools::patch::OccurrenceRule::resolve(
                        gate_count,
                        args.occurrence,
                    ) {
                        rule if rule.is_allowed() => match rule {
                            // A NAMED occurrence rewrites exactly one block, so
                            // report exactly one -- not the total the gate saw.
                            crate::tools::patch::OccurrenceRule::Chosen { .. } => 1,
                            _ => gate_count,
                        },
                        rule => {
                            let why = rule
                                .refusal_message(&args.find)
                                .unwrap_or_else(|| "ambiguous find".to_string());
                            return ToolResult::error(
                                "patch",
                                format!("{} (file '{}')", why, args.path),
                            );
                        }
                    };
                    (replaced, applied, "fuzzy")
                }
                None => {
                    return ToolResult::error(
                        "patch",
                        format!(
                            "Find string not found in file '{}' (tried both exact and fuzzy matching)",
                            args.path
                        ),
                    );
                }
            }
        };

        // Write the patched content back — atomically (hermes _atomic_write
        // parity) so a crash mid-write cannot corrupt the file.
        if let Err(e) = atomic_write(&path, &new_content) {
            return ToolResult::error("patch", format!("Failed to write file: {}", e));
        }

        ToolResult::success(
            "patch",
            serde_json::json!({
                "path": args.path,
                "replacements": replacements,
                "matchType": match_type,
                "fileSize": new_content.len(),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;

    fn default_context() -> ToolContext {
        ToolContext::default()
    }

    fn create_temp_file(content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "operant_patch_test_{}.txt",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[tokio::test]
    async fn test_patch_rejects_denied_sensitive_path() {
        // `patch` must apply the same security gate as file_write: a write
        // into a sensitive path (here `.netrc` in a scratch dir) is refused.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".netrc");
        std::fs::write(&path, "machine example.com login x password y\n").unwrap();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "find": "password y",
                    "replace": "password z"
                }),
                default_context(),
            )
            .await;

        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("denied"), "expected denied, got: {err}");
        // The file must be untouched.
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("password y"));
    }

    #[tokio::test]
    async fn test_patch_exact_match() {
        let path = create_temp_file("Hello, world!\nGoodbye, world!\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "Hello, world!",
                    "replace": "Hi, world!"
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["replacements"], 1);
        assert_eq!(parsed["matchType"], "exact");

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("Hi, world!"));
        assert!(!updated.contains("Hello, world!"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_file_not_found() {
        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "/nonexistent/file.txt",
                    "find": "a",
                    "replace": "b"
                }),
                default_context(),
            )
            .await;

        assert!(!result.success);
        assert!(result.error.unwrap().contains("File not found"));
    }

    #[tokio::test]
    async fn test_patch_find_not_found() {
        let path = create_temp_file("Some content here\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "nonexistent string",
                    "replace": "replacement"
                }),
                default_context(),
            )
            .await;

        assert!(!result.success);
        assert!(result.error.unwrap().contains("not found"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_multiple_replacements() {
        // D1 (codex's contract): an ambiguous find is REFUSED and the count is
        // named, never a blind rewrite of every occurrence.
        //
        // This test used to assert the opposite -- that all three occurrences
        // were replaced and reported as success. The row-4 engine in
        // `tools/patch/mod.rs` shipped `OccurrenceRule` to close exactly this,
        // but it was never called from the registered tool (the port landed as
        // an orphan module with zero callers), so the old behaviour was still
        // what shipped. The refusal is now enforced at the tool.
        let path = create_temp_file("foo bar foo baz foo\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "foo",
                    "replace": "qux"
                }),
                default_context(),
            )
            .await;

        assert!(
            !result.success,
            "a find matching three times must be refused, not rewritten: {:?}",
            result.error
        );
        // The message IS the contract: the model can only act on the count if
        // it is named. `ToolResult::error` puts it in `error`, not `content`.
        let message = result.error.as_deref().unwrap_or("");
        assert!(
            message.contains("3 times") || message.contains("three"),
            "the refusal must name the count so the model can act on it: {message}"
        );

        // The file must be untouched: a refusal writes nothing.
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after, "foo bar foo baz foo\n",
            "a refused ambiguous find must leave the file byte-identical"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_occurrence_index_selects_one() {
        // The escape hatch the refusal message offers: name which occurrence.
        let path = create_temp_file("foo bar foo baz foo\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "foo",
                    "replace": "qux",
                    "occurrence": 1
                }),
                default_context(),
            )
            .await;

        assert!(
            result.success,
            "a named occurrence must be applied: {}",
            result.content
        );
        let updated = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            updated, "foo bar qux baz foo\n",
            "only occurrence 1 changes; the others stay"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_occurrence_index_out_of_range_is_refused() {
        // An index past the count is an ambiguous find, not a silent pick.
        let path = create_temp_file("foo bar foo baz foo\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "foo",
                    "replace": "qux",
                    "occurrence": 7
                }),
                default_context(),
            )
            .await;

        assert!(!result.success, "an out-of-range index must be refused");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "foo bar foo baz foo\n"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_single_occurrence_still_succeeds() {
        // The refusal must not make the common single-match case worse.
        let path = create_temp_file("only one foo here\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "foo",
                    "replace": "qux"
                }),
                default_context(),
            )
            .await;

        assert!(
            result.success,
            "a unique find must still apply: {}",
            result.content
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "only one qux here\n"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_fuzzy_ambiguous_is_refused_too() {
        // The gate applies to the fuzzy tier as well: normalised whitespace
        // must not become a quieter way to rewrite more than one place.
        let path = create_temp_file("fn  a()  {}\nfn  a()  {}\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "fn a() {}",
                    "replace": "fn a() { /* changed */ }"
                }),
                default_context(),
            )
            .await;

        assert!(
            !result.success,
            "an ambiguous FUZZY find must be refused too: {}",
            result.content
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fn  a()  {}\nfn  a()  {}\n"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_named_occurrence_reports_one_replacement_not_the_match_count() {
        // Row 4 red-team (2026-11-01), defect D3. The reported
        // `replacements` was the raw MATCH count, so a named occurrence over
        // three matches answered `"replacements": 3` for the one edit it
        // actually made. The model reads that number to reason about what it
        // did, so it has to be the truth.
        let path = create_temp_file("foo bar foo baz foo\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "foo",
                    "replace": "qux",
                    "occurrence": 1
                }),
                default_context(),
            )
            .await;

        assert!(
            result.success,
            "a named occurrence must apply: {}",
            result.content
        );
        let reported: serde_json::Value =
            serde_json::from_str(&result.content).expect("the success payload is JSON");
        assert_eq!(
            reported["replacements"], 1,
            "one occurrence was named and one edit was made, so the count is 1 -- \
             reporting the match count tells the model it changed three places"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "foo bar qux baz foo\n"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_named_occurrence_is_honoured_on_the_fuzzy_tier() {
        // Row 4 red-team (2026-11-01), defect D4 -- the worse of the two. The
        // occurrence gate was applied to the fuzzy tier's COUNT, but
        // `fuzzy_replace` had no occurrence parameter, so it rewrote EVERY
        // matched block regardless. `occurrence: 1` over two fuzzy matches
        // rewrote both: the exact failure the gate exists to prevent, surviving
        // on the tier that was supposed to be the safer one.
        let path = create_temp_file("the quick  fox\njumps\nthe quick fox\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "the   quick   fox",
                    "replace": "SLOW",
                    "occurrence": 1
                }),
                default_context(),
            )
            .await;

        assert!(
            result.success,
            "the second of two fuzzy matches must be reachable: {}",
            result.content
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "the quick  fox\njumps\nSLOW\n",
            "only the NAMED block may change; the first match must be left alone"
        );
        let reported: serde_json::Value =
            serde_json::from_str(&result.content).expect("the success payload is JSON");
        assert_eq!(reported["replacements"], 1);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn the_fuzzy_tier_with_no_occurrence_still_replaces_every_match() {
        // The bookend for D4: honouring `occurrence` must not quietly change the
        // default. With no index named, the fuzzy tier keeps its documented
        // replace-every-match behaviour -- which the ambiguity gate refuses
        // before it gets there, so this reaches it through `fuzzy_replace`
        // directly with a single match.
        let out = fuzzy_replace("a  b\nc\n", "a b", "X", None).expect("one match");
        assert_eq!(out.1, 1, "gate count is the number of matches");
        assert_eq!(out.0, "X\nc\n");
    }

    #[tokio::test]
    async fn test_patch_fuzzy_whitespace() {
        let path = create_temp_file("  fn  main()  {\n    println!(\"hi\");\n  }\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "fn main() {\nprintln!(\"hi\");\n}",
                    "replace": "fn main() {\n    println!(\"hello\");\n}"
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["matchType"], "fuzzy");

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("hello"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(
            normalize_whitespace("  hello   world  \r\n  foo  bar  "),
            "hello world\nfoo bar"
        );
    }

    #[test]
    fn test_fuzzy_replace_basic() {
        let content = "  fn  foo()  {\n    bar();\n  }\n";
        let find = "fn foo() {\nbar();\n}";
        let replace = "fn foo() {\n    baz();\n}";

        let result = fuzzy_replace(content, find, replace, None);
        assert!(result.is_some());
        let (replaced, count) = result.unwrap();
        assert_eq!(count, 1);
        assert!(replaced.contains("baz"));
    }

    #[test]
    fn test_normalize_whitespace_empty() {
        assert_eq!(normalize_whitespace(""), "");
    }

    #[test]
    fn test_normalize_whitespace_only_whitespace() {
        assert_eq!(normalize_whitespace("   \n  \n  "), "\n\n");
    }

    #[test]
    fn test_normalize_whitespace_mixed_line_endings() {
        assert_eq!(normalize_whitespace("foo\r\nbar\r\nbaz"), "foo\nbar\nbaz");
    }

    #[test]
    fn test_normalize_whitespace_no_change() {
        let input = "hello world\nfoo bar\n";
        assert_eq!(normalize_whitespace(input), "hello world\nfoo bar");
    }

    #[test]
    fn test_fuzzy_replace_empty_find() {
        let result = fuzzy_replace("some content", "", "replacement", None);
        assert!(result.is_none());
    }

    #[test]
    fn test_fuzzy_replace_no_match() {
        let result = fuzzy_replace("hello world", "goodbye", "hi", None);
        assert!(result.is_none());
    }

    #[test]
    fn test_fuzzy_replace_exact_equivalent() {
        // When content is already normalized, fuzzy_replace should still match
        let content = "fn main() {\n    return;\n}\n";
        let find = "fn main() {\n    return;\n}";
        let replace = "fn main() {\n    return 0;\n}";
        let result = fuzzy_replace(content, find, replace, None);
        assert!(result.is_some());
        let (replaced, count) = result.unwrap();
        assert_eq!(count, 1);
        assert!(replaced.contains("return 0"));
    }

    #[test]
    fn test_fuzzy_replace_delete_content() {
        let content = "keep\nremove\nkeep\n";
        let find = "remove";
        let replace = "";
        let result = fuzzy_replace(content, find, replace, None);
        assert!(result.is_some());
        let (replaced, count) = result.unwrap();
        assert_eq!(count, 1);
        // Deleting a line with empty replacement removes it; surrounding lines remain
        assert_eq!(replaced, "keep\nkeep\n");
    }

    #[test]
    fn test_fuzzy_replace_trailing_newline_preserved() {
        let content = "line1\nline2\nline3\n";
        let find = "line2";
        let replace = "modified";
        let result = fuzzy_replace(content, find, replace, None);
        assert!(result.is_some());
        let (replaced, _) = result.unwrap();
        assert!(
            replaced.ends_with('\n'),
            "trailing newline should be preserved"
        );
    }

    #[test]
    fn test_fuzzy_replace_no_trailing_newline() {
        let content = "line1\nline2\nline3";
        let find = "line2";
        let replace = "modified";
        let result = fuzzy_replace(content, find, replace, None);
        assert!(result.is_some());
        let (replaced, _) = result.unwrap();
        assert!(
            !replaced.ends_with('\n'),
            "no trailing newline added if original lacked it"
        );
    }

    #[tokio::test]
    async fn test_patch_delete_content() {
        let path = create_temp_file("Hello, world!\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "Hello, world!",
                    "replace": ""
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["replacements"], 1);

        // The whole-file delete must leave an EMPTY file, not a stray
        // newline. The old assertion here pinned `"\n"` -- which was the
        // leave-a-blank-line defect in action: the text was removed but the
        // LINE it occupied stayed. A delete that cannot actually delete is not
        // a delete, so this expectation is corrected rather than preserved.
        let updated = std::fs::read_to_string(&path).unwrap();
        assert_eq!(updated, "");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_path_is_directory() {
        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "/tmp",
                    "find": "a",
                    "replace": "b"
                }),
                default_context(),
            )
            .await;

        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("not a file"));
    }

    #[tokio::test]
    async fn test_patch_no_trailing_newline() {
        let path = create_temp_file("line1");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "line1",
                    "replace": "replaced"
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let updated = std::fs::read_to_string(&path).unwrap();
        assert_eq!(updated, "replaced");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_fuzzy_fallback_with_extra_whitespace() {
        let path = create_temp_file("  hello   world\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "hello world",
                    "replace": "hi there"
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["matchType"], "fuzzy");

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("hi there"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_fuzzy_preserves_crlf() {
        // D2's sibling: row 4's LineEnding fidelity. `normalize_whitespace`
        // normalizes CRLF to LF for MATCHING (correct), but the WRITER joined
        // with "\n", so a fuzzy patch on a CRLF file converted the whole file.
        // The file's own ending must survive the rewrite.
        let path = create_temp_file("fn  a()  {\r\n  b();\r\n}\r\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "fn a() {\n  b();\n}",
                    "replace": "fn a() {\n  c();\n}"
                }),
                default_context(),
            )
            .await;

        assert!(
            result.success,
            "fuzzy CRLF patch must apply: {:?}",
            result.error
        );
        let updated = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            updated, "fn a() {\r\n  c();\r\n}\r\n",
            "every kept line must retain its CRLF ending"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_exact_preserves_crlf() {
        // The exact tier must be equally faithful.
        let path = create_temp_file("alpha\r\nbeta\r\ngamma\r\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "beta",
                    "replace": "BETA"
                }),
                default_context(),
            )
            .await;

        assert!(
            result.success,
            "exact CRLF patch must apply: {:?}",
            result.error
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "alpha\r\nBETA\r\ngamma\r\n"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_lone_cr_is_preserved() {
        // Classic Mac CR files are rare but exist; the writer must not
        // normalise them either.
        let path = create_temp_file("one\rtwo\rthree\r");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "two",
                    "replace": "TWO"
                }),
                default_context(),
            )
            .await;

        assert!(
            result.success,
            "lone-CR patch must apply: {:?}",
            result.error
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\rTWO\rthree\r");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_patch_delete_a_line_with_an_empty_replacement() {
        // D2: the dead branch that claimed to handle deletion. The behaviour it
        // pretended to have is what `replace.lines()` already did -- an empty
        // replacement yields no lines, so the matched block disappears. This is
        // the test that keeps the branch removed.
        let path = create_temp_file("keep\ndrop this\nkeep too\n");
        let path_str = path.to_str().unwrap().to_string();

        let tool = PatchTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "find": "drop this",
                    "replace": ""
                }),
                default_context(),
            )
            .await;

        assert!(
            result.success,
            "a delete-by-empty-replacement must apply: {:?}",
            result.error
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep\nkeep too\n");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dominant_line_ending_detects_the_files_own_style() {
        // The helper the writer uses, unit-tested directly.
        assert_eq!(dominant_line_ending("a\nb\n"), "\n");
        assert_eq!(dominant_line_ending("a\r\nb\r\n"), "\r\n");
        assert_eq!(dominant_line_ending("a\rb\r"), "\r");
        assert_eq!(dominant_line_ending("no ending at all"), "\n");
        // Mixed: the FIRST ending wins, matching the ported SourceFile rule.
        assert_eq!(dominant_line_ending("a\r\nb\n"), "\r\n");
    }
}
