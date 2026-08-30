//! Harness-kernel seam adapters for memory provider, gateway command, and
//! channel adapter (plan 016 Phase 2 r3). Family-level seams — the kernel
//! handles individual family members; a single family provider installs a
//! whole family in one activate. Flag-off boots never construct these.
//!
//! Each seam wraps one existing mutator on the host side. install() routes
//! the kernel payload into the host; effect-undo calls the inverse.

use std::sync::Arc;

use async_trait::async_trait;
use std::sync::Mutex as PMutex;
use tracing::info;

use operant_harness::{Effect, HarnessError, Registration, Seam};

use crate::gateway::PlatformAdapter;
use crate::memory_provider::MemoryProvider;

// ─── memory.provider seam ──────────────────────────────────────────────

/// Family-level wrapper over the host's currently-selected [`MemoryProvider`].
///
/// The active provider is a single `Arc<dyn MemoryProvider>` (selection, not
/// accumulation). install() replaces it; effect-undo restores the prior one.
/// Payload type: `Arc<dyn MemoryProvider>`.
pub struct MemoryProviderSeam {
    current: Arc<PMutex<Option<Arc<dyn MemoryProvider>>>>,
}

impl MemoryProviderSeam {
    pub fn new() -> Self {
        Self {
            current: Arc::new(PMutex::new(None)),
        }
    }

    /// The current selection (None when no provider is mounted).
    pub fn current(&self) -> Option<Arc<dyn MemoryProvider>> {
        self.current.lock().expect("memory seam current").clone()
    }
}

impl Default for MemoryProviderSeam {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Seam for MemoryProviderSeam {
    fn name(&self) -> &str {
        "memory.provider"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let provider: Arc<dyn MemoryProvider> = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn MemoryProvider>>())
            .cloned()
            .ok_or_else(|| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: "memory.provider seam install requires an Arc<dyn MemoryProvider>"
                    .to_string(),
            })?;

        let prior = self.current.lock().expect("memory seam current").clone();
        *self.current.lock().expect("memory seam current") = Some(Arc::clone(&provider));
        info!(provider = %provider.name(), "memory.provider seam: selected");

        let current = Arc::clone(&self.current);
        let label = format!("memory.provider:{}", provider.name());
        Ok(Effect::new(label, move || {
            Box::pin(async move {
                *current.lock().expect("memory seam current") = prior;
            })
        }))
    }
}

// ─── channel.adapter seam ──────────────────────────────────────────────

/// Trait the host gateway implements so the seam can call into it without
/// depending on the full Gateway type.
pub trait ChannelAdapterHost: Send {
    fn register_adapter(&mut self, adapter: Arc<dyn PlatformAdapter>);
    fn remove_adapter(&mut self, name: &str);
}

/// Wraps a host gateway. install() routes `Arc<dyn PlatformAdapter>` into the
/// host; effect-undo calls the inverse. The host callable is held as
/// `Arc<dyn Fn>` so closures can be cloned into Effect undo handles.
pub struct ChannelAdapterSeam {
    register: Arc<dyn Fn(Arc<dyn PlatformAdapter>) + Send + Sync>,
    unregister: Arc<dyn Fn(&str) + Send + Sync>,
}

impl std::fmt::Debug for ChannelAdapterSeam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelAdapterSeam").finish_non_exhaustive()
    }
}

impl ChannelAdapterSeam {
    /// Build from any `ChannelAdapterHost` (the real `Gateway` implements it).
    pub fn from_host<G>(gateway: Arc<PMutex<G>>) -> Self
    where
        G: ChannelAdapterHost + 'static,
    {
        let gw_reg = Arc::clone(&gateway);
        let gw_unreg = Arc::clone(&gateway);
        let register: Arc<dyn Fn(Arc<dyn PlatformAdapter>) + Send + Sync> = Arc::new(move |a| {
            gw_reg
                .lock()
                .expect("channel adapter host")
                .register_adapter(a);
        });
        let unregister: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |name| {
            gw_unreg
                .lock()
                .expect("channel adapter host")
                .remove_adapter(name);
        });
        Self {
            register,
            unregister,
        }
    }
}

#[async_trait]
impl Seam for ChannelAdapterSeam {
    fn name(&self) -> &str {
        "channel.adapter"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let adapter: Arc<dyn PlatformAdapter> = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn PlatformAdapter>>())
            .cloned()
            .ok_or_else(|| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: "channel.adapter seam install requires an Arc<dyn PlatformAdapter>"
                    .to_string(),
            })?;

        let name = adapter.name().to_string();
        (self.register)(Arc::clone(&adapter));
        info!(adapter = %name, "channel.adapter seam: registered");

        let unregister = Arc::clone(&self.unregister);
        let name_for_undo = name.clone();
        let label = format!("channel.adapter:{name}");
        Ok(Effect::new(label, move || {
            Box::pin(async move {
                unregister(&name_for_undo);
            })
        }))
    }
}

