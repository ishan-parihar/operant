//! Binary file extension detection tool
//!
//! Checks file extensions against a known list of binary format extensions.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Known binary file extensions
const BINARY_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".svg", ".pdf", ".doc", ".docx", ".xls",
    ".xlsx", ".ppt", ".pptx", ".zip", ".gz", ".bz2", ".xz", ".7z", ".rar", ".tar", ".exe", ".dll",
    ".so", ".dylib", ".bin", ".dat", ".class", ".pyc", ".o", ".a", ".lib", ".pdb", ".mp3", ".mp4",
    ".avi", ".mov", ".wav", ".flac", ".ogg", ".webm", ".mkv", ".woff", ".woff2", ".ttf", ".otf",
    ".eot", ".ico", ".icns", ".iso", ".img", ".dmg", ".db", ".sqlite", ".sqlite3", ".deb", ".rpm",
    ".apk", ".ipa",
];

/// Tool for checking whether a file path points to a binary file
pub struct BinaryExtensionsTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinaryExtensionsArgs {
    /// File path to check
    path: String,
}

#[async_trait]
impl OperantTool for BinaryExtensionsTool {
    fn name(&self) -> &str {
        "check_binary_file"
    }

    fn description(&self) -> &str {
        "Check whether a file is a binary file based on its extension. \
         Returns whether the file extension matches known binary formats \
         such as images, audio, video, archives, executables, and other non-text formats."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BinaryExtensionsArgs>(
            "check_binary_file",
            "Check if a file is binary by extension",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: BinaryExtensionsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("check_binary_file", format!("Invalid arguments: {}", e))
            }
        };

        let path = Path::new(&args.path);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_lowercase()))
            .unwrap_or_default();

        let is_binary = !extension.is_empty() && BINARY_EXTENSIONS.contains(&extension.as_str());

        ToolResult::success(
            "check_binary_file",
            serde_json::json!({
                "is_binary": is_binary,
                "extension": extension,
                "path": args.path
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_binary_extensions_png() {
        let tool = BinaryExtensionsTool;
        let args = serde_json::json!({ "path": "image.png" });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["is_binary"], true);
        assert_eq!(v["extension"], ".png");
    }

    #[tokio::test]
    async fn test_binary_extensions_txt() {
        let tool = BinaryExtensionsTool;
        let args = serde_json::json!({ "path": "readme.txt" });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["is_binary"], false);
        assert_eq!(v["extension"], ".txt");
    }

    #[tokio::test]
    async fn test_binary_extensions_no_ext() {
        let tool = BinaryExtensionsTool;
        let args = serde_json::json!({ "path": "Makefile" });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["is_binary"], false);
        assert_eq!(v["extension"], "");
    }
}
