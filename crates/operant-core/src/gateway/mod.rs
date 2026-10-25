//! Multi-platform gateway for Operant-RS
//!
//! Provides unified messaging interface across multiple platforms including
//! Telegram, Discord, Slack, WhatsApp, and more.

pub mod dispatch_breaker;
pub mod lifecycle;

/// The platforms whose inbound messages actually reach the agent.
///
/// A platform can be in the gateway's registry and still be OUTBOUND-ONLY:
/// the adapter has a complete inbound parser, but nothing ever calls it.
///
/// The `PlatformAdapter::handle_update` trait method is DEAD — it has no
/// production caller for ANY adapter. The platforms that receive use their
/// own path instead: Telegram's and Discord's poll loops and the webhook
/// adapter's own HTTP handler. The adapters that do NOT — Slack,
/// WhatsApp, Email and SMS — have an inbound parser that nothing reaches,
/// so a message sent to them is never seen by the agent.
///
/// This table is the single source of that truth, so the adapters' status
/// reports and the `operant gateway status` command cannot disagree.
pub const INBOUND_CAPABLE_PLATFORMS: &[&str] = &["telegram", "discord", "webhook"];

/// Whether a platform's inbound messages reach the agent.
///
/// The name is matched case-insensitively, and an unknown platform is
/// treated as OUTBOUND-ONLY: the safe direction, because claiming a
/// platform can receive when it cannot is the failure that misleads the
/// operator.
pub fn platform_can_receive(platform: &str) -> bool {
    INBOUND_CAPABLE_PLATFORMS
        .iter()
        .any(|p| p.eq_ignore_ascii_case(platform))
}

use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::gateway::dispatch_breaker::DispatchBreaker;
use crate::gateway::lifecycle::{DeliveryLedger, MirrorRule, SessionStallTracker, TurnLease};
use crate::gateway_session::{PersistentSessionStore, SessionSource};

// Platform adapters extracted verbatim from this file (dedup pass 6).
mod admin;
mod discord;
mod email;
mod slack;
mod sms;
mod telegram;
mod types;
// (in-flight completion by the concurrent agent — the peer's phase-F
// gateway_runner.rs builds a HolonHook on the CLI side, so the module
// must be public).
pub mod webhook;
pub mod webhook_subscriptions;
mod whatsapp;
pub use admin::*;
pub use discord::*;
pub use email::*;
pub use slack::*;
pub use sms::*;
pub use telegram::*;
pub use types::*;
pub use webhook::*;
pub use whatsapp::*;

/// Gateway for routing messages between platforms and the agent
pub struct Gateway {
    config: GatewayConfig,
    adapters: HashMap<String, Arc<dyn PlatformAdapter>>,
    message_handler: Option<Arc<dyn MessageHandler>>,
    running: Arc<RwLock<bool>>,
    session_store: SessionStore,
    persistent_sessions: Option<Arc<PersistentSessionStore>>,
    channel_directory: ChannelDirectory,
    start_time: Instant,
    start_time_formatted: String,
    messages_processed: Arc<AtomicU64>,
    /// Per-session turn lease (hermes `gateway/turn_lease.py` parity):
    /// only one in-flight agent turn per session key.
    turn_lease: TurnLease,
    /// Last-activity tracker for stall detection (hermes
    /// `gateway/session_stall.py` parity).
    stall_tracker: SessionStallTracker,
    /// Bounded record of outbound deliveries (hermes
    /// `gateway/delivery_ledger.py` parity).
    delivery_ledger: DeliveryLedger,
    /// Response mirror rules (hermes `gateway/mirror.py` parity).
    mirror_rules: Vec<MirrorRule>,
    /// The per-channel dispatch breaker (v6 outline §2 P1-A). `None`
    /// until mounted with [`Self::with_dispatch_breaker`] — a gateway
    /// without one behaves exactly as before every send (the hook is a
    /// no-op), so tests and kernel-only builds stay untouched.
    dispatch_breaker: Option<Arc<DispatchBreaker>>,
}

/// Handler for incoming messages from any platform
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync {
    /// Handle an incoming message
    async fn handle(&self, message: IncomingMessage) -> Result<OutgoingMessage>;

    /// Record a feed item (channel/group post captured at route time,
    /// plan-2026-10-09 §3). No-op default: handlers without a context
    /// injector (tests, harnesses) stay untouched — the feed branch in
    /// `route_message` calls this BEFORE the admin gate and never depends
    /// on the reply. `handle`'s contract is deliberately NOT widened.
    fn record_feed(&self, _message: &IncomingMessage) {}
}

impl Gateway {
    /// Create a new gateway with the given configuration
    pub fn new(config: GatewayConfig) -> Self {
        Self {
            config,
            adapters: HashMap::new(),
            message_handler: None,
            running: Arc::new(RwLock::new(false)),
            session_store: SessionStore::new(),
            persistent_sessions: None,
            channel_directory: ChannelDirectory::new(),
            start_time: Instant::now(),
            start_time_formatted: Utc::now().to_rfc3339(),
            messages_processed: Arc::new(AtomicU64::new(0)),
            turn_lease: TurnLease::new(),
            stall_tracker: SessionStallTracker::new(),
            delivery_ledger: DeliveryLedger::default(),
            mirror_rules: Vec::new(),
            dispatch_breaker: None,
        }
    }

    /// Register a platform adapter
    pub fn with_adapter(mut self, adapter: Arc<dyn PlatformAdapter>) -> Self {
        let name = adapter.name().to_string();
        info!(platform = %name, "Registering platform adapter");
        self.adapters.insert(name, adapter);
        self
    }

    /// Set the message handler
    pub fn with_handler(mut self, handler: Arc<dyn MessageHandler>) -> Self {
        self.message_handler = Some(handler);
        self
    }

    /// Attach a persistent session store for cross-restart session tracking.
    pub fn with_persistent_sessions(mut self, store: Arc<PersistentSessionStore>) -> Self {
        self.persistent_sessions = Some(store);
        self
    }

    /// Add a response mirror rule (hermes `gateway/mirror.py` parity).
    pub fn with_mirror_rule(mut self, rule: MirrorRule) -> Self {
        self.mirror_rules.push(rule);
        self
    }

    /// Register a mirror rule in place.
    pub fn add_mirror_rule(&mut self, rule: MirrorRule) {
        self.mirror_rules.push(rule);
    }

    /// Mount the per-channel dispatch breaker (v6 outline §2 P1-A).
    /// Without it every send passes through untouched — the lookup in the
    /// seam is a `None` check, so unmounted gateways (tests, harnesses)
    /// keep the pre-breaker behavior byte-for-byte.
    pub fn with_dispatch_breaker(mut self, breaker: Arc<DispatchBreaker>) -> Self {
        self.dispatch_breaker = Some(breaker);
        self
    }

    /// The seam hook (v6 outline §2 P1-A). Class-free pre-check: an OPEN
    /// channel+any-class row fast-fails the send. Classed post-record:
    /// success resets the channel (all classes), a failure increments the
    /// class the error fell into. Every outcome is best-effort — a store
    /// error logs and the send proceeds (fail-open on infra down), so the
    /// breaker can never itself take delivery down.
    async fn breaker_guard(
        &self,
        platform: &str,
        message: &OutgoingMessage,
    ) -> Option<crate::error::Error> {
        let breaker = self.dispatch_breaker.as_ref()?;
        match breaker.check(platform, &message.channel_id) {
            crate::gateway::dispatch_breaker::DispatchVerdict::Reject => {
                tracing::info!(
                    platform,
                    channel_id = %message.channel_id,
                    "dispatch breaker OPEN for this channel — send skipped (logged once per open, not per cycle)"
                );
                return Some(crate::error::Error::Agent(format!(
                    "dispatch breaker open for {} (channel {}); send skipped",
                    platform, message.channel_id
                )));
            }
            crate::gateway::dispatch_breaker::DispatchVerdict::Probe => {
                tracing::info!(
                    platform,
                    channel_id = %message.channel_id,
                    "dispatch breaker HALF-OPEN — this send is the single probe"
                );
            }
            crate::gateway::dispatch_breaker::DispatchVerdict::Allow => {}
        }
        None
    }

    async fn breaker_record(&self, platform: &str, channel_id: &str, outcome: Option<&Error>) {
        let Some(breaker) = self.dispatch_breaker.as_ref() else {
            return;
        };
        match outcome {
            None => {
                if let Err(e) = breaker.record_success(platform, channel_id) {
                    tracing::warn!(error = %e, "dispatch breaker success record failed (fail-open)");
                }
            }
            Some(e) => {
                let rendered = e.to_string();
                let class = crate::gateway::dispatch_breaker::classify_error(&rendered);
                if let Err(e2) = breaker.record_failure(platform, channel_id, class, &rendered) {
                    tracing::warn!(error = %e2, "dispatch breaker failure record failed (fail-open)");
                }
            }
        }
    }

    /// Configure the delivery ledger capacity (default 500).
    pub fn with_delivery_ledger_capacity(mut self, max: usize) -> Self {
        self.delivery_ledger = DeliveryLedger::new(max);
        self
    }

    /// Access the per-session turn lease.
    pub fn turn_lease(&self) -> &TurnLease {
        &self.turn_lease
    }

    /// Access the session stall tracker.
    pub fn stall_tracker(&self) -> &SessionStallTracker {
        &self.stall_tracker
    }

    /// Access the delivery ledger.
    pub fn delivery_ledger(&self) -> &DeliveryLedger {
        &self.delivery_ledger
    }

    /// Registered mirror rules.
    pub fn mirror_rules(&self) -> &[MirrorRule] {
        &self.mirror_rules
    }

    /// Target channels an outbound response on `platform`/`channel` should
    /// be mirrored to (hermes `gateway/mirror.py` parity). The caller sends
    /// the mirrored copies via `send_to_platform`.
    pub fn mirror_targets(&self, platform: &str, channel: &str) -> Vec<String> {
        self.mirror_rules
            .iter()
            .filter(|r| r.matches(platform, channel))
            .map(|r| r.target_channel.clone())
            .collect()
    }

    /// Run one handler turn under the gateway lifecycle: acquire the
    /// per-session turn lease (busy sessions get a polite refusal instead
    /// of a concurrent agent run), track the turn for stall detection, and
    /// record the delivery outcome in the ledger.
    async fn handle_with_lifecycle(
        &self,
        handler: &Arc<dyn MessageHandler>,
        message: IncomingMessage,
        session_key: &str,
    ) -> Result<OutgoingMessage> {
        // Turn lease — one in-flight turn per session key.
        let Some(_guard) = self.turn_lease.try_acquire(session_key).await else {
            debug!(
                session = %session_key,
                "Turn lease busy; polite refusal instead of concurrent agent run"
            );
            return Ok(OutgoingMessage::new(
                &message.channel_id,
                "⏳ Still working on your previous message — I'll reply as soon as it's done.",
            )
            .with_thread_id(message.thread_id));
        };

        self.stall_tracker.touch(session_key).await;
        let result = handler.handle(message.clone()).await;
        self.stall_tracker.complete(session_key).await;

        let mut response = match result {
            Ok(r) => r,
            Err(e) => {
                self.delivery_ledger
                    .record(&message.platform, &message.channel_id, "", "failed")
                    .await;
                return Err(e);
            }
        };

        // (iter-1132) A webhook subscription's `deliver` target: the answer
        // goes to the platform the operator named, not back to the inbound
        // channel — the webhook adapter is inbound-only, so an answer sent
        // there is dropped on the floor.
        let ledger_target = match &message.deliver_to {
            Some(target) => {
                response.channel_id = target.channel_id.clone();
                response.target_platform = Some(target.platform.clone());
                (target.platform.clone(), target.channel_id.clone())
            }
            None => (message.platform.clone(), message.channel_id.clone()),
        };

        self.delivery_ledger
            .record(
                &ledger_target.0,
                &ledger_target.1,
                &response.content,
                "delivered",
            )
            .await;
        Ok(response)
    }

