//! Bridges agent streaming events to progressive platform message editing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::warn;

use super::{Gateway, OutgoingMessage};
use crate::error::Result;

const THINK_TAGS: &[&str] = &[
    "<think>", "</think>", "<reasoning>", "</reasoning>",
    "<THINKING>", "</THINKING>", "<thought>", "</thought>",
    "<REASONING_SCRATCHPAD>", "</REASONING_SCRATCHPAD>",
];

#[derive(Debug, Clone)]
pub enum StreamTransport {
    Auto,
    Edit,
    Draft,
    Off,
}

#[derive(Debug, Clone)]
pub struct StreamConsumerConfig {
    pub edit_interval: Duration,
    pub buffer_threshold: usize,
    pub cursor: String,
    pub fresh_final_after_seconds: f64,
    pub transport: StreamTransport,
}

impl Default for StreamConsumerConfig {
    fn default() -> Self {
        Self {
            edit_interval: Duration::from_millis(1500),
            buffer_threshold: 100,
            cursor: "▍".to_string(),
            fresh_final_after_seconds: 30.0,
            transport: StreamTransport::Auto,
        }
    }
}

#[derive(Debug)]
pub enum StreamEvent {
    Content(String),
    SegmentBreak,
    Done,
}

pub struct GatewayStreamConsumer {
    config: StreamConsumerConfig,
    gateway: Arc<Gateway>,
    platform: String,
    channel_id: String,
    rx: mpsc::Receiver<StreamEvent>,
    message_id: Option<String>,
    buffer: String,
    last_edit: Instant,
    flood_strikes: u8,
    first_edit_time: Option<Instant>,
}

impl GatewayStreamConsumer {
    pub fn new(
        config: StreamConsumerConfig,
        gateway: Arc<Gateway>,
        platform: String,
        channel_id: String,
        rx: mpsc::Receiver<StreamEvent>,
    ) -> Self {
        Self {
            config,
            gateway,
            platform,
            channel_id,
            rx,
            message_id: None,
            buffer: String::new(),
            last_edit: Instant::now() - Duration::from_secs(10),
            flood_strikes: 0,
            first_edit_time: None,
        }
    }

    pub async fn run(mut self) -> Result<()> {
        while let Some(event) = self.rx.recv().await {
            match event {
                StreamEvent::Content(text) => {
                    let filtered = filter_think_tags(&text);
                    if filtered.is_empty() {
                        continue;
                    }
                    self.buffer.push_str(&filtered);
                    self.maybe_edit().await;
                }
                StreamEvent::SegmentBreak => {
                    self.finalize_segment().await;
                }
                StreamEvent::Done => {
                    self.finalize_done().await;
                    break;
                }
            }
        }
        Ok(())
    }

    async fn maybe_edit(&mut self) {
        if self.flood_strikes >= 3 {
            return;
        }
        if matches!(self.config.transport, StreamTransport::Off) {
            return;
        }
        if self.last_edit.elapsed() < self.config.edit_interval {
            return;
        }
        let display = format!("{}{}", self.buffer, self.config.cursor);
        let msg = OutgoingMessage::new(&self.channel_id, &display);

        let result = if let Some(mid) = &self.message_id {
            self.gateway
                .edit_message(&self.platform, &self.channel_id, mid, msg)
                .await
        } else {
            self.gateway
                .send_message_return_id(&self.platform, msg)
                .await
        };

        match result {
            Ok(id) => {
                if self.message_id.is_none() {
                    self.message_id = Some(id);
                    self.first_edit_time = Some(Instant::now());
                }
                self.flood_strikes = 0;
                self.last_edit = Instant::now();
            }
            Err(e) => {
                self.flood_strikes += 1;
                warn!(strikes = self.flood_strikes, error = %e, "edit failed");
            }
        }
    }

    async fn finalize_segment(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        self.send_final().await;
        self.buffer.clear();
        self.message_id = None;
        self.first_edit_time = None;
        self.flood_strikes = 0;
    }

    async fn finalize_done(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        self.send_final().await;
    }

    async fn send_final(&mut self) {
        let fresh = self.should_send_fresh();
        let msg = OutgoingMessage::new(&self.channel_id, &self.buffer);

        if fresh || self.message_id.is_none() {
            let _ = self.gateway.send_message_return_id(&self.platform, msg).await;
        } else if let Some(mid) = &self.message_id {
            let _ = self
                .gateway
                .edit_message(&self.platform, &self.channel_id, mid, msg)
                .await;
        }
    }

    fn should_send_fresh(&self) -> bool {
        self.first_edit_time
            .map(|t| t.elapsed().as_secs_f64() > self.config.fresh_final_after_seconds)
            .unwrap_or(false)
    }
}

fn filter_think_tags(text: &str) -> String {
    let mut result = text.to_string();
    for tag in THINK_TAGS {
        result = result.replace(tag, "");
    }
    result
}
