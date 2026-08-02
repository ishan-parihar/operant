//! Hardware board info tool — returns chip name, architecture, memory map for Telegram/agent.
//!
//! Use when user asks "what board do I have?", "board info", "connected hardware", etc.
//! Returns static datasheet info (probe-rs live lookup is not wired into this
//! build — the `probe` feature was removed 2026-08-02).

use async_trait::async_trait;
use operant_api::tool::{Tool, ToolResult};
use serde_json::json;

/// Static board info (datasheets). Used when probe-rs is unavailable.
const BOARD_INFO: &[(&str, &str, &str)] = &[
    (
        "nucleo-f401re",
        "STM32F401RET6",
        "ARM Cortex-M4, 84 MHz. Flash: 512 KB, RAM: 128 KB. User LED on PA5 (pin 13).",
    ),
    (
        "nucleo-f411re",
        "STM32F411RET6",
        "ARM Cortex-M4, 100 MHz. Flash: 512 KB, RAM: 128 KB. User LED on PA5 (pin 13).",
    ),
    (
        "arduino-uno",
        "ATmega328P",
        "8-bit AVR, 16 MHz. Flash: 16 KB, SRAM: 2 KB. Built-in LED on pin 13.",
    ),
    (
        "arduino-uno-q",
        "STM32U585 + Qualcomm",
        "Dual-core: STM32 (MCU) + Linux (aarch64). GPIO via Bridge app on port 9999.",
    ),
    (
        "esp32",
        "ESP32",
        "Dual-core Xtensa LX6, 240 MHz. Flash: 4 MB typical. Built-in LED on GPIO 2.",
    ),
    (
        "rpi-gpio",
        "Raspberry Pi",
        "ARM Linux. Native GPIO via sysfs/rppal. No fixed LED pin.",
    ),
];

/// Tool: return full board info (chip, architecture, memory map) for agent/Telegram.
pub struct HardwareBoardInfoTool {
    boards: Vec<String>,
}

impl HardwareBoardInfoTool {
    pub fn new(boards: Vec<String>) -> Self {
        Self { boards }
    }

    fn static_info_for_board(&self, board: &str) -> Option<String> {
        BOARD_INFO
            .iter()
            .find(|(b, _, _)| *b == board)
            .map(|(_, chip, desc)| {
                format!(
                    "**Board:** {}\n**Chip:** {}\n**Description:** {}",
                    board, chip, desc
                )
            })
    }
}

#[async_trait]
impl Tool for HardwareBoardInfoTool {
    fn name(&self) -> &str {
        "hardware_board_info"
    }

    fn description(&self) -> &str {
        "Return full board info (chip, architecture, memory map) for connected hardware. Use when: user asks for 'board info', 'what board do I have', 'connected hardware', 'chip info', 'what hardware', or 'memory map'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "board": {
                    "type": "string",
                    "description": "Optional board name (e.g. nucleo-f401re). If omitted, returns info for first configured board."
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

        // probe-rs live chip info is not wired into this build (the `probe`
        // feature had never compiled — see Cargo.toml note); always use
        // static datasheet info.
        if let Some(info) = self.static_info_for_board(board) {
            output.push_str(&info);
            if let Some(mem) = memory_map_static(board) {
                use std::fmt::Write;
                let _ = write!(output, "\n\n**Memory map:**\n{mem}");
            }
        } else {
            use std::fmt::Write;
            let _ = write!(
                output,
                "Board '{board}' configured. No static info available."
            );
        }

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}

fn memory_map_static(board: &str) -> Option<&'static str> {
    match board {
        "nucleo-f401re" | "nucleo-f411re" => Some(
            "Flash: 0x0800_0000 - 0x0807_FFFF (512 KB)\nRAM: 0x2000_0000 - 0x2001_FFFF (128 KB)",
        ),
        "arduino-uno" => Some("Flash: 16 KB, SRAM: 2 KB, EEPROM: 1 KB"),
        "esp32" => Some("Flash: 4 MB, IRAM/DRAM per ESP-IDF layout"),
        _ => None,
    }
}
