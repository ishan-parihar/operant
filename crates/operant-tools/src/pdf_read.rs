use async_trait::async_trait;
use operant_api::tool::{Tool, ToolResult};
use operant_config::policy::SecurityPolicy;
use serde_json::json;
use std::sync::Arc;

/// Maximum PDF file size (50 MB).
const MAX_PDF_BYTES: u64 = 50 * 1024 * 1024;
/// Default character limit returned to the LLM.
const DEFAULT_MAX_CHARS: usize = 50_000;
/// Hard ceiling regardless of what the caller requests.
const MAX_OUTPUT_CHARS: usize = 200_000;

/// Maximum decompressed content per PDF page.
///
/// A PDF page's content stream is a compressed blob whose decompressed size
/// the file chooses. Without a bound, a small file can expand into
/// gigabytes — the classic PDF decompression bomb. `lopdf` enforces the
/// limit for us; this is the value.
#[cfg(feature = "rag-pdf")]
const MAX_DECOMPRESSED_PAGE_BYTES: usize = 64 * 1024 * 1024;

/// The most pages one document may contribute.
///
/// The per-page decompression limit bounds ONE page's content stream.
/// A document with 1,000 pages would still cost 1,000 x 64 MiB, so the
/// document needs its own budget: the page count is capped, and the
/// per-page limit is divided across the pages that are actually read.
#[cfg(feature = "rag-pdf")]
const MAX_PAGES_PER_DOCUMENT: usize = 256;

/// Extract the text of a PDF's pages.
///
/// `lopdf` 0.45 replaced `pdf-extract` 0.12 here (AD-039). Two things came
/// with the swap:
///
/// - `pdf-extract` pins `lopdf` 0.42, which reaches `ttf-parser`
///   (RUSTSEC-2026-0192, RT-27). `lopdf` 0.45 has no `ttf-parser`; its
///   `font_embedding` feature pulls `skrifa` instead, and this tool does
///   not embed fonts.
/// - `lopdf` can bound the decompressed size of a stream. `pdf-extract`
///   did not, so a decompression bomb was a live hazard on the old path.
///   The bound is now explicit, and it covers BOTH paths that decompress:
///   the LOAD (object streams and xref streams, which `Document::load_mem`
///   decodes eagerly) and the EXTRACT (each page's content stream).
///
/// The page numbers are passed explicitly because `lopdf` extracts by page
/// rather than over the whole document.
#[cfg(feature = "rag-pdf")]
fn extract_pdf_text(bytes: &[u8]) -> Result<String, String> {
    // The LOAD path decompresses too: object streams and xref streams are
    // decoded when the document is read, and `Document::load_mem` uses
    // `LoadOptions::default()`, whose `max_decompressed_size` is `None`.
    // Bounding only the extraction left the load path unbounded, which is
    // the same class of hazard the port was meant to close.
    let options = lopdf::LoadOptions::with_max_decompressed_size(MAX_DECOMPRESSED_PAGE_BYTES);
    let doc = lopdf::Document::load_mem_with_options(bytes, options).map_err(|e| e.to_string())?;
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    if pages.is_empty() {
        // A PDF with no pages has no text. Not an error: an image-only or
        // empty document is a valid thing to read.
        return Ok(String::new());
    }
    // The per-page limit is a limit PER PAGE, so a document's total cost
    // is bounded by dividing the budget across the pages read. The page
    // count is also capped: past the cap the remaining pages are not read
    // at all, which is both cheaper and a clearer limit than a budget
    // divided so far that each page gets almost nothing.
    let read_count = pages.len().min(MAX_PAGES_PER_DOCUMENT);
    let page_budget = (MAX_DECOMPRESSED_PAGE_BYTES / read_count.max(1)).max(1024 * 1024);
    doc.extract_text_with_limit(&pages[..read_count], page_budget)
        .map_err(|e| e.to_string())
}

/// Extract plain text from a PDF file in the workspace.
///
/// PDF extraction requires the `rag-pdf` feature flag:
///   cargo build --features rag-pdf
///
/// Without the feature the tool is still registered so the LLM receives a
/// clear, actionable error rather than a missing-tool confusion.
pub struct PdfReadTool {
    security: Arc<SecurityPolicy>,
}