    /// Start the gateway and all enabled adapters
    pub async fn start(&self) -> Result<()> {
        *self.running.write().await = true;

        for (name, adapter) in &self.adapters {
            if adapter.is_enabled() {
                info!(platform = %name, "Starting platform adapter");
                if let Err(e) = adapter.start().await {
                    error!(platform = %name, error = %e, "Failed to start adapter");
                    Self::supervise_adapter_start(
                        self.running.clone(),
                        name.clone(),
                        Arc::clone(adapter),
                        None,
                    );
                }
            }
        }

        Ok(())
    }

    /// Stop the gateway and all adapters
    pub async fn stop(&self) -> Result<()> {
        *self.running.write().await = false;

        for (name, adapter) in &self.adapters {
            info!(platform = %name, "Stopping platform adapter");
            if let Err(e) = adapter.stop().await {
                error!(platform = %name, error = %e, "Failed to stop adapter");
            }
        }

        // The delivery ledger's summary.
        //
        // The ledger is IN-MEMORY in this process, and nothing else
        // exposes it: `operant gateway status` reads the CONFIG file, and
        // the daemon writes no status file. So the counts the ledger was
        // built for — how many sends were delivered, how many failed —
        // were visible NOWHERE. The stop path is the one moment the
        // operator is guaranteed to see the log.
        let delivered = self.delivery_ledger.delivered_count().await;
        let failed = self.delivery_ledger.failed_count().await;
        if delivered > 0 || failed > 0 {
            info!(delivered, failed, "gateway delivery summary");
        }

        Ok(())
    }