// ─── gateway.command seam ──────────────────────────────────────────────

/// Wraps a per-seam dynamic command map. Plan: "fn-pointer compat kept;
/// seam routes NEW dynamic commands beside it" — the global plugin registry
/// is not exposed for unregister, so the seam keeps its own map of dynamic
/// commands. Existing global commands stay registered regardless of seam
/// state.
pub struct GatewayCommandSeam {
    /// Dynamic commands installed via the kernel. (Name, PluginCommand).
    commands: Arc<PMutex<std::collections::HashMap<String, crate::plugins::PluginCommand>>>,
}

impl GatewayCommandSeam {
    pub fn new() -> Self {
        Self {
            commands: Arc::new(PMutex::new(Default::default())),
        }
    }

    /// Snapshot of dynamic commands installed via this seam.
    pub fn commands(&self) -> Vec<crate::plugins::PluginCommand> {
        self.commands
            .lock()
            .expect("command seam")
            .values()
            .cloned()
            .collect()
    }
}

impl Default for GatewayCommandSeam {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Seam for GatewayCommandSeam {
    fn name(&self) -> &str {
        "gateway.command"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let cmd: crate::plugins::PluginCommand = reg
            .payload
            .and_then(|p| p.downcast_ref::<crate::plugins::PluginCommand>())
            .cloned()
            .ok_or_else(|| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: "gateway.command seam install requires a PluginCommand payload"
                    .to_string(),
            })?;

        let name = cmd.name.clone();
        self.commands
            .lock()
            .expect("command seam")
            .insert(name.clone(), cmd);
        info!(command = %name, "gateway.command seam: installed");