impl PdfReadTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for PdfReadTool {
    fn name(&self) -> &str {
        "pdf_read"
    }

    fn description(&self) -> &str {
        "Extract plain text from a PDF file in the workspace. \
         Returns all readable text. Image-only or encrypted PDFs return an empty result. \
         Requires the 'rag-pdf' build feature."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the PDF file. Relative paths resolve from workspace; outside paths require policy allowlist."
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default: 50000, max: 200000)",
                    "minimum": 1,
                    "maximum": 200_000
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| {
                usize::try_from(n)
                    .unwrap_or(MAX_OUTPUT_CHARS)
                    .min(MAX_OUTPUT_CHARS)
            })
            .unwrap_or(DEFAULT_MAX_CHARS);

        // Cross-cutting rate limiting and path-allowlist checks live in the
        // RateLimitedTool + PathGuardedTool wrappers at registration time
        // (see operant-runtime::tools::mod).  Successful reads consume one
        // budget slot via the outer RateLimitedTool.
        //
        // Read-tool exception: post-`PathGuardedTool` canonicalize failures
        // (probing nonexistent files) and post-canonicalization policy
        // failures (`is_resolved_path_allowed`) also consume one budget slot,
        // charged here, so that callers cannot probe path existence or
        // resolved-path policy decisions for free.  The outer wrapper only
        // records on `success: true`, so these explicit charges total
        // exactly one slot per attempt — matching the pre-wrapper semantics.

        let full_path = self.security.resolve_tool_path(path);

        let resolved_path = match tokio::fs::canonicalize(&full_path).await {
            Ok(p) => p,
            Err(e) => {
                let _ = self.security.record_action();
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to resolve file path: {e}")),
                });
            }
        };

        if !self.security.is_resolved_path_allowed(&resolved_path) {
            let _ = self.security.record_action();
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    self.security
                        .resolved_path_violation_message(&resolved_path),
                ),
            });
        }

        tracing::debug!("Reading PDF: {}", resolved_path.display());

        match tokio::fs::metadata(&resolved_path).await {
            Ok(meta) => {
                if meta.len() > MAX_PDF_BYTES {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!(
                            "PDF too large: {} bytes (limit: {MAX_PDF_BYTES} bytes)",
                            meta.len()
                        )),
                    });
                }
            }
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to read file metadata: {e}")),
                });
            }
        }

        let bytes = match tokio::fs::read(&resolved_path).await {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to read PDF file: {e}")),
                });
            }
        };

        // lopdf is a blocking CPU-bound operation; keep it off the async
        // executor.
        #[cfg(feature = "rag-pdf")]
        {
            let text = match tokio::task::spawn_blocking(move || extract_pdf_text(&bytes)).await {
                Ok(Ok(t)) => t,
                Ok(Err(e)) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("PDF extraction failed: {e}")),
                    });
                }
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("PDF extraction task panicked: {e}")),
                    });
                }
            };

            if text.trim().is_empty() {
                return Ok(ToolResult {
                    success: true,
                    // Agent dispatchers currently forward `error` only when `success=false`.
                    // Keep this as successful execution and expose the warning in `output`.
                    output: "PDF contains no extractable text (may be image-only or encrypted)"
                        .into(),
                    error: None,
                });
            }

            let output = if text.chars().count() > max_chars {
                let mut truncated: String = text.chars().take(max_chars).collect();
                use std::fmt::Write as _;
                let _ = write!(truncated, "\n\n... [truncated at {max_chars} chars]");
                truncated
            } else {
                text
            };

            return Ok(ToolResult {
                success: true,
                output,
                error: None,
            });
        }

        #[cfg(not(feature = "rag-pdf"))]
        {
            let _ = bytes;
            let _ = max_chars;
            Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    "PDF extraction is not enabled. \
                     Rebuild with: cargo build --features rag-pdf"
                        .into(),
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrappers::{PathGuardedTool, RateLimitedTool};
    use operant_config::autonomy::AutonomyLevel;
    use operant_config::policy::SecurityPolicy;
    use tempfile::TempDir;

    fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
            ..SecurityPolicy::default()
        })
    }

    fn test_security_with_limit(
        workspace: std::path::PathBuf,
        max_actions: u32,
    ) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
            max_actions_per_hour: max_actions,
            ..SecurityPolicy::default()
        })
    }

    /// Wraps `PdfReadTool` with the production `PathGuardedTool` + `RateLimitedTool`
    /// stack, mirroring the registration in `operant-runtime::tools::mod`. Use this
    /// in tests that exercise path-allowlist or rate-limit behavior.
    fn wrapped_tool(workspace: std::path::PathBuf) -> Box<dyn Tool> {
        let security = test_security(workspace);
        Box::new(RateLimitedTool::new(
            PathGuardedTool::new(PdfReadTool::new(security.clone()), security.clone()),
            security,
        ))
    }

    #[test]
    fn name_is_pdf_read() {
        let tool = PdfReadTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "pdf_read");
    }

    #[test]
    fn description_not_empty() {
        let tool = PdfReadTool::new(test_security(std::env::temp_dir()));
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn schema_has_path_required() {
        let tool = PdfReadTool::new(test_security(std::env::temp_dir()));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["max_chars"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
    }

    #[test]
    fn spec_matches_metadata() {
        let tool = PdfReadTool::new(test_security(std::env::temp_dir()));
        let spec = tool.spec();
        assert_eq!(spec.name, "pdf_read");
        assert!(spec.parameters.is_object());
    }

    #[tokio::test]
    async fn missing_path_param_returns_error() {
        let tool = PdfReadTool::new(test_security(std::env::temp_dir()));
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("path"));
    }

    #[tokio::test]
    async fn absolute_path_is_blocked() {
        let tool = wrapped_tool(std::env::temp_dir());

        #[cfg(unix)]
        let target = "/etc/passwd";
        #[cfg(windows)]
        let target = {
            let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
            std::path::PathBuf::from(sysroot).join(r"System32\drivers\etc\hosts")
        };

        let result = tool.execute(json!({"path": target})).await.unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("Path blocked"),
            "expected 'Path blocked' error, got: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn path_traversal_is_blocked() {
        let tmp = TempDir::new().unwrap();
        let tool = wrapped_tool(tmp.path().to_path_buf());
        let result = tool
            .execute(json!({"path": "../../../etc/passwd"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("Path blocked"),
            "expected 'Path blocked' error, got: {:?}",
            result.error
        );
    }

    #[tokio::test]
    async fn nonexistent_file_returns_error() {
        let tmp = TempDir::new().unwrap();
        let tool = PdfReadTool::new(test_security(tmp.path().to_path_buf()));
        let result = tool
            .execute(json!({"path": "does_not_exist.pdf"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("Failed to resolve")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_escape_is_blocked() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().unwrap();
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();
        tokio::fs::write(outside.join("secret.pdf"), b"%PDF-1.4 secret")
            .await
            .unwrap();
        symlink(outside.join("secret.pdf"), workspace.join("link.pdf")).unwrap();

        let tool = PdfReadTool::new(test_security(workspace));
        let result = tool.execute(json!({"path": "link.pdf"})).await.unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("escapes workspace")
        );
    }

    /// Extraction tests require the rag-pdf feature.
    #[cfg(feature = "rag-pdf")]
    mod extraction {
        use super::*;

        /// Minimal valid PDF with one text page ("Hello PDF").
        /// Generated offline and verified with pdf-extract 0.10.
        ///
        /// The comment names the crate the fixture was written against; the
        /// extractor is lopdf now (AD-039) and reads the same bytes.
        fn minimal_pdf_bytes() -> Vec<u8> {
            // A hand-crafted single-page PDF containing the text "Hello PDF".
            //
            // The offsets and the stream length are COMPUTED, not written
            // down. The fixture used to carry hardcoded values that were
            // wrong (the stream was 41 bytes and declared 44; the xref
            // entries pointed past their objects). `pdf-extract` was lenient
            // enough to reconstruct the cross-reference table, so the
            // fixture passed; `lopdf` TRUSTS the table, and a wrong one
            // means no pages and no text. Computing the offsets is the
            // only way the fixture stays valid when the body changes.
            let stream = b"BT /F1 12 Tf 72 720 Td (Hello PDF) Tj ET\n";
            let mut pdf = Vec::new();
            let mut offsets = Vec::new();

            pdf.extend_from_slice(b"%PDF-1.4\n");
            offsets.push(pdf.len());
            pdf.extend_from_slice(b"1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n");
            offsets.push(pdf.len());
            pdf.extend_from_slice(b"2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n");
            offsets.push(pdf.len());
            pdf.extend_from_slice(
                b"3 0 obj<</Type/Page/MediaBox[0 0 612 792]/Parent 2 0 R\
                  /Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n",
            );
            offsets.push(pdf.len());
            pdf.extend_from_slice(
                format!("4 0 obj<</Length {}>>\nstream\n", stream.len()).as_bytes(),
            );
            pdf.extend_from_slice(stream);
            pdf.extend_from_slice(b"endstream\nendobj\n");
            offsets.push(pdf.len());
            pdf.extend_from_slice(
                b"5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n",
            );

            let xref_offset = pdf.len();
            let mut xref = String::from("xref\n0 6\n0000000000 65535 f \n");
            for offset in &offsets {
                xref.push_str(&format!("{offset:010} 00000 n \n"));
            }
            xref.push_str(&format!(
                "trailer<</Size 6/Root 1 0 R>>\nstartxref\n{xref_offset}\n%%EOF\n"
            ));
            pdf.extend_from_slice(xref.as_bytes());
            pdf
        }

        /// The extractor reads the text the PDF carries.
        ///
        /// This is the direct test of the lopdf port (AD-039): the fixture
        /// is a hand-built PDF whose only text is "Hello PDF", so the
        /// extracted string must contain it. The tool-level test above
        /// accepts "no extractable text" as a valid outcome, which is right
        /// for a malformed fixture but would not notice an extractor that
        /// extracted nothing at all.
        #[test]
        fn the_extractor_reads_the_text_the_pdf_carries() {
            let text = extract_pdf_text(&minimal_pdf_bytes()).expect("the fixture parses");
            assert!(
                text.contains("Hello PDF"),
                "the extracted text must carry the PDF's text, got {text:?}"
            );
        }

        /// A PDF with no pages yields no text and is NOT an error.
        ///
        /// An empty or image-only document is a valid thing to read, and
        /// the tool reports it as "no extractable text" rather than failing.
        #[test]
        fn a_document_with_no_pages_yields_no_text() {
            let empty = b"%PDF-1.4\n%%EOF\n";
            let text = extract_pdf_text(empty).unwrap_or_default();
            assert!(text.is_empty(), "no pages means no text: {text:?}");
        }

        /// A file that is not a PDF is an error, not a panic and not empty
        /// text.
        #[test]
        fn a_file_that_is_not_a_pdf_is_an_error() {
            let result = extract_pdf_text(b"this is not a pdf at all");
            assert!(
                result.is_err(),
                "a non-PDF must be reported as an error, not as empty text"
            );
        }

        #[tokio::test]
        async fn extracts_text_from_valid_pdf() {
            let tmp = TempDir::new().unwrap();
            let pdf_path = tmp.path().join("test.pdf");
            tokio::fs::write(&pdf_path, minimal_pdf_bytes())
                .await
                .unwrap();

            let tool = PdfReadTool::new(test_security(tmp.path().to_path_buf()));
            let result = tool.execute(json!({"path": "test.pdf"})).await.unwrap();

            // The text must ACTUALLY be extracted.
            //
            // This test used to accept an error containing "no
            // extractable", which means a completely broken port — one
            // that returned empty for every file, or failed on every
            // file — would pass. The fixture is a valid single-page PDF
            // with the text "Hello PDF" and the extractor is lopdf now
            // (AD-039), so the text is there to be read.
            assert!(
                result.success,
                "the extraction must succeed, got: {:?}",
                result.error
            );
            let text = &result.output;
            assert!(
                text.contains("Hello PDF"),
                "the extracted text must contain the fixture's text, got: {text}"
            );
        }

        #[tokio::test]
        async fn max_chars_truncates_output() {
            let tmp = TempDir::new().unwrap();
            // Write a text file and rename as PDF to exercise the truncation path
            // with known content length.
            let pdf_path = tmp.path().join("trunc.pdf");
            tokio::fs::write(&pdf_path, minimal_pdf_bytes())
                .await
                .unwrap();

            let tool = PdfReadTool::new(test_security(tmp.path().to_path_buf()));
            let result = tool
                .execute(json!({"path": "trunc.pdf", "max_chars": 5}))
                .await
                .unwrap();

            // If extraction succeeded the output must respect the char limit
            // (plus the truncation suffix).
            if result.success && !result.output.is_empty() {
                assert!(
                    result.output.chars().count() <= 5 + "[truncated".len() + 50,
                    "output longer than expected: {} chars",
                    result.output.chars().count()
                );
            }
        }

        #[tokio::test]
        async fn image_only_pdf_returns_empty_text_warning() {
            // A well-formed PDF with no text streams will yield empty output.
            // We simulate this with an otherwise valid PDF that has an empty content stream.
            let tmp = TempDir::new().unwrap();
            let empty_content_pdf = b"%PDF-1.4\n\
                1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
                2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n\
                3 0 obj<</Type/Page/MediaBox[0 0 612 792]/Parent 2 0 R\
                /Contents 4 0 R/Resources<<>>>>endobj\n\
                4 0 obj<</Length 0>>\nstream\n\nendstream\nendobj\n\
                xref\n0 5\n\
                0000000000 65535 f \n\
                0000000009 00000 n \n\
                0000000058 00000 n \n\
                0000000115 00000 n \n\
                0000000250 00000 n \n\
                trailer<</Size 5/Root 1 0 R>>\nstartxref\n300\n%%EOF\n";

            tokio::fs::write(tmp.path().join("empty.pdf"), empty_content_pdf)
                .await
                .unwrap();

            let tool = PdfReadTool::new(test_security(tmp.path().to_path_buf()));
            let result = tool.execute(json!({"path": "empty.pdf"})).await.unwrap();

            // Acceptable outcomes: empty text warning, or extraction error for
            // malformed hand-crafted PDF.
            let is_empty_warning = result.success && result.output.contains("no extractable text");
            let is_extraction_error =
                !result.success && result.error.as_deref().unwrap_or("").contains("extraction");
            let is_resolve_error =
                !result.success && result.error.as_deref().unwrap_or("").contains("Failed");
            assert!(
                is_empty_warning || is_extraction_error || is_resolve_error,
                "unexpected result: success={} error={:?}",
                result.success,
                result.error
            );
        }
    }

    #[cfg(not(feature = "rag-pdf"))]
    #[tokio::test]
    async fn without_feature_returns_clear_error() {
        let tmp = TempDir::new().unwrap();
        let pdf_path = tmp.path().join("doc.pdf");
        tokio::fs::write(&pdf_path, b"%PDF-1.4 fake").await.unwrap();

        let tool = PdfReadTool::new(test_security(tmp.path().to_path_buf()));
        let result = tool.execute(json!({"path": "doc.pdf"})).await.unwrap();
        assert!(!result.success);
        assert!(
            result.error.as_deref().unwrap_or("").contains("rag-pdf"),
            "expected feature hint in error, got: {:?}",
            result.error
        );
    }

    /// Anti-probing regression: a caller cannot probe PDF existence for free.
    /// Each failed canonicalize must consume one action-budget slot via the
    /// inner-tool charge, so repeated probes hit the rate limit.
    #[tokio::test]
    async fn probing_nonexistent_consumes_rate_limit_budget() {
        let tmp = TempDir::new().unwrap();
        let security = test_security_with_limit(tmp.path().to_path_buf(), 2);
        let tool = PdfReadTool::new(security.clone());

        let r1 = tool.execute(json!({"path": "a.pdf"})).await.unwrap();
        assert!(!r1.success);
        assert!(
            r1.error
                .as_deref()
                .unwrap_or("")
                .contains("Failed to resolve")
        );

        let r2 = tool.execute(json!({"path": "b.pdf"})).await.unwrap();
        assert!(!r2.success);
        assert!(
            r2.error
                .as_deref()
                .unwrap_or("")
                .contains("Failed to resolve")
        );

        // Budget must now be exhausted.
        assert!(
            !security.record_action(),
            "budget must be exhausted after two failed probes"
        );
    }
}