    /// Start all enabled adapters with a shared message channel.
    /// Each adapter that supports it will spawn a polling/listening
    /// task and forward incoming messages through the sender.
    pub async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        // Mark the gateway running here too: callers that boot exclusively
        // through the channel path (gateway_runner) never invoked `start`, so
        // supervise_adapter_start's `running` gate and `stop` both depended on
        // this flag being set. Starting an adapter twice (once via each entry
        // point) double-delivers every outbound message — see R36.
        *self.running.write().await = true;
        for (name, adapter) in &self.adapters {
            if adapter.is_enabled() {
                info!(platform = %name, "Starting platform adapter with channel");
                if let Err(e) = adapter.start_with_channel(message_tx.clone()).await {
                    error!(platform = %name, error = %e, "Failed to start adapter with channel");
                    Self::supervise_adapter_start(
                        self.running.clone(),
                        name.clone(),
                        Arc::clone(adapter),
                        Some(message_tx.clone()),
                    );
                }
            }
        }
        Ok(())
    }

    /// Supervise a failed adapter start in the background: transient failures
    /// (boot-time network race, DNS hiccup, provider outage) must not leave a
    /// platform permanently dead for the life of the process — systemd only
    /// restarts on process exit, and the gateway process stays up. Retries
    /// with capped exponential backoff until the adapter starts or the
    /// gateway stops. Telegram's getMe gate fires before any listen task is
    /// spawned, so retrying after Err cannot double-start an adapter.
    fn supervise_adapter_start(
        running: Arc<RwLock<bool>>,
        name: String,
        adapter: Arc<dyn PlatformAdapter>,
        message_tx: Option<mpsc::UnboundedSender<IncomingMessage>>,
    ) {
        tokio::spawn(async move {
            let mut delay_secs: u64 = 5;
            loop {
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                delay_secs = (delay_secs.saturating_mul(2)).min(300);
                if !*running.read().await {
                    debug!(platform = %name, "Gateway stopped — abandoning adapter restart");
                    return;
                }
                if !adapter.is_enabled() {
                    debug!(platform = %name, "Adapter disabled — abandoning adapter restart");
                    return;
                }
                info!(platform = %name, delay_secs, "Retrying platform adapter start");
                let result = match &message_tx {
                    Some(tx) => adapter.start_with_channel(tx.clone()).await,
                    None => adapter.start().await,
                };
                match result {
                    Ok(()) => {
                        info!(platform = %name, "Platform adapter started after retry");
                        return;
                    }
                    Err(e) => {
                        error!(
                            platform = %name,
                            error = %e,
                            next_retry_secs = delay_secs,
                            "Adapter start retry failed"
                        );
                    }
                }
            }
        });
    }

    /// Check if the gateway is running
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// Get the number of registered adapters.
    pub fn adapter_count(&self) -> usize {
        self.adapters.len()
    }

    /// Get a registered adapter by platform name (adapters are keyed by
    /// `PlatformAdapter::name()`).
    pub fn adapter_for(&self, platform: &str) -> Option<Arc<dyn PlatformAdapter>> {
        self.adapters.get(platform).cloned()
    }

    // (iter-151: Gateway::status() deleted — duplicate of get_platform_status,
    // both had zero external callers. Use adapter_count() for adapter count.)

    /// Route an incoming message to the handler and send response
    pub async fn route_message(&self, message: IncomingMessage) -> Result<Option<OutgoingMessage>> {
        self.messages_processed.fetch_add(1, Ordering::SeqCst);

        debug!(
            platform = %message.platform,
            user = %message.user_id,
            content = %message.content,
            "Routing message"
        );

        // Feed capture (plan-2026-10-09 §3) — BEFORE the admin gate at all
        // costs: channel/group posts arrive from arbitrary users, and the
        // gate below would both miss them (capture-in-handle only ever sees
        // the operator's own posts) and answer "not authorized" INTO the
        // channel. Channel posts are feed-ONLY: record, no turn, no reply.
        // Group messages record AND fall through — the operator's
        // group-command surface keeps its existing turn behavior.
        if message.is_channel_post || message.is_group_chat {
            if let Some(handler) = &self.message_handler {
                handler.record_feed(&message);
            }
            if message.is_channel_post {
                return Ok(None);
            }
        }

        // Track session in persistent store if available. Thread id is
        // propagated so forum topics / threads get their own persistent
        // session records (hermes build_session_key parity).
        if let Some(ref store) = self.persistent_sessions {
            let source = SessionSource {
                platform: message.platform.clone(),
                chat_id: message.channel_id.clone(),
                chat_name: None,
                chat_type: if message.thread_id.is_some() {
                    "thread"
                } else if message.is_group_chat {
                    "group"
                } else {
                    "dm"
                }
                .to_string(),
                user_id: Some(message.user_id.clone()),
                user_name: Some(message.username.clone()),
                thread_id: message.thread_id.map(|t| t.to_string()),
                chat_topic: None,
                user_id_alt: None,
                chat_id_alt: None,
                is_bot: false,
                guild_id: None,
                parent_chat_id: None,
                message_id: None,
                role_authorized: false,
            };
            // Create-or-touch, discarding the resolved entry on purpose.
            //
            // The CREATE is the point. `get_or_create_session` derives a
            // deterministic key from `source` (build_session_key,
            // gateway_session.rs:810) and then either inserts a new row
            // (save_entry -> SQLite, gateway_session.rs:886) or bumps
            // `updated_at` on the existing one (gateway_session.rs:856-865).
            // Without this call the chat has no persistent record at all:
            // `session_count()` (read by `get_stats`) and `list_sessions()`
            // never see it, and the next message would mint a second,
            // unrelated session id for the same conversation.
            //
            // Discarded: `Result<SessionEntry, Error>` — the created-or-
            // touched entry on Ok; a store/DB write failure on Err, which
            // must not abort message routing. Nothing in this function can
            // consume the id: the turn-lease key computed below is a
            // different namespace ("{platform}:{user_id}:{channel_id}" vs
            // the store's "agent:main:{platform}:{chat_type}:…"), and
            // `MessageHandler::handle` takes only an `IncomingMessage`, so
            // there is nowhere to pass an id without changing a public
            // trait. The id is not stable across calls either — reset policy
            // and suspended sessions rotate it (reset_session_inner,
            // gateway_session.rs:929).
            let _ = store.get_or_create_session(&source, false);
        }

        // Check if user is admin.
        //
        // An AUTHENTICATED event is not a user: the gate is a user gate, and
        // the user of a signature-verified webhook is the operator's own
        // external service. Without this exemption every webhook event was
        // refused with "You are not authorized" in any deployment that
        // configures `admins` — which is every real deployment. (iter-1132)
        if !message.authenticated
            && !self.config.admins.is_empty()
            && !self.config.admins.contains(&message.user_id)
        {
            debug!(user = %message.user_id, "User not authorized");
            return Ok(Some(
                OutgoingMessage::new(
                    &message.channel_id,
                    "You are not authorized to use this bot.",
                )
                .with_thread_id(message.thread_id),
            ));
        }

        let handler = match &self.message_handler {
            Some(h) => h,
            None => {
                warn!("No message handler configured");
                return Ok(None);
            }
        };

        // Session key for the turn lease / stall tracker / delivery ledger.
        // Thread-aware so a long-running agent in one forum topic never
        // blocks another topic's turns (hermes session isolation parity).
        let session_key = match message.thread_id {
            Some(tid) => format!(
                "{}:{}:{}:thread:{}",
                message.platform, message.user_id, message.channel_id, tid
            ),
            None => format!(
                "{}:{}:{}",
                message.platform, message.user_id, message.channel_id
            ),
        };

        // Global emergency stop (hermes estop parity): while engaged, new
        // turns get a brief paused reply instead of an agent run. In-flight
        // work is never killed — this is pause-new-work.
        if crate::estop::is_engaged() {
            let state = crate::estop::state();
            let reason = state
                .reason
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            info!("Gateway turn rejected: ESTOP engaged{reason}");
            return Ok(Some(
                OutgoingMessage::new(
                    &message.channel_id,
                    format!("⏸️ Operant is paused{reason}. Resume with `operant resume`."),
                )
                .with_thread_id(message.thread_id),
            ));
        }

        // Active-session cap (hermes `max_concurrent_sessions` parity): a
        // new session is refused when the cap is reached; existing holders
        // keep their slots (their locks refresh on every message).
        if let Some(max) = self.config.max_concurrent_sessions {
            let tracker = crate::active_sessions::ActiveSessionTracker::new(
                crate::active_sessions::ActiveSessionTracker::default_dir(),
                Some(max),
            );
            if !tracker.acquire(&session_key)? {
                warn!(
                    session = %session_key,
                    max,
                    "Gateway session refused: concurrency cap reached"
                );
                return Ok(Some(
                    OutgoingMessage::new(
                        &message.channel_id,
                        format!(
                            "Too many concurrent sessions (limit {max}). Please try again later."
                        ),
                    )
                    .with_thread_id(message.thread_id),
                ));
            }
            let result = self
                .handle_with_lifecycle(handler, message, &session_key)
                .await;
            tracker.release(&session_key);
            let response = result?;
            return Ok(Some(response));
        }

        let response = self
            .handle_with_lifecycle(handler, message, &session_key)
            .await?;

        Ok(Some(response))
    }

    /// Send a message to a specific platform
    pub async fn send_to_platform(&self, platform: &str, message: OutgoingMessage) -> Result<()> {
        // v6 outline §2 P1-A: the dispatch breaker seam — an open
        // channel+class pair never reaches the adapter, an open pair past
        // its cooldown lets exactly one probe through.
        if let Some(rejection) = self.breaker_guard(platform, &message).await {
            return Err(rejection);
        }
        let adapter = match self.adapters.get(platform) {
            Some(a) => a,
            None => {
                return Err(crate::error::Error::Agent(format!(
                    "Unknown platform: {}",
                    platform
                )));
            }
        };
        // Snapshot the channel id (the message moves into the adapter).
        let channel_id = message.channel_id.clone();

        let outcome = adapter.send_message(message).await;
        self.breaker_record(platform, &channel_id, outcome.as_ref().err())
            .await;

        // Record the delivery. The ledger used to be written ONLY on the
        // REPLY path (`handle_with_lifecycle`), so every PROACTIVE send —
        // a scheduled message, a cron push, a permission notice, a
        // gateway event — was invisible in the operator's view of
        // delivery health. The ledger's own doc says "Every send is
        // recorded — success or failure, so operators can inspect recent
        // delivery health instead of guessing whether a reply actually
        // left the gateway"; a proactive send is a send.
        //
        // And the failure is now RECORDED, which matters because most
        // proactive call sites discard the result (`let _ = ...`), so a
        // failed proactive send was previously invisible everywhere.
        let status = if outcome.is_ok() {
            "delivered"
        } else {
            "failed"
        };
        self.delivery_ledger
            .record(platform, &channel_id, "", status)
            .await;

        outcome
    }

    /// Delete a message that was previously sent on `platform`.
    ///
    /// The adapter method existed and Telegram implemented it, but nothing in
    /// the gateway or the CLI could reach it, so a message operant sent could
    /// never be retracted on any platform (RT-126). This is the path.
    pub async fn delete_message_on_platform(
        &self,
        platform: &str,
        channel_id: &str,
        message_id: &str,
    ) -> Result<()> {
        let adapter = self
            .adapters
            .get(platform)
            .ok_or_else(|| crate::error::Error::Agent(format!("Unknown platform: {}", platform)))?;
        adapter.delete_message(channel_id, message_id).await
    }

    /// Send a message and return the platform message ID.
    pub async fn send_message_return_id(
        &self,
        platform: &str,
        message: OutgoingMessage,
    ) -> Result<String> {
        // v6 outline §2 P1-A: the same breaker seam (callers that need
        // the message id back — proactive sends, edits — must not bypass
        // the guard the plain send enforces).
        if let Some(rejection) = self.breaker_guard(platform, &message).await {
            return Err(rejection);
        }
        let adapter = match self.adapters.get(platform) {
            Some(a) => a,
            None => {
                return Err(crate::error::Error::Agent(format!(
                    "Unknown platform: {}",
                    platform
                )));
            }
        };
        let channel_id = message.channel_id.clone();
        let outcome = adapter.send_message_return_id(message).await;
        self.breaker_record(platform, &channel_id, outcome.as_ref().err())
            .await;
        // The delivery ledger.
        //
        // The plain send (`send_to_platform`) has always recorded here.
        // The STREAMING path — this method — did NOT: so every answer
        // the agent streamed to a platform was invisible in the delivery
        // health the ledger exists to show, while the proactive sends
        // around it were visible. The two paths disagreed about the same
        // thing.
        //
        // The status is the OUTCOME of the send, so a failed send is
        // recorded as failed rather than as a success.
        let status = if outcome.is_ok() {
            "delivered"
        } else {
            "failed"
        };
        self.delivery_ledger
            .record(platform, &channel_id, "", status)
            .await;
        outcome
    }

    /// Edit an existing message on a platform.
    pub async fn edit_message(
        &self,
        platform: &str,
        channel_id: &str,
        message_id: &str,
        message: OutgoingMessage,
    ) -> Result<String> {
        let adapter = match self.adapters.get(platform) {
            Some(a) => a,
            None => {
                return Err(crate::error::Error::Agent(format!(
                    "Unknown platform: {}",
                    platform
                )));
            }
        };
        adapter.edit_message(channel_id, message_id, &message).await
    }

    /// Send a voice/audio message to a platform channel.
    pub async fn send_voice(
        &self,
        platform: &str,
        channel_id: &str,
        audio_data: &[u8],
        format: &str,
    ) -> Result<()> {
        let adapter = match self.adapters.get(platform) {
            Some(a) => a,
            None => return Ok(()), // silently skip unsupported platforms
        };
        adapter.send_voice(channel_id, audio_data, format).await
    }

    /// Send a typing indicator to a platform channel
    pub fn send_typing(
        &self,
        platform: &str,
        channel_id: &str,
        thread_id: Option<i64>,
    ) -> Result<()> {
        if let Some(adapter) = self.adapters.get(platform) {
            adapter.send_typing(channel_id, thread_id)?;
        }
        Ok(())
    }

    // ── Extended Gateway API ──

    /// Register a platform adapter using `&mut self` (in-place)
    pub fn register_adapter(&mut self, adapter: Arc<dyn PlatformAdapter>) {
        let name = adapter.name().to_string();
        info!(platform = %name, "Registering platform adapter");
        self.adapters.insert(name, adapter);
    }

    /// Get gateway statistics
    pub async fn get_stats(&self) -> GatewayStats {
        let uptime = self.start_time.elapsed().as_secs();

        GatewayStats {
            uptime_seconds: uptime,
            active_sessions: self
                .persistent_sessions
                .as_ref()
                .map(|s| s.session_count())
                .unwrap_or_else(|| self.session_store.get_session_count()),
            registered_channels: self.channel_directory.list_channels(None).len(),
            active_adapters: self.adapters.len(),
            messages_processed: self.messages_processed.load(Ordering::SeqCst),
            start_time: self.start_time_formatted.clone(),
        }
    }

    // (iter-151: get_platform_status deleted — zero external callers)

    /// Get a reference to the session store
    pub fn get_session_store(&self) -> &SessionStore {
        &self.session_store
    }

    /// Get the persistent session store, when the gateway runs with one
    /// (Wave 2, ORGANISM-ARCHITECTURE §2): the DB-backed layer that owns
    /// the employee binding — `SessionEntry.employee_id` — which survives
    /// restarts, unlike the in-memory [`SessionStore`]. The `/session`
    /// command and the turn prologue read and write the binding through
    /// THIS seam so there is exactly one employee-binding path.
    pub fn get_persistent_sessions(&self) -> Option<&Arc<PersistentSessionStore>> {
        self.persistent_sessions.as_ref()
    }

    /// Get a reference to the channel directory
    pub fn get_channel_directory(&self) -> &ChannelDirectory {
        &self.channel_directory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message() {
        let msg = IncomingMessage::new("telegram", "12345", "testuser", "67890", "Hello, world!");

        assert_eq!(msg.platform, "telegram");
        assert_eq!(msg.user_id, "12345");
        assert_eq!(msg.content, "Hello, world!");
    }

    #[test]
    fn test_outgoing_message() {
        let msg = OutgoingMessage::new("67890", "Response to you")
            .no_markdown()
            .with_reply_to("111");

        assert_eq!(msg.channel_id, "67890");
        assert_eq!(msg.content, "Response to you");
        assert!(!msg.parse_markdown);
        assert_eq!(msg.reply_to, Some("111".to_string()));
        assert_eq!(msg.thread_id, None);
    }

    /// A reply to a forum-topic message must carry the incoming
    /// message_thread_id so it lands in the same topic (hermes parity).
    #[test]
    fn test_outgoing_message_thread_id_roundtrip() {
        // Incoming forum message with a topic thread id.
        let incoming = IncomingMessage::new("telegram", "111", "ishan", "-100123", "hello")
            .with_thread_id(Some(65901));
        assert_eq!(incoming.thread_id, Some(65901));

        // Reply built from it forwards the thread id.
        let reply = OutgoingMessage::new(&incoming.channel_id, "hi back")
            .with_thread_id(incoming.thread_id);
        assert_eq!(reply.thread_id, Some(65901));

        // Messages without a thread context stay None (general chat).
        let plain = OutgoingMessage::new("-100123", "plain");
        assert_eq!(plain.thread_id, None);
    }

    /// A forum message update must surface message_thread_id on the parsed
    /// IncomingMessage so replies are routed back to the topic.
    #[test]
    fn parse_update_captures_message_thread_id() {
        let update = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 10,
                "message_thread_id": 91609,
                "from": {"id": 111, "is_bot": false, "username": "ishan"},
                "chat": {"id": -100123, "type": "supergroup"},
                "text": "Testing the operant functioning."
            }
        });
        let msg = TelegramAdapter::parse_update(update)
            .expect("parse ok")
            .expect("message present");
        assert_eq!(msg.channel_id, "-100123");
        assert_eq!(msg.thread_id, Some(91609));
        assert!(msg.is_group_chat);
    }

    /// Service messages (forum topic created, member joined, …) carry no
    /// user content and must be dropped by parse_update — they previously
    /// fell through to "[sent an attachment]" and spawned a bogus agent turn
    /// (the double-reply bug: topic-creation service message + real text).
    #[test]
    fn parse_update_filters_service_messages() {
        // forum_topic_created — the exact shape Telegram sends when a user
        // creates a new topic (no text, no from, no media).
        let topic_created = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 10,
                "message_thread_id": 91663,
                "chat": {"id": -100123, "type": "supergroup"},
                "date": 1752872700,
                "forum_topic_created": {"name": "New Chat", "icon_color": 0}
            }
        });
        assert!(
            TelegramAdapter::parse_update(topic_created)
                .expect("parse ok")
                .is_none()
        );

        // new_chat_members
        let joined = serde_json::json!({
            "update_id": 2,
            "message": {
                "message_id": 11,
                "chat": {"id": -100123, "type": "supergroup"},
                "new_chat_members": [{"id": 999, "first_name": "Bot"}]
            }
        });
        assert!(
            TelegramAdapter::parse_update(joined)
                .expect("parse ok")
                .is_none()
        );

        // pinned_message
        let pinned = serde_json::json!({
            "update_id": 3,
            "message": {
                "message_id": 12,
                "chat": {"id": -100123, "type": "supergroup"},
                "pinned_message": {"message_id": 9, "text": "rules"}
            }
        });
        assert!(
            TelegramAdapter::parse_update(pinned)
                .expect("parse ok")
                .is_none()
        );

        // A media-less message with no recognized content must NOT fabricate
        // "[sent an attachment]" — it is not a user message.
        let unknown = serde_json::json!({
            "update_id": 4,
            "message": {
                "message_id": 13,
                "from": {"id": 111, "is_bot": false, "username": "ishan"},
                "chat": {"id": -100123, "type": "supergroup"},
                "date": 1752872700
            }
        });
        assert!(
            TelegramAdapter::parse_update(unknown)
                .expect("parse ok")
                .is_none()
        );

        // Sanity: a real text message in the same topic still parses.
        let text = serde_json::json!({
            "update_id": 5,
            "message": {
                "message_id": 14,
                "message_thread_id": 91663,
                "from": {"id": 111, "is_bot": false, "username": "ishan"},
                "chat": {"id": -100123, "type": "supergroup"},
                "text": "Testing operant executive functions."
            }
        });
        let msg = TelegramAdapter::parse_update(text)
            .expect("parse ok")
            .expect("message present");
        assert_eq!(msg.content, "Testing operant executive functions.");
        assert_eq!(msg.thread_id, Some(91663));
    }

    /// Media types beyond photo/document/voice must get accurate placeholders
    /// instead of the generic "[sent an attachment]" fallback.
    #[test]
    fn parse_update_media_placeholders() {
        let video = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 1,
                "from": {"id": 1, "is_bot": false, "username": "u"},
                "chat": {"id": 2, "type": "private"},
                "video": {"file_id": "v1"}
            }
        });
        let msg = TelegramAdapter::parse_update(video)
            .expect("parse ok")
            .expect("present");
        assert_eq!(msg.content, "[sent a video]");

        let audio = serde_json::json!({
            "update_id": 2,
            "message": {
                "message_id": 2,
                "from": {"id": 1, "is_bot": false, "username": "u"},
                "chat": {"id": 2, "type": "private"},
                "audio": {"file_id": "a1"}
            }
        });
        let msg = TelegramAdapter::parse_update(audio)
            .expect("parse ok")
            .expect("present");
        assert_eq!(msg.content, "[sent an audio message]");

        // caption-bearing video uses the caption.
        let video_cap = serde_json::json!({
            "update_id": 3,
            "message": {
                "message_id": 3,
                "from": {"id": 1, "is_bot": false, "username": "u"},
                "chat": {"id": 2, "type": "private"},
                "video": {"file_id": "v2"},
                "caption": "watch this"
            }
        });
        let msg = TelegramAdapter::parse_update(video_cap)
            .expect("parse ok")
            .expect("present");
        assert_eq!(msg.content, "watch this");
    }

    /// A photo update must extract the largest PhotoSize file_id (hermes
    /// takes `photo[-1]` — sizes are sorted ascending).
    #[test]
    fn telegram_attachment_file_id_takes_largest_photo() {
        let message = serde_json::json!({
            "photo": [
                {"file_id": "small", "width": 100, "height": 100},
                {"file_id": "large", "width": 1000, "height": 1000},
            ]
        });
        assert_eq!(
            telegram_attachment_file_id(&message).as_deref(),
            Some("large")
        );
    }

    /// A document update must extract the document file_id.
    #[test]
    fn telegram_attachment_file_id_document() {
        let message = serde_json::json!({
            "document": {"file_id": "doc_1", "file_name": "report.pdf"}
        });
        assert_eq!(
            telegram_attachment_file_id(&message).as_deref(),
            Some("doc_1")
        );
    }

    /// A voice update must extract the voice file_id; plain text yields None.
    #[test]
    fn telegram_attachment_file_id_voice_and_text() {
        let voice = serde_json::json!({"voice": {"file_id": "voice_1"}});
        assert_eq!(
            telegram_attachment_file_id(&voice).as_deref(),
            Some("voice_1")
        );
        assert_eq!(telegram_attachment_file_id(&serde_json::json!({})), None);
    }

    /// Extension guessing must map common Telegram file paths and fall back
    /// to the hint for unknown ones.
    #[test]
    fn telegram_media_extension_guessing() {
        assert_eq!(
            telegram_media_extension("docs/photo_123.jpg", ".bin"),
            ".jpg"
        );
        assert_eq!(telegram_media_extension("voice/msg.oga", ".bin"), ".oga");
        assert_eq!(
            telegram_media_extension("documents/notes.md", ".bin"),
            ".md"
        );
        assert_eq!(
            telegram_media_extension("files/unknown-123", ".pdf"),
            ".pdf"
        );
    }

    /// with_media_urls must round-trip the cached attachment paths onto the
    /// IncomingMessage so the runner can inject them into the agent prompt.
    #[test]
    fn incoming_message_media_urls_roundtrip() {
        let msg = IncomingMessage::new("telegram", "1", "u", "c", "[sent a photo]")
            .with_media_urls(vec!["/home/u/.operant/media/photo_1.jpg".to_string()]);
        assert_eq!(msg.media_urls.len(), 1);
        assert!(msg.media_urls[0].ends_with("photo_1.jpg"));
        assert!(
            IncomingMessage::new("telegram", "1", "u", "c", "hi")
                .media_urls
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_gateway_config() {
        let config = GatewayConfig::default();
        assert!(!config.telegram_enabled);
        assert!(!config.discord_enabled);
    }

    /// An inline-keyboard approve tap must synthesize a `/approve` text
    /// message on the same channel, so the shared gateway_commands resolver
    /// handles it identically to a typed reply.
    #[test]
    fn approval_callback_approve_synthesizes_slash_approve() {
        let cb = serde_json::json!({
            "id": "cb_1",
            "data": "approval:approve",
            "from": {"id": 111, "username": "ishan"},
            "message": {
                "chat": {"id": -100123},
                "message_id": 42,
                "text": "🔧 Permission required: terminal"
            }
        });
        let msg = TelegramAdapter::approval_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.platform, "telegram");
        assert_eq!(msg.user_id, "111");
        assert_eq!(msg.channel_id, "-100123");
        assert_eq!(msg.content, "/approve");
    }

    #[test]
    fn approval_callback_deny_synthesizes_slash_deny() {
        let cb = serde_json::json!({
            "id": "cb_2",
            "data": "approval:deny",
            "from": {"id": 222, "username": "other"},
            "message": {"chat": {"id": 999}}
        });
        let msg = TelegramAdapter::approval_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.content, "/deny");
        assert_eq!(msg.channel_id, "999");
    }

    #[test]
    fn non_approval_callback_is_ignored() {
        let cb = serde_json::json!({
            "id": "cb_3",
            "data": "some-other-action",
            "from": {"id": 333},
            "message": {"chat": {"id": 1}}
        });
        assert!(TelegramAdapter::approval_message_from_callback(&cb).is_none());
    }

    #[test]
    fn callback_without_chat_is_ignored() {
        let cb = serde_json::json!({
            "id": "cb_4",
            "data": "approval:approve",
            "from": {"id": 444}
        });
        assert!(TelegramAdapter::approval_message_from_callback(&cb).is_none());
    }

    /// An approval tap in a forum topic must carry the thread_id (so the
    /// `/approve` resolution reply lands in the same topic, not the general
    /// chat) and the `approval_callback` raw marker (so the dispatch layer
    /// can edit the prompt message to show the outcome — hermes
    /// `query.edit_message_text` parity).
    #[test]
    fn approval_callback_carries_thread_and_edit_marker() {
        let cb = serde_json::json!({
            "id": "cb_5",
            "data": "approval:approve",
            "from": {"id": 555, "first_name": "Ishan", "username": "ishan"},
            "message": {
                "chat": {"id": -100123, "type": "supergroup"},
                "message_id": 42,
                "message_thread_id": 91609,
                "text": "🔧 Permission required: terminal"
            }
        });
        let msg = TelegramAdapter::approval_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.thread_id, Some(91609));
        assert!(msg.is_group_chat);
        let marker = msg
            .raw
            .get("approval_callback")
            .expect("edit marker present");
        assert_eq!(marker["chat_id"], "-100123");
        assert_eq!(marker["message_id"], 42);
        assert_eq!(marker["thread_id"], 91609);
        assert_eq!(marker["user"], "Ishan");
        assert_eq!(marker["action"], "approve");
    }

    /// A clarify() button tap must synthesize a message carrying the
    /// `choice_callback` raw marker (prompt chat/message ids, tap user and
    /// the selected index) so the dispatch layer resolves the pending
    /// question with the option text and edits the prompt message.
    #[test]
    fn choice_callback_synthesizes_marked_message() {
        let cb = serde_json::json!({
            "id": "cb_6",
            "data": "choice:1",
            "from": {"id": 666, "first_name": "Ishan"},
            "message": {
                "chat": {"id": -100123, "type": "supergroup"},
                "message_id": 77,
                "message_thread_id": 91609,
                "text": "❓ Which approach?"
            }
        });
        let msg = TelegramAdapter::choice_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.thread_id, Some(91609));
        assert!(msg.is_group_chat);
        assert_eq!(msg.content, "__choice__1");
        let marker = msg.raw.get("choice_callback").expect("edit marker present");
        assert_eq!(marker["chat_id"], "-100123");
        assert_eq!(marker["message_id"], 77);
        assert_eq!(marker["user"], "Ishan");
        assert_eq!(marker["idx"], 1);
    }

    #[test]
    fn non_choice_callback_is_ignored() {
        let cb = serde_json::json!({
            "id": "cb_7",
            "data": "approval:deny",
            "from": {"id": 777},
            "message": {"chat": {"id": 1}}
        });
        assert!(TelegramAdapter::choice_message_from_callback(&cb).is_none());
    }

    /// An `approval:always` button tap must synthesize `/approve always` so
    /// the shared gateway_commands resolver grants the permanent allowlist
    /// (hermes `always` → command_allowlist parity), carrying the thread and
    /// the edit marker for the outcome label.
    #[test]
    fn approval_always_callback_synthesizes_approve_always() {
        let cb = serde_json::json!({
            "id": "cb_always",
            "data": "approval:always",
            "from": {"id": 42, "first_name": "Ishan", "username": "ishanp"},
            "message": {
                "chat": {"id": -100123, "type": "supergroup"},
                "message_id": 88,
                "message_thread_id": 91609
            }
        });
        let msg = TelegramAdapter::approval_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.content, "/approve always");
        assert_eq!(msg.thread_id, Some(91609));
        assert!(msg.is_group_chat);
        let marker = msg
            .raw
            .get("approval_callback")
            .expect("edit marker present");
        assert_eq!(marker["chat_id"], "-100123");
        assert_eq!(marker["message_id"], 88);
        assert_eq!(marker["action"], "always");
        assert_eq!(marker["user"], "Ishan");
    }

    /// Echo handler that optionally blocks inside `handle` until released
    /// (simulates a long-running agent turn).
    struct BlockingEchoHandler {
        gate: Option<Arc<tokio::sync::Notify>>,
    }

    /// Feed-capture contract (plan-2026-10-09 §3): a channel post is
    /// recorded BEFORE the admin gate — a non-admin poster would be refused
    /// by the gate, and capture-in-`handle` would never see the post — and
    /// the post NEVER turns or draws a reply into the channel. A group
    /// message records AND still falls through to normal routing.
    #[tokio::test]
    async fn channel_post_captures_feed_and_never_turns_or_replies() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingHandler {
            feeds: AtomicUsize,
            turns: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl MessageHandler for CountingHandler {
            async fn handle(&self, _message: IncomingMessage) -> Result<OutgoingMessage> {
                self.turns.fetch_add(1, Ordering::SeqCst);
                Ok(OutgoingMessage::new("c", "ok"))
            }
            fn record_feed(&self, _message: &IncomingMessage) {
                self.feeds.fetch_add(1, Ordering::SeqCst);
            }
        }

        let handler = Arc::new(CountingHandler {
            feeds: AtomicUsize::new(0),
            turns: AtomicUsize::new(0),
        });
        let config = GatewayConfig {
            admins: vec!["admin".to_string()],
            ..GatewayConfig::default()
        };
        let gw = Arc::new(Gateway::new(config).with_handler(handler.clone()));

        // Channel post from a NON-ADMIN: capture must happen before the
        // gate; no turn; no reply (None, not the "not authorized" message).
        let post =
            IncomingMessage::new("telegram", "anon", "Some Channel", "-1001", "daily update")
                .with_channel_post(true);
        let out = gw.route_message(post).await.expect("route ok");
        assert!(
            out.is_none(),
            "channel post must never produce a reply: {out:?}"
        );
        assert_eq!(handler.feeds.load(Ordering::SeqCst), 1);
        assert_eq!(handler.turns.load(Ordering::SeqCst), 0);

        // Group message from the ADMIN: recorded as feed AND still turns.
        let group = IncomingMessage::new("telegram", "admin", "alice", "g1", "run the audit")
            .with_group_chat(true);
        let out = gw.route_message(group).await.expect("route ok");
        assert!(out.is_some(), "the admin's group command keeps its turn");
        assert_eq!(handler.feeds.load(Ordering::SeqCst), 2);
        assert_eq!(handler.turns.load(Ordering::SeqCst), 1);
    }

    #[async_trait::async_trait]
    impl MessageHandler for BlockingEchoHandler {
        async fn handle(&self, message: IncomingMessage) -> Result<OutgoingMessage> {
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            Ok(OutgoingMessage::new(
                &message.channel_id,
                format!("echo:{}", message.content),
            ))
        }
    }

    /// The per-session turn lease must refuse a second concurrent turn for
    /// the same session while the first is in flight, and the stall tracker
    /// must observe the in-flight turn.
    #[tokio::test]
    async fn route_message_serializes_concurrent_turns_via_lease() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let handler = Arc::new(BlockingEchoHandler {
            gate: Some(gate.clone()),
        });
        let gw = Arc::new(Gateway::new(GatewayConfig::default()).with_handler(handler));

        let msg1 = IncomingMessage::new("telegram", "u1", "alice", "c1", "first");
        let msg2 = IncomingMessage::new("telegram", "u1", "alice", "c1", "second");

        let t1 = tokio::spawn({
            let gw = gw.clone();
            async move { gw.route_message(msg1).await.unwrap() }
        });

        // Wait for turn 1 to acquire the lease and enter handle().
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if gw.turn_lease().is_busy("telegram:u1:c1").await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            gw.turn_lease().is_busy("telegram:u1:c1").await,
            "turn 1 must hold the lease"
        );
        assert_eq!(gw.stall_tracker().active_count().await, 1);

        // Second message for the same session: busy reply, no agent run.
        let busy = gw.route_message(msg2).await.unwrap().expect("busy reply");
        assert!(
            busy.content
                .contains("Still working on your previous message"),
            "got: {}",
            busy.content
        );
        assert_eq!(gw.stall_tracker().active_count().await, 1, "no second turn");

        // Release turn 1 — it completes and the lease is freed.
        gate.notify_one();
        let first = t1.await.expect("task panicked").expect("response");
        assert_eq!(first.content, "echo:first");
        assert!(!gw.turn_lease().is_busy("telegram:u1:c1").await);
        assert_eq!(gw.stall_tracker().active_count().await, 0);
    }

    /// A PROACTIVE send must appear in the delivery ledger.
    ///
    /// The ledger used to be written ONLY on the reply path
    /// (`handle_with_lifecycle`), so every proactive send — a scheduled
    /// message, a cron push, a permission notice, a gateway event — was
    /// invisible in the operator's view of delivery health. The ledger's
    /// own doc says "Every send is recorded — success or failure".
    ///
    /// And the FAILURE is recorded, which matters because most proactive
    /// call sites discard the result (`let _ = ...`), so a failed
    /// proactive send was previously invisible everywhere.
    #[tokio::test]
    async fn a_proactive_send_is_recorded_in_the_delivery_ledger() {
        let gw = Arc::new(
            Gateway::new(GatewayConfig::default())
                .with_adapter(Arc::new(FlakyAdapter {
                    name: "flaky".to_string(),
                    sends: Arc::new(AtomicUsize::new(0)),
                    error: "the channel is gone".to_string(),
                }))
                .with_adapter(Arc::new(SucceedingAdapter {
                    name: "working".to_string(),
                })),
        );

        // A proactive send that succeeds.
        gw.send_to_platform("working", OutgoingMessage::new("c1", "hello"))
            .await
            .expect("the proactive send succeeds");
        // A proactive send that fails.
        let failed = gw
            .send_to_platform("flaky", OutgoingMessage::new("c2", "hello"))
            .await;
        assert!(
            failed.is_err(),
            "the failing adapter must surface its error"
        );

        assert_eq!(
            gw.delivery_ledger().delivered_count().await,
            1,
            "a proactive send that succeeded must be recorded as delivered"
        );
        assert_eq!(
            gw.delivery_ledger().failed_count().await,
            1,
            "a proactive send that failed must be recorded as failed"
        );
    }

    /// Every successful route records a delivery in the ledger; mirror rules
    /// resolve matching target channels.
    #[tokio::test]
    async fn route_message_records_delivery_and_mirror_targets() {
        let handler = Arc::new(BlockingEchoHandler { gate: None });
        let gw = Arc::new(
            Gateway::new(GatewayConfig::default())
                .with_handler(handler)
                .with_mirror_rule(MirrorRule {
                    platform: "telegram".to_string(),
                    source_channel: "c1".to_string(),
                    target_channel: "group:ops".to_string(),
                }),
        );

        let msg = IncomingMessage::new("telegram", "u1", "alice", "c1", "hi");
        let resp = gw.route_message(msg).await.unwrap().expect("response");
        assert_eq!(resp.content, "echo:hi");

        assert_eq!(gw.delivery_ledger().delivered_count().await, 1);
        assert_eq!(gw.delivery_ledger().failed_count().await, 0);
        let recent = gw.delivery_ledger().recent(5).await;
        assert_eq!(recent[0].platform, "telegram");
        assert_eq!(recent[0].channel_id, "c1");
        assert!(recent[0].content_len > 0);

        // Mirror rule matches the source channel and resolves the target.
        assert_eq!(
            gw.mirror_targets("telegram", "c1"),
            vec!["group:ops".to_string()]
        );
        assert!(gw.mirror_targets("telegram", "c9").is_empty());
        assert!(gw.mirror_targets("discord", "c1").is_empty());
    }

    /// A webhook event is not a user: the admin gate is a USER gate.
    ///
    /// Before iter-1132 every webhook event was refused with "You are not
    /// authorized to use this bot." in any deployment that configures
    /// `admins` — which is every real deployment. An AUTHENTICATED event (a
    /// signature-verified webhook) is the operator's own service talking, so
    /// the gate must not apply to it. An unauthenticated one is still
    /// refused: a public webhook is not a user either.
    #[tokio::test]
    async fn an_authenticated_event_is_not_refused_by_the_admin_gate() {
        let handler = Arc::new(BlockingEchoHandler { gate: None });
        let config = GatewayConfig {
            admins: vec!["alice".to_string()],
            ..GatewayConfig::default()
        };
        let gw = Arc::new(Gateway::new(config).with_handler(handler));

        let mut event = IncomingMessage::new(
            "webhook",
            "webhook",
            "Webhook",
            "webhook:orders",
            "an order",
        );
        let refused = gw
            .route_message(event.clone())
            .await
            .expect("route ok")
            .expect("a reply");
        assert!(
            refused.content.contains("not authorized"),
            "an unauthenticated event must still be refused: {}",
            refused.content
        );

        event.authenticated = true;
        let answered = gw
            .route_message(event)
            .await
            .expect("route ok")
            .expect("a reply");
        assert_eq!(answered.content, "echo:an order");
    }

    /// A webhook subscription's `deliver` target moves the answer.
    ///
    /// The inbound channel cannot carry a reply (the webhook adapter is
    /// inbound-only), so without this the agent's answer was dropped on the
    /// floor — the event was handled and the operator never learned the
    /// outcome.
    #[tokio::test]
    async fn a_delivery_target_moves_the_answer_to_the_named_platform() {
        let handler = Arc::new(BlockingEchoHandler { gate: None });
        let gw = Arc::new(Gateway::new(GatewayConfig::default()).with_handler(handler));

        let mut event = IncomingMessage::new(
            "webhook",
            "webhook",
            "Webhook",
            "webhook:orders",
            "an order",
        );
        event.deliver_to = Some(DeliveryTarget {
            platform: "telegram".to_string(),
            channel_id: "chat-42".to_string(),
        });
        let response = gw
            .route_message(event)
            .await
            .expect("route ok")
            .expect("a reply");
        assert_eq!(response.channel_id, "chat-42");
        assert_eq!(response.target_platform.as_deref(), Some("telegram"));

        // The ledger records the delivery where it actually went.
        let recent = gw.delivery_ledger().recent(5).await;
        assert_eq!(recent[0].platform, "telegram");
        assert_eq!(recent[0].channel_id, "chat-42");
    }

    #[tokio::test]
    async fn test_telegram_adapter_disabled() {
        let adapter = TelegramAdapter::new(None);
        assert!(!adapter.is_enabled());
    }

    #[tokio::test]
    async fn test_discord_adapter_disabled() {
        let adapter = DiscordAdapter::new(None);
        assert!(!adapter.is_enabled());
    }

    #[tokio::test]
    async fn test_slack_adapter_disabled() {
        let adapter = SlackAdapter::new(None);
        assert!(!adapter.is_enabled());
    }

    /// The Slack adapter's status report must say it cannot RECEIVE.
    ///
    /// The gateway's report used to say "enabled" and nothing more, so an
    /// operator who configured a Slack token believed they could receive
    /// Slack messages. There is no Socket Mode WebSocket in this tree:
    /// `start()` logs and returns, and nothing opens a `wss://`
    /// connection — so Slack messages never reach the agent.
    #[tokio::test]
    async fn the_slack_adapter_reports_that_it_cannot_receive() {
        let adapter = SlackAdapter::new(Some("xoxb-token".to_string()));
        assert!(adapter.is_enabled());
        let json = adapter.config_json();
        assert_eq!(
            json["inbound"],
            serde_json::json!(false),
            "the status report must say the adapter cannot receive"
        );
        assert_eq!(json["outbound"], serde_json::json!(true));
    }

    /// EVERY outbound-only adapter must say so in its status report.
    ///
    /// `PlatformAdapter::handle_update` is called from NOWHERE in the
    /// workspace — the Telegram and Discord poll loops use their own
    /// static parsers, and the webhook adapter has its own HTTP handler.
    /// So the trait method is dead for all seven adapters, and the
    /// adapters that implement a complete inbound parser — Slack,
    /// WhatsApp, Email, SMS — never receive anything.
    ///
    /// The status report used to say "enabled" and nothing more, so an
    /// operator believed they could receive messages on a channel that
    /// cannot deliver them. Every outbound-only adapter now carries
    /// `"inbound": false`.
    ///
    /// The test is over ALL FOUR, so adding a fifth adapter without the
    /// field fails it.
    #[tokio::test]
    async fn every_outbound_only_adapter_says_it_cannot_receive() {
        let adapters: Vec<(&str, serde_json::Value)> = vec![
            (
                "slack",
                SlackAdapter::new(Some("xoxb-token".to_string())).config_json(),
            ),
            ("whatsapp", WhatsAppAdapter::new(true).config_json()),
            ("email", EmailAdapter::new(true).config_json()),
            ("sms", SmsAdapter::new(true).config_json()),
        ];
        for (platform, json) in adapters {
            assert_eq!(
                json["inbound"],
                serde_json::json!(false),
                "the {platform} adapter's status report must say it cannot receive"
            );
            assert_eq!(
                json["outbound"],
                serde_json::json!(true),
                "the {platform} adapter can send"
            );
        }
    }

    // ── SessionStore tests ──

    #[test]
    fn test_session_store_create_and_get() {
        let store = SessionStore::new();
        let session = store
            .create_session("telegram", "user123", "channel456")
            .unwrap();
        assert_eq!(session.platform, "telegram");
        assert_eq!(session.platform_user_id, "user123");
        assert_eq!(session.platform_channel_id, "channel456");
        assert!(!session.session_id.is_empty());

        let fetched = store.get_session(&session.session_id);
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().platform_user_id, "user123");
    }

    #[test]
    fn test_session_store_find() {
        let store = SessionStore::new();
        store.create_session("discord", "user1", "chan1").unwrap();
        let s2 = store.create_session("telegram", "user2", "chan2").unwrap();

        let found = store.find_session("telegram", "user2", "chan2");
        assert!(found.is_some());
        assert_eq!(found.unwrap().session_id, s2.session_id);

        let not_found = store.find_session("telegram", "user1", "chan1");
        assert!(not_found.is_none());
    }

    // RT-176: `/title` used to mutate a session CLONE and drop it — the
    // store had no by-id persistence at all, so the command replied success
    // while the title never landed. set_title/session_metadata_value pin
    // the round-trip the command now depends on.
    #[test]
    fn session_store_set_title_persists_and_reads_back() {
        let store = SessionStore::new();
        let s = store
            .create_session("telegram", "u1", "c1")
            .expect("create");

        assert!(store.set_title(&s.session_id, " debugging the gateway "));
        assert_eq!(
            store
                .session_metadata_value(&s.session_id, "title")
                .as_deref(),
            Some(" debugging the gateway ")
        );

        // An unknown id is an honest failure, not a silent no-op.
        assert!(!store.set_title("no-such-session", "x"));
        assert_eq!(
            store.session_metadata_value("no-such-session", "title"),
            None
        );
    }

    // RT-176: `/branch` used to call the TRIPLE-keyed update with the
    // source session's (platform, user, channel) — annotating the source
    // row and never the branch it just created (and both rows share the
    // triple, so the write could never distinguish them). The by-id API
    // must reach exactly the named row.
    #[test]
    fn session_store_update_metadata_by_id_targets_only_the_named_session() {
        let store = SessionStore::new();
        let source = store
            .create_session("telegram", "u1", "c1")
            .expect("source");
        // Same triple — exactly the /branch shape.
        let branch = store
            .create_session("telegram", "u1", "c1")
            .expect("branch");
        assert_ne!(source.session_id, branch.session_id);

        assert!(store.update_session_metadata_by_id(
            &branch.session_id,
            &[
                ("title".to_string(), "branch-of-thing".to_string()),
                ("branched_from".to_string(), source.session_id.clone()),
            ],
        ));

        assert_eq!(
            store
                .session_metadata_value(&branch.session_id, "title")
                .as_deref(),
            Some("branch-of-thing")
        );
        assert_eq!(
            store
                .session_metadata_value(&branch.session_id, "branched_from")
                .as_deref(),
            Some(source.session_id.as_str())
        );
        // The SOURCE row is untouched — the old triple-keyed write would
        // have scribbled on exactly this row.
        assert_eq!(
            store.session_metadata_value(&source.session_id, "title"),
            None
        );
        assert_eq!(
            store.session_metadata_value(&source.session_id, "branched_from"),
            None
        );

        // Unknown id: false, not a panic and not a wrong-row write.
        assert!(!store.update_session_metadata_by_id("missing", &[]));
    }

    /// Each Telegram forum topic must get its own shared session — a message
    /// in a new topic must NOT continue the parent channel's (or another
    /// topic's) conversation. (hermes build_session_key parity — the bug
    /// where every topic in a supergroup shared one gateway session.)
    #[test]
    fn test_shared_session_scoped_per_thread() {
        let store = SessionStore::new();
        let base = store
            .find_or_create_shared_session("telegram", "-100123", None)
            .unwrap();
        let t1 = store
            .find_or_create_shared_session("telegram", "-100123", Some(91609))
            .unwrap();
        let t2 = store
            .find_or_create_shared_session("telegram", "-100123", Some(91610))
            .unwrap();

        // The general chat, topic 91609, and topic 91610 are three distinct
        // sessions.
        assert_ne!(base.session_id, t1.session_id);
        assert_ne!(base.session_id, t2.session_id);
        assert_ne!(t1.session_id, t2.session_id);

        // Re-fetching the same thread returns the same (stable) session.
        let t1_again = store
            .find_or_create_shared_session("telegram", "-100123", Some(91609))
            .unwrap();
        assert_eq!(t1.session_id, t1_again.session_id);
    }

    #[test]
    fn test_session_store_list() {
        let store = SessionStore::new();
        store.create_session("telegram", "a", "c1").unwrap();
        store.create_session("telegram", "b", "c2").unwrap();
        store.create_session("discord", "c", "c3").unwrap();

        assert_eq!(store.get_session_count(), 3);

        let all = store.list_active_sessions(None);
        assert_eq!(all.len(), 3);

        let telegram_only = store.list_active_sessions(Some("telegram"));
        assert_eq!(telegram_only.len(), 2);

        let discord_only = store.list_active_sessions(Some("discord"));
        assert_eq!(discord_only.len(), 1);
    }

    #[test]
    fn test_session_store_close_and_update() {
        let store = SessionStore::new();
        let session = store.create_session("test", "user", "chan").unwrap();
        let sid = session.session_id.clone();

        assert!(store.update_activity(&sid).is_ok());
        assert!(store.close_session(&sid).is_ok());
        assert!(store.get_session(&sid).is_none());
        assert_eq!(store.get_session_count(), 0);
    }

    #[test]
    fn test_session_operant_lookup() {
        let store = SessionStore::new();
        let session = store.create_session("test", "user", "chan").unwrap();
        // manually assign operant_session_id via metadata
        let _sid = session.session_id.clone();

        // get_operant_session uses the field, which is empty by default
        let found = store.get_operant_session("");
        // the operant_session_id is empty string for newly created sessions
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn test_concurrent_session_access() {
        let store = std::sync::Arc::new(SessionStore::new());
        let mut handles = Vec::new();
        for i in 0..10 {
            let s = store.clone();
            handles.push(tokio::spawn(async move {
                let session = s
                    .create_session("test", &format!("user{}", i), "chan1")
                    .unwrap();
                assert_eq!(session.platform, "test");
                let fetched = s.get_session(&session.session_id);
                assert!(fetched.is_some());
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(store.get_session_count(), 10);
    }

    // ── ChannelDirectory tests ──

    #[test]
    fn test_channel_directory_register_and_get() {
        let dir = ChannelDirectory::new();
        dir.register_channel(
            "ch1",
            "telegram",
            Some("General"),
            ChannelType::Group,
            vec!["admin1".to_string()],
        )
        .unwrap();

        let ch = dir.get_channel("ch1");
        assert!(ch.is_some());
        let ch = ch.unwrap();
        assert_eq!(ch.platform, "telegram");
        assert_eq!(ch.name, Some("General".to_string()));
        assert_eq!(ch.channel_type, ChannelType::Group);
        assert!(ch.admins.contains(&"admin1".to_string()));
    }

    #[test]
    fn test_channel_directory_duplicate_fails() {
        let dir = ChannelDirectory::new();
        dir.register_channel("ch1", "t", None, ChannelType::Direct, vec![])
            .unwrap();
        let result = dir.register_channel("ch1", "t", None, ChannelType::Direct, vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_channel_directory_list_and_remove() {
        let dir = ChannelDirectory::new();
        dir.register_channel("a", "t", None, ChannelType::Direct, vec![])
            .unwrap();
        dir.register_channel("b", "d", None, ChannelType::Group, vec![])
            .unwrap();
        dir.register_channel("c", "t", None, ChannelType::Channel, vec![])
            .unwrap();

        assert_eq!(dir.list_channels(None).len(), 3);
        assert_eq!(dir.list_channels(Some("t")).len(), 2);
        assert_eq!(dir.list_channels(Some("d")).len(), 1);

        dir.remove_channel("a").unwrap();
        assert_eq!(dir.list_channels(None).len(), 2);
    }

    #[test]
    fn test_channel_directory_is_admin() {
        let dir = ChannelDirectory::new();
        dir.register_channel(
            "ch1",
            "t",
            None,
            ChannelType::Group,
            vec!["admin1".to_string(), "admin2".to_string()],
        )
        .unwrap();

        assert!(dir.is_admin("ch1", "admin1"));
        assert!(dir.is_admin("ch1", "admin2"));
        assert!(!dir.is_admin("ch1", "user3"));
        assert!(!dir.is_admin("nonexistent", "admin1"));
    }

    // ── GatewayStats tests ──

    #[tokio::test]
    async fn test_gateway_stats() {
        let config = GatewayConfig {
            holon_notion: None,
            telegram_enabled: false,
            telegram_token: None,
            discord_enabled: false,
            discord_token: None,
            slack_enabled: false,
            slack_token: None,
            webhooks_enabled: false,
            webhooks_addr: None,
            webhooks_secret: None,
            admins: vec![],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
            whatsapp_enabled: false,
            whatsapp_token: None,
            whatsapp_phone_number_id: None,
            email_enabled: false,
            email_smtp_host: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            sms_twilio_enabled: false,
            max_concurrent_sessions: None,
        };
        let gw = Gateway::new(config);
        let stats = gw.get_stats().await;

        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.registered_channels, 0);
        assert_eq!(stats.active_adapters, 0);
        assert_eq!(stats.messages_processed, 0);
        assert!(stats.uptime_seconds < 5); // just started
    }

    // ── Admin command tests ──

    #[tokio::test]
    async fn test_admin_command_unauthorized() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "help",
            &[],
            "ch1",
            "unknown_user",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("not authorized"));
    }

    #[tokio::test]
    async fn test_admin_command_help() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "help",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("sessions"));
        assert!(result.contains("channels"));
        assert!(result.contains("broadcast"));
        assert!(result.contains("shutdown"));
    }

    #[tokio::test]
    async fn test_admin_command_sessions_empty() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "sessions",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("No active sessions"));
    }

    #[tokio::test]
    async fn test_admin_command_channels_empty() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "channels",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("No registered channels"));
    }

    #[tokio::test]
    async fn test_admin_command_unknown() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "foobar",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_admin_command_broadcast() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "broadcast",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("Broadcast"));
    }

    // ── Adapter health check tests ──

    /// The inbound table must name EXACTLY the platforms whose inbound
    /// messages reach the agent.
    ///
    /// Telegram, Discord and Webhooks receive — through their own poll
    /// loops and the webhook adapter's HTTP handler. The
    /// `PlatformAdapter::handle_update` trait method is DEAD: it has no
    /// production caller for ANY adapter. Slack, WhatsApp, Email and SMS
    /// have an inbound parser that nothing reaches, so a message sent to
    /// them is never seen by the agent — and the table must say so.
    ///
    /// The test is over the whole set, so a platform added to the gateway
    /// without a decision here fails it.
    #[test]
    fn the_inbound_table_names_exactly_the_platforms_that_receive() {
        // The platforms that receive.
        for platform in ["telegram", "discord", "webhook"] {
            assert!(
                platform_can_receive(platform),
                "{platform} receives messages and must be in the table"
            );
        }
        // The platforms that do NOT.
        for platform in ["slack", "whatsapp", "email", "sms"] {
            assert!(
                !platform_can_receive(platform),
                "{platform} is send-only — its inbound parser is never called"
            );
        }
        // An unknown platform is treated as send-only: the safe direction,
        // because claiming a platform can receive when it cannot is the
        // failure that misleads the operator.
        assert!(
            !platform_can_receive("carrier-pigeon"),
            "an unknown platform must be treated as send-only"
        );
        // The match is case-insensitive, because the platform name comes
        // from a config key and from the adapter's own name.
        assert!(platform_can_receive("Telegram"));
        assert!(platform_can_receive("TELEGRAM"));
    }

    /// The delivery ledger must record what the STREAMING path did.
    ///
    /// The plain send (`send_to_platform`) has always recorded. The
    /// streaming path (`send_message_return_id`) did NOT: so every answer
    /// the agent streamed to a platform was invisible in the delivery
    /// health the ledger exists to show, while the proactive sends around
    /// it were visible. The two paths disagreed about the same thing.
    ///
    /// The test drives the STREAMING path for both outcomes, so a ledger
    /// that recorded only successes (or only failures) fails it — and so
    /// removing the record call fails it.
    #[tokio::test]
    async fn the_delivery_ledger_records_what_each_streamed_send_did() {
        let gateway = Gateway::new(GatewayConfig::default());
        let gateway = gateway
            .with_adapter(Arc::new(SucceedingAdapter {
                name: "ok-platform".to_string(),
            }))
            .with_adapter(Arc::new(FlakyAdapter {
                name: "bad-platform".to_string(),
                sends: Arc::new(AtomicUsize::new(0)),
                error: "the platform refused it".to_string(),
            }));

        let ok = gateway
            .send_message_return_id(
                "ok-platform",
                OutgoingMessage::new("chat-1".to_string(), "hello"),
            )
            .await;
        assert!(ok.is_ok(), "the succeeding adapter must succeed");

        let bad = gateway
            .send_message_return_id(
                "bad-platform",
                OutgoingMessage::new("chat-2".to_string(), "hello"),
            )
            .await;
        assert!(bad.is_err(), "the flaky adapter must fail");

        assert_eq!(
            gateway.delivery_ledger.delivered_count().await,
            1,
            "the successful streamed send must be recorded as delivered"
        );
        assert_eq!(
            gateway.delivery_ledger.failed_count().await,
            1,
            "the failed streamed send must be recorded as failed"
        );
    }

    #[test]
    fn test_webhook_adapter_health() {
        let adapter = WebhookAdapter::new(true);
        assert!(adapter.health_check().unwrap());
    }

    #[test]
    fn test_webhook_secret_flows_from_settings_to_adapter() {
        // Regression: webhooks_secret existed in the schema and the webhook
        // handler supports HMAC verification, but the CLI never wired it —
        // a configured secret silently did nothing (unsigned webhooks accepted).
        let adapter = WebhookAdapter::new(true).with_secret(Some("s3cret".to_string()));
        assert!(adapter.config_json()["hmac_secret_configured"] == serde_json::json!(true));

        let no_secret = WebhookAdapter::new(true);
        assert!(no_secret.config_json()["hmac_secret_configured"] == serde_json::json!(false));
    }

    #[tokio::test]
    async fn test_webhook_hmac_live_server() {
        // End-to-end: boot the real axum server with a configured secret and
        // prove the verification gate actually rejects/accepts on the wire.
        // (Regression for R14-1: the secret used to be dead config — every
        // webhook was accepted unsigned.)
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        use tokio::sync::mpsc;

        // Grab a free ephemeral port.
        //
        // The probe is DROPPED before the adapter binds, and that is a
        // TOCTOU race: another test's ephemeral bind can take the port in
        // between, and the adapter then fails with `AddrInUse`. So try a
        // few ports — the retry is on the HARNESS side only.
        let (tx, mut rx) = mpsc::unbounded_channel::<IncomingMessage>();
        let mut addr = None;
        let mut last_error = None;
        for _ in 0..5 {
            let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let candidate = probe.local_addr().unwrap();
            drop(probe);
            let adapter = WebhookAdapter::new(true)
                .with_addr(candidate.to_string())
                .with_secret(Some("topsecret".to_string()));
            match adapter.start_with_channel(tx.clone()).await {
                Ok(()) => {
                    addr = Some(candidate);
                    break;
                }
                Err(e) => {
                    let in_use = e.to_string().contains("Address already in use");
                    last_error = Some(e);
                    if !in_use {
                        break;
                    }
                }
            }
        }
        let addr = addr.unwrap_or_else(|| panic!("the webhook adapter starts: {last_error:?}"));

        let url = format!("http://{}/webhook/r14", addr);
        let client = reqwest::Client::new();
        let body = r#"{"text":"hmac live test"}"#;

        // The server binds inside a spawned task — wait until it accepts.
        for _ in 0..50 {
            if client
                .get(url.replace("/webhook/r14", "/health"))
                .send()
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // 1) No signature -> 401
        let resp = client
            .post(&url)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);

        // 2) Wrong signature -> 401
        let resp = client
            .post(&url)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header("x-webhook-signature", hex::encode(vec![0u8; 32]))
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);

        // 3) Correct HMAC-SHA256(secret, body) -> 200 + message forwarded
        let mut mac = HmacSha256::new_from_slice(b"topsecret").unwrap();
        mac.update(body.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        let resp = client
            .post(&url)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header("x-webhook-signature", &expected)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // The forwarded message must carry the body as content.
        let msg = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("timed out waiting for forwarded webhook")
            .expect("channel closed");
        assert_eq!(msg.platform, "webhook");
        assert_eq!(msg.channel_id, "webhook:r14");
        assert_eq!(msg.content, "hmac live test");
    }

    #[test]
    fn test_telegram_adapter_health_check_default() {
        let adapter = TelegramAdapter::new(None);
        let healthy = adapter.health_check().unwrap();
        assert!(healthy);
    }

    // ── Startup message tests ──

    #[test]
    fn test_format_startup_message() {
        let config = GatewayConfig {
            holon_notion: None,
            telegram_enabled: true,
            telegram_token: Some("tok".to_string()),
            discord_enabled: false,
            discord_token: None,
            slack_enabled: true,
            slack_token: Some("tok".to_string()),
            webhooks_enabled: false,
            webhooks_addr: None,
            webhooks_secret: None,
            admins: vec!["admin1".to_string()],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
            whatsapp_enabled: false,
            whatsapp_token: None,
            whatsapp_phone_number_id: None,
            email_enabled: false,
            email_smtp_host: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            sms_twilio_enabled: false,
            max_concurrent_sessions: None,
        };
        let msg = format_startup_message(&config);
        assert!(msg.contains("Operant Gateway"));
        assert!(msg.contains("Telegram"));
        assert!(msg.contains("ENABLED"));
        assert!(msg.contains("DISABLED"));
        assert!(msg.contains("Admin Users: 1"));
    }

    // ── ChannelType tests ──

    #[test]
    fn test_channel_type_equality() {
        assert_eq!(ChannelType::Direct, ChannelType::Direct);
        assert_eq!(ChannelType::Group, ChannelType::Group);
        assert_ne!(ChannelType::Direct, ChannelType::Group);
    }

    // ── chunk_text tests ──

    #[test]
    fn test_chunk_text_small_message_no_split() {
        let text = "Hello, world!";
        let chunks = chunk_text(text, 4000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn test_chunk_text_exact_boundary() {
        // Text that fits exactly in one chunk (UTF-16 length == max_chunk_size)
        let text = "A".repeat(4000);
        let chunks = chunk_text(&text, 4000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(utf16_len(&chunks[0]), 4000);
    }

    #[test]
    fn test_chunk_text_multi_chunk_with_suffix_indicators() {
        let text = "A".repeat(9000);
        let chunks = chunk_text(&text, 4000);
        assert!(chunks.len() >= 2);
        // Each chunk should have (X/Y) suffix
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.contains(&format!("({}/{})", i + 1, chunks.len())),
                "chunk {} missing suffix: {}",
                i,
                chunk
            );
        }
    }

    #[test]
    fn test_chunk_text_code_block_preservation() {
        let text = format!(
            "intro text\n\n```rust\n{}\n```\n\noutro",
            "let x = 1;\n".repeat(500)
        );
        let chunks = chunk_text(&text, 4000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            let opens = chunk.matches("```").count();
            assert_eq!(opens % 2, 0, "chunk has unbalanced code fences: {}", chunk);
        }
    }

    #[test]
    fn test_chunk_text_word_boundary_splitting() {
        let text = "word ".repeat(1000);
        let chunks = chunk_text(&text, 4000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            let content = chunk.split(" (").next().unwrap_or(chunk);
            if !content.is_empty() {
                let trimmed = content.trim_end();
                assert!(
                    trimmed.ends_with("word"),
                    "chunk doesn't end at word boundary: ...{}",
                    &trimmed[trimmed.len().saturating_sub(20)..]
                );
            }
        }
    }

    #[test]
    fn test_chunk_text_utf16_emoji_handling() {
        // Emojis are 2 UTF-16 code units each
        let emoji_text = "🎉".repeat(2500); // 5000 UTF-16 code units
        let chunks = chunk_text(&emoji_text, 4000);
        assert!(chunks.len() >= 2);
        // Verify each chunk respects UTF-16 limit (accounting for suffix)
        for chunk in &chunks {
            let content = chunk.split(" (").next().unwrap_or(chunk);
            assert!(
                utf16_len(content) <= 4000,
                "chunk exceeds UTF-16 limit: {} units",
                utf16_len(content)
            );
        }
    }

    #[test]
    fn test_chunk_text_newline_split_preference() {
        let text = format!("{}\n\n{}", "line".repeat(500), "next".repeat(500));
        let chunks = chunk_text(&text, 4000);
        assert!(chunks.len() >= 2);
        // Split should prefer \n\n boundary
        let first_content = chunks[0].split(" (").next().unwrap_or(&chunks[0]);
        assert!(
            first_content.ends_with("\n\n") || first_content.ends_with('\n'),
            "split didn't use newline boundary"
        );
    }

    #[test]
    fn test_utf16_len_ascii() {
        assert_eq!(utf16_len("hello"), 5);
    }

    #[test]
    fn test_utf16_len_emoji() {
        // Most emojis are 2 UTF-16 code units
        assert_eq!(utf16_len("🎉"), 2);
        assert_eq!(utf16_len("a🎉b"), 4);
    }

    #[test]
    fn test_utf16_len_mixed() {
        // "Hello " = 6, "🎉" = 2 utf16, " World" = 6 → total 14
        assert_eq!(utf16_len("Hello 🎉 World"), 14);
    }

    // ── v6 outline §2 P1-A: the dispatch breaker seam ────────────────

    use crate::gateway::dispatch_breaker::DispatchBreaker;
    use crate::gateway::types::PlatformAdapter;
    use std::sync::atomic::AtomicUsize;

    /// RT-126: `delete_message` existed on the trait, Telegram implemented it,
    /// and no path in the gateway could reach either. This adapter records what
    /// the gateway hands it, so the test proves the CALL arrives — a test that
    /// invoked the adapter's method directly would pass with the gateway method
    /// missing.
    struct RecordingDeleteAdapter {
        name: String,
        deleted: Arc<std::sync::Mutex<Vec<(String, String)>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl PlatformAdapter for RecordingDeleteAdapter {
        fn name(&self) -> &str {
            &self.name
        }
        fn is_enabled(&self) -> bool {
            true
        }
        async fn start(&self) -> Result<()> {
            Ok(())
        }
        async fn stop(&self) -> Result<()> {
            Ok(())
        }
        async fn send_message(&self, _message: OutgoingMessage) -> Result<()> {
            Ok(())
        }
        async fn handle_update(
            &self,
            _update: serde_json::Value,
        ) -> Result<Option<IncomingMessage>> {
            Ok(None)
        }
        fn config_json(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn start_with_channel(
            &self,
            _message_tx: tokio::sync::mpsc::UnboundedSender<IncomingMessage>,
        ) -> Result<()> {
            Ok(())
        }
        async fn send_message_to_channel(
            &self,
            _channel_id: &str,
            _message: &OutgoingMessage,
        ) -> Result<String> {
            Ok(String::new())
        }
        async fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
            lock(&self.deleted).push((channel_id.to_string(), message_id.to_string()));
            if self.fail {
                return Err(Error::Agent("deleteMessage failed: HTTP 400".to_string()));
            }
            Ok(())
        }
    }

    fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[tokio::test]
    async fn the_gateway_reaches_the_adapters_delete_and_reports_its_failure() {
        let deleted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gw =
            Gateway::new(GatewayConfig::default()).with_adapter(Arc::new(RecordingDeleteAdapter {
                name: "telegram".to_string(),
                deleted: deleted.clone(),
                fail: false,
            }));

        gw.delete_message_on_platform("telegram", "42", "7")
            .await
            .expect("the adapter accepted the delete");
        assert_eq!(
            *lock(&deleted),
            vec![("42".to_string(), "7".to_string())],
            "the gateway must hand the adapter the chat AND the message id"
        );

        // A platform that cannot delete must refuse, not report success.
        let err = gw
            .delete_message_on_platform("discord", "42", "7")
            .await
            .expect_err("no discord adapter is registered");
        assert!(err.to_string().contains("Unknown platform"), "{err}");

        // An adapter's own failure reaches the caller instead of being logged
        // and swallowed (the old Telegram implementation spawned the request
        // and returned Ok before the platform answered).
        let failing =
            Gateway::new(GatewayConfig::default()).with_adapter(Arc::new(RecordingDeleteAdapter {
                name: "telegram".to_string(),
                deleted: Arc::default(),
                fail: true,
            }));
        let err = failing
            .delete_message_on_platform("telegram", "42", "7")
            .await
            .expect_err("the adapter's error must surface");
        assert!(err.to_string().contains("deleteMessage failed"), "{err}");
    }

    /// The trait's DEFAULT must refuse: a platform with no delete support
    /// reporting a successful deletion is the defect this row is about.
    #[tokio::test]
    async fn an_adapter_without_delete_support_refuses_instead_of_reporting_success() {
        let adapter = SucceedingAdapter {
            name: "email".to_string(),
        };
        let err = adapter
            .delete_message("42", "7")
            .await
            .expect_err("the default must not claim a deletion");
        assert!(
            err.to_string()
                .contains("does not support deleting messages"),
            "{err}"
        );
    }

    /// A platform adapter that counts its sends and fails every one with a
    /// fixed rendered error — the hammered channel the breaker exists for.
    struct FlakyAdapter {
        name: String,
        sends: Arc<AtomicUsize>,
        error: String,
    }

    /// An adapter whose sends always succeed.
    struct SucceedingAdapter {
        name: String,
    }

    #[async_trait::async_trait]
    impl PlatformAdapter for SucceedingAdapter {
        fn name(&self) -> &str {
            &self.name
        }
        fn is_enabled(&self) -> bool {
            true
        }
        async fn start(&self) -> Result<()> {
            Ok(())
        }
        async fn stop(&self) -> Result<()> {
            Ok(())
        }
        async fn send_message(&self, _message: OutgoingMessage) -> Result<()> {
            Ok(())
        }
        async fn send_message_to_channel(
            &self,
            _channel_id: &str,
            _message: &OutgoingMessage,
        ) -> Result<String> {
            Ok(String::new())
        }
        async fn handle_update(
            &self,
            _update: serde_json::Value,
        ) -> Result<Option<IncomingMessage>> {
            Ok(None)
        }
        fn config_json(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn start_with_channel(
            &self,
            _message_tx: tokio::sync::mpsc::UnboundedSender<IncomingMessage>,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl PlatformAdapter for FlakyAdapter {
        fn name(&self) -> &str {
            &self.name
        }
        fn is_enabled(&self) -> bool {
            true
        }
        async fn start(&self) -> Result<()> {
            Ok(())
        }
        async fn stop(&self) -> Result<()> {
            Ok(())
        }
        async fn send_message(&self, _message: OutgoingMessage) -> Result<()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Err(Error::Agent(self.error.clone()))
        }
        async fn handle_update(
            &self,
            _update: serde_json::Value,
        ) -> Result<Option<IncomingMessage>> {
            Ok(None)
        }
        fn config_json(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn start_with_channel(
            &self,
            _message_tx: tokio::sync::mpsc::UnboundedSender<IncomingMessage>,
        ) -> Result<()> {
            Ok(())
        }
        async fn send_message_to_channel(
            &self,
            _channel_id: &str,
            _message: &OutgoingMessage,
        ) -> Result<String> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Err(Error::Agent(self.error.clone()))
        }
    }

    #[tokio::test]
    async fn open_breaker_fast_fails_at_the_seam_without_touching_the_adapter() {
        let dir = tempfile::tempdir().expect("tempdir");
        let breaker =
            Arc::new(DispatchBreaker::init(dir.path().join("breaker.sqlite")).expect("breaker"));
        let sends = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(FlakyAdapter {
            name: "telegram".to_string(),
            sends: Arc::clone(&sends),
            error: "HTTP 401 Unauthorized".to_string(),
        });
        let gw = Gateway::new(GatewayConfig::default())
            .with_adapter(adapter.clone())
            .with_dispatch_breaker(Arc::clone(&breaker));

        // Three failing sends each reach the adapter and walk the breaker.
        for i in 0..3 {
            let r = gw
                .send_to_platform("telegram", OutgoingMessage::new("chat-42", format!("m{i}")))
                .await;
            assert!(r.is_err(), "the send itself still fails (i={i})");
        }
        assert_eq!(sends.load(Ordering::SeqCst), 3, "the adapter saw all three");
        assert_eq!(
            breaker
                .state_of(
                    "telegram",
                    "chat-42",
                    crate::gateway::dispatch_breaker::FailureClass::Auth
                )
                .unwrap()
                .state,
            crate::gateway::dispatch_breaker::BreakerState::Open,
            "the third consecutive auth failure opened the breaker"
        );

        // The fourth send never reaches the adapter — this is the whole
        // point of the breaker.
        let r = gw
            .send_to_platform("telegram", OutgoingMessage::new("chat-42", "m4"))
            .await;
        assert!(r.is_err());
        assert!(
            r.unwrap_err().to_string().contains("dispatch breaker open"),
            "the fast-fail names the breaker"
        );
        assert_eq!(
            sends.load(Ordering::SeqCst),
            3,
            "the adapter was never called for the fourth send"
        );
    }

    #[tokio::test]
    async fn send_message_return_id_shares_the_seam() {
        let dir = tempfile::tempdir().expect("tempdir");
        let breaker =
            Arc::new(DispatchBreaker::init(dir.path().join("breaker.sqlite")).expect("breaker"));
        let sends = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(FlakyAdapter {
            name: "telegram".to_string(),
            sends: Arc::clone(&sends),
            error: "connection refused".to_string(),
        });
        let gw = Gateway::new(GatewayConfig::default())
            .with_adapter(adapter.clone())
            .with_dispatch_breaker(Arc::clone(&breaker));

        for i in 0..3 {
            let r = gw
                .send_message_return_id("telegram", OutgoingMessage::new("chat-7", format!("m{i}")))
                .await;
            assert!(r.is_err());
        }
        assert_eq!(sends.load(Ordering::SeqCst), 3);
        let r = gw
            .send_message_return_id("telegram", OutgoingMessage::new("chat-7", "m3"))
            .await;
        assert!(r.unwrap_err().to_string().contains("dispatch breaker open"));
        assert_eq!(
            sends.load(Ordering::SeqCst),
            3,
            "the id-returning seam enforces the same breaker"
        );
    }

    #[tokio::test]
    async fn unmounted_gateway_keeps_pre_breaker_behavior() {
        let sends = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(FlakyAdapter {
            name: "telegram".to_string(),
            sends: Arc::clone(&sends),
            error: "HTTP 500".to_string(),
        });
        let gw = Gateway::new(GatewayConfig::default()).with_adapter(adapter.clone());

        // Ten failures, no breaker mounted: every send still reaches the
        // adapter — the hook is a None check, so unmounted gateways are
        // byte-for-byte the old behavior.
        for i in 0..10 {
            let r = gw
                .send_to_platform("telegram", OutgoingMessage::new("chat-9", format!("m{i}")))
                .await;
            assert!(r.is_err());
        }
        assert_eq!(sends.load(Ordering::SeqCst), 10);
    }
}
