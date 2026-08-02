//! Hardware memory map tool — returns flash/RAM address ranges for connected boards.
//!
//! Phase B: When user asks "what are the upper and lower memory addresses?", this tool
//! returns the memory map. Returns static maps from datasheets (probe-rs live
//! lookup is not wired into this build — the `probe` feature was removed 2026-08-02).

use async_trait::async_trait;
use operant_api::tool::{Tool, ToolResult};
use serde_json::json;

/// Known memory maps (from datasheets). Used when probe-rs is unavailable.
const MEMORY_MAPS: &[(&str, &str)] = &[
    (
        "nucleo-f401re",
        "Flash: 0x0800_0000 - 0x0807_FFFF (512 KB)\nRAM: 0x2000_0000 - 0x2001_FFFF (128 KB)\nSTM32F401RET6, ARM Cortex-M4",
    ),
    (
        "nucleo-f411re",
        "Flash: 0x0800_0000 - 0x0807_FFFF (512 KB)\nRAM: 0x2000_0000 - 0x2001_FFFF (128 KB)\nSTM32F411RET6, ARM Cortex-M4",
    ),
    (
        "arduino-uno",
        "Flash: 0x0000 - 0x3FFF (16 KB, ATmega328P)\nSRAM: 0x0100 - 0x08FF (2 KB)\nEEPROM: 0x0000 - 0x03FF (1 KB)",
    ),
    (
        "arduino-mega",
        "Flash: 0x0000 - 0x3FFFF (256 KB, ATmega2560)\nSRAM: 0x0200 - 0x21FF (8 KB)\nEEPROM: 0x0000 - 0x0FFF (4 KB)",
    ),
    (
        "esp32",
        "Flash: 0x3F40_0000 - 0x3F7F_FFFF (4 MB typical)\nIRAM: 0x4000_0000 - 0x4005_FFFF\nDRAM: 0x3FFB_0000 - 0x3FFF_FFFF",
    ),
];

/// Tool: report hardware memory map for connected boards.
pub struct HardwareMemoryMapTool {
    boards: Vec<String>,
}

impl HardwareMemoryMapTool {
    pub fn new(boards: Vec<String>) -> Self {
        Self { boards }
    }

    fn static_map_for_board(&self, board: &str) -> Option<&'static str> {
        MEMORY_MAPS
            .iter()
            .find(|(b, _)| *b == board)
            .map(|(_, m)| *m)
    }
}

#[async_trait]
impl Tool for HardwareMemoryMapTool {
    fn name(&self) -> &str {
        "hardware_memory_map"
    }

    fn description(&self) -> &str {
        "Return the memory map (flash and RAM address ranges) for connected hardware. Use when: user asks for 'upper and lower memory addresses', 'memory map', 'address space', or 'readable addresses'. Returns flash/RAM ranges from datasheets."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "board": {
                    "type": "string",
                    "description": "Optional board name (e.g. nucleo-f401re, arduino-uno). If omitted, returns map for first configured board."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let board = args
            .get("board")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| self.boards.first().cloned());

        let board = board.as_deref().unwrap_or("unknown");

        if self.boards.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    "No peripherals configured. Add boards to config.toml [peripherals.boards]."
                        .into(),
                ),
            });
        }

        let mut output = String::new();

        // Live probe-rs memory map lookup is not wired into this build (the
        // `probe` feature had never compiled — see Cargo.toml note).
        // Fall back to datasheet maps.
        if let Some(map) = self.static_map_for_board(board) {
            use std::fmt::Write;
            let _ = write!(output, "**{board}** (from datasheet):\n{map}");
        } else {
            use std::fmt::Write;
            let known: Vec<&str> = MEMORY_MAPS.iter().map(|(b, _)| *b).collect();
            let _ = write!(
                output,
                "No memory map for board '{board}'. Known boards: {}",
                known.join(", ")
            );
        }

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_map_nucleo() {
        let tool = HardwareMemoryMapTool::new(vec!["nucleo-f401re".into()]);
        assert!(tool.static_map_for_board("nucleo-f401re").is_some());
        assert!(
            tool.static_map_for_board("nucleo-f401re")
                .unwrap()
                .contains("Flash")
        );
    }

    #[test]
    fn static_map_arduino() {
        let tool = HardwareMemoryMapTool::new(vec!["arduino-uno".into()]);
        assert!(tool.static_map_for_board("arduino-uno").is_some());
    }
}