        let commands = Arc::clone(&self.commands);
        let label = format!("gateway.command:{name}");
        Ok(Effect::new(label, move || {
            Box::pin(async move {
                commands.lock().expect("command seam").remove(&name);
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;

    use operant_harness::{
        ActivateCx, Claim, Harness, KernelOptions, Provider, ProviderSource, ProviderSpec,
    };

    use super::*;

    // ── memory.provider seam ────────────────────────────────────────────

    struct Mp(&'static str);

    #[async_trait]
    impl MemoryProvider for Mp {
        fn name(&self) -> &str {
            self.0
        }
        fn is_available(&self) -> bool {
            true
        }
        async fn initialize(&self, _session_id: &str) -> crate::error::Result<()> {
            Ok(())
        }
    }

    struct MemProvider {
        id: &'static str,
        mp: Arc<dyn MemoryProvider>,
    }

    impl ProviderSpec for MemProvider {
        fn id(&self) -> &str {
            self.id
        }
        fn source(&self) -> ProviderSource {
            ProviderSource::Native
        }
        fn provides(&self) -> &[Claim] {
            &[]
        }
        fn requires(&self) -> &[Claim] {
            &[]
        }
    }

    #[async_trait]
    impl Provider for MemProvider {
        fn spec(&self) -> &dyn ProviderSpec {
            self
        }
        async fn activate(
            &self,
            cx: &mut ActivateCx<'_>,
        ) -> Result<(), operant_harness::HarnessError> {
            cx.install_with("memory.provider", "sel", &self.mp).await?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn memory_provider_seam_install_and_undo_via_kernel() {
        let seam = Arc::new(MemoryProviderSeam::new());
        let mut h = Harness::new(KernelOptions { audit: false });
        h.add_seam(Arc::clone(&seam) as Arc<dyn operant_harness::Seam>);
        let mp: Arc<dyn MemoryProvider> = Arc::new(Mp("alpha"));
        h.mount(Arc::new(MemProvider {
            id: "p1",
            mp: Arc::clone(&mp),
        }))
        .await
        .unwrap();
        assert_eq!(seam.current().unwrap().name(), "alpha");
        h.unmount("p1").await.unwrap();
        assert!(seam.current().is_none());
    }

    // ── gateway.command seam ─────────────────────────────────────────────

    fn dummy_handler(_args: &str) -> String {
        "ok".to_string()
    }

    struct CmdProvider {
        id: &'static str,
        cmd: crate::plugins::PluginCommand,
    }

    impl ProviderSpec for CmdProvider {
        fn id(&self) -> &str {
            self.id
        }
        fn source(&self) -> ProviderSource {
            ProviderSource::Native
        }
        fn provides(&self) -> &[Claim] {
            &[]
        }
        fn requires(&self) -> &[Claim] {
            &[]
        }
    }

    #[async_trait]
    impl Provider for CmdProvider {
        fn spec(&self) -> &dyn ProviderSpec {
            self
        }
        async fn activate(
            &self,
            cx: &mut ActivateCx<'_>,
        ) -> Result<(), operant_harness::HarnessError> {
            cx.install_with("gateway.command", &self.cmd.name, &self.cmd)
                .await?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn gateway_command_seam_add_and_remove_via_kernel() {
        let seam = Arc::new(GatewayCommandSeam::new());
        let mut h = Harness::new(KernelOptions { audit: false });
        h.add_seam(Arc::clone(&seam) as Arc<dyn operant_harness::Seam>);
        let cmd = crate::plugins::PluginCommand::new("kernel-cmd", "from kernel", dummy_handler);
        h.mount(Arc::new(CmdProvider { id: "c1", cmd }))
            .await
            .unwrap();
        let installed = seam.commands();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "kernel-cmd");
        h.unmount("c1").await.unwrap();
        assert!(seam.commands().is_empty());
    }

    // ── channel.adapter seam (with a mock host) ──────────────────────────

    struct MockHost {
        registered: std::collections::HashMap<String, Arc<dyn PlatformAdapter>>,
    }

    impl MockHost {
        fn new() -> Self {
            Self {
                registered: Default::default(),
            }
        }
    }

    impl ChannelAdapterHost for MockHost {
        fn register_adapter(&mut self, adapter: Arc<dyn PlatformAdapter>) {
            self.registered.insert(adapter.name().to_string(), adapter);
        }
        fn remove_adapter(&mut self, name: &str) {
            self.registered.remove(name);
        }
    }

    struct TestAdapter;

    #[async_trait]
    impl PlatformAdapter for TestAdapter {
        fn name(&self) -> &str {
            "test"
        }
        fn is_enabled(&self) -> bool {
            true
        }
        async fn start(&self) -> crate::error::Result<()> {
            Ok(())
        }
        async fn start_with_channel(
            &self,
            _message_tx: tokio::sync::mpsc::UnboundedSender<crate::gateway::IncomingMessage>,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        async fn stop(&self) -> crate::error::Result<()> {
            Ok(())
        }
        async fn send_message(
            &self,
            _msg: crate::gateway::OutgoingMessage,
        ) -> crate::error::Result<()> {
            Ok(())
        }
        async fn send_message_to_channel(
            &self,
            _channel_id: &str,
            _message: &crate::gateway::OutgoingMessage,
        ) -> crate::error::Result<String> {
            Ok("msg-id".to_string())
        }
        async fn handle_update(
            &self,
            _update: serde_json::Value,
        ) -> crate::error::Result<Option<crate::gateway::IncomingMessage>> {
            Ok(None)
        }
        fn config_json(&self) -> serde_json::Value {
            serde_json::json!({})
        }
    }

    struct AdapterProvider {
        id: &'static str,
        adapter: Arc<dyn PlatformAdapter>,
    }

    impl ProviderSpec for AdapterProvider {
        fn id(&self) -> &str {
            self.id
        }
        fn source(&self) -> ProviderSource {
            ProviderSource::Native
        }
        fn provides(&self) -> &[Claim] {
            &[]
        }
        fn requires(&self) -> &[Claim] {
            &[]
        }
    }

    #[async_trait]
    impl Provider for AdapterProvider {
        fn spec(&self) -> &dyn ProviderSpec {
            self
        }
        async fn activate(
            &self,
            cx: &mut ActivateCx<'_>,
        ) -> Result<(), operant_harness::HarnessError> {
            cx.install_with("channel.adapter", "test", &self.adapter)
                .await?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn channel_adapter_seam_add_and_remove_via_kernel() {
        let host = Arc::new(PMutex::new(MockHost::new()));
        let seam = Arc::new(ChannelAdapterSeam::from_host(Arc::clone(&host)));
        let mut h = Harness::new(KernelOptions { audit: false });
        h.add_seam(Arc::clone(&seam) as Arc<dyn operant_harness::Seam>);
        let adapter: Arc<dyn PlatformAdapter> = Arc::new(TestAdapter);
        h.mount(Arc::new(AdapterProvider { id: "a1", adapter }))
            .await
            .unwrap();
        assert!(
            host.lock()
                .expect("channel adapter host")
                .registered
                .contains_key("test")
        );
        h.unmount("a1").await.unwrap();
        assert!(
            !host
                .lock()
                .expect("channel adapter host")
                .registered
                .contains_key("test")
        );
    }

    // ── all three seams compose under a single Harness ──────────────────

    #[tokio::test]
    async fn three_seams_compose_under_one_harness() {
        let host = Arc::new(PMutex::new(MockHost::new()));
        let mut h = Harness::new(KernelOptions { audit: false });
        h.add_seam(Arc::new(MemoryProviderSeam::new()));
        h.add_seam(Arc::new(ChannelAdapterSeam::from_host(Arc::clone(&host))));
        h.add_seam(Arc::new(GatewayCommandSeam::new()));
        drop(h);
    }
}
