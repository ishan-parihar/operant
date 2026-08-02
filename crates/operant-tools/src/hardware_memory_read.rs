//! Hardware memory read tool — read actual memory/register values from Nucleo via probe-rs.
//!
//! Use when user asks to "read register values", "read memory at address", "dump lower memory", etc.
//! NOTE: probe-rs is not wired into this build (the `probe` feature was removed
//! 2026-08-02 — it had never compiled); the tool currently returns a clear
//! error explaining that. Wire probe-rs back up to restore live reads.

use async_trait::async_trait;
use operant_api::tool::{Tool, ToolResult};
use serde_json::json;

/// Tool: read memory at address from connected Nucleo via probe-rs.
pub struct HardwareMemoryReadTool {
    boards: Vec<String>,
}

impl HardwareMemoryReadTool {
    pub fn new(boards: Vec<String>) -> Self {
        Self { boards }
    }

    fn chip_for_board(board: &str) -> Option<&'static str> {
        match board {
            "nucleo-f401re" => Some("STM32F401RETx"),
            "nucleo-f411re" => Some("STM32F411RETx"),
            _ => None,
        }
    }
}

#[async_trait]
impl Tool for HardwareMemoryReadTool {
    fn name(&self) -> &str {
        "hardware_memory_read"
    }

    fn description(&self) -> &str {
        "Read actual memory/register values from Nucleo via USB. Use when: user asks to 'read register values', 'read memory at address', 'dump memory', 'lower memory 0-126', or 'give address and value'. Returns hex dump. Requires Nucleo connected via USB and probe feature. Params: address (hex, e.g. 0x20000000 for RAM start), length (bytes, default 128)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "address": {
                    "type": "string",
                    "description": "Memory address in hex (e.g. 0x20000000 for RAM start). Default: 0x20000000 (RAM base)."
                },
                "length": {
                    "type": "integer",
                    "description": "Number of bytes to read (default 128, max 256)."
                },
                "board": {
                    "type": "string",
                    "description": "Board name (nucleo-f401re). Optional if only one configured."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if self.boards.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    "No peripherals configured. Add nucleo-f401re to config.toml [peripherals.boards]."
                        .into(),
                ),
            });
        }

        let board = args
            .get("board")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| self.boards.first().cloned())
            .unwrap_or_else(|| "nucleo-f401re".into());

        let chip = Self::chip_for_board(&board);
        if chip.is_none() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Memory read only supports nucleo-f401re, nucleo-f411re. Got: {}",
                    board
                )),
            });
        }

        // probe-rs live memory reads are not wired into this build (the
        // `probe` feature had never compiled — see Cargo.toml note).
        // Return a clear error rather than silently returning nothing.
        Ok(ToolResult {
            success: false,
            output: String::new(),
            error: Some(
                "Memory read requires probe-rs (SWD/JTAG) support, which is not wired into this build (see docs/RUST_BEST_PRACTICES_PLAN.md)."
                    .into(),
            ),
        })
    }
}
