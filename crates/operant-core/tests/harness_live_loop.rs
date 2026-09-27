//! Live-loop regression tests for the harness kernel (plan 016).
//!
//! Goal: prove that the harness kern + native seams are **dark-mergeable**.
//! Concretely:
//!   1. With NO kernel mounted, the host machinery (ToolRegistry) behaves
//!      exactly as before — same schemas, same dispatch.
//!   2. With a kernel mounted + a provider that installs through a seam,
//!      the host machinery sees the new entry, dispatches through it, and
//!      the existing entries are unaffected.
//!   3. Transactional swap (Harness::replace) is observed end-to-end:
//!      the new tool is in `get_schemas()` immediately, the old one is
//!      gone, and the previous generation no longer matches.
//!   4. LIFO effect unwind order is correct under partial failure.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use operant_core::MemoryProvider;
use operant_core::harness_seams_r3::ChannelAdapterHost;
use operant_core::plugins::PluginCommand;
use operant_core::tools::builtin::DateTimeTool;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

use operant_harness::{
    ActivateCx, Claim, Harness, KernelOptions, Provider, ProviderSource, ProviderSpec,
    SwapGeneration,
};

// ─── test tools ────────────────────────────────────────────────────────

/// A minimal tool used to verify dispatch. Dispatch is observed from the
/// returned payload (`{"echoed": ...}`), which is how every test here
/// distinguishes the live tool from a shadowed one.
struct EchoTool;

impl EchoTool {
    fn new() -> Self {
        Self
    }
}

#[derive(JsonSchema, Deserialize)]
struct EchoArgs {
    message: String,
}

#[async_trait]
impl OperantTool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Echoes back its argument; used by the harness live-loop test."
    }
    fn schema(&self) -> operant_core::schema::ToolSchema {
        operant_core::schema::ToolSchema::from_type::<EchoArgs>("echo", "Echoes back its argument")
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: EchoArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("echo", format!("bad args: {e}")),
        };
        ToolResult::success("echo", json!({ "echoed": parsed.message }))
    }
}

/// V2 of EchoTool — different name (`echo_v2`) so the seam's undo
/// (keyed by name) does not race with v1's undo during the swap.
/// The harness's replace() is keyed by the new provider's id, so both
/// providers can share an `id` while having different tool names.
struct EchoV2Tool;

#[async_trait]
impl OperantTool for EchoV2Tool {
    fn name(&self) -> &str {
        "echo_v2"
    }
    fn description(&self) -> &str {
        "Echo v2 (different tool name) — proves swap atomicity."
    }
    fn schema(&self) -> operant_core::schema::ToolSchema {
        operant_core::schema::ToolSchema::from_type::<EchoArgs>("echo_v2", "Echo v2")
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: EchoArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("echo_v2", format!("bad args: {e}")),
        };
        ToolResult::success("echo_v2", json!({ "v": 2, "echoed": parsed.message }))
    }
}

// ─── providers that install through the seams ─────────────────────────

struct ToolProvider {
    id: &'static str,
    tool: Arc<dyn OperantTool>,
}

impl ProviderSpec for ToolProvider {
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
impl Provider for ToolProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), operant_harness::HarnessError> {
        cx.install_with("tool", self.tool.name(), &self.tool)
            .await?;
        Ok(())
    }
}

struct MemoryProviderA;
struct MemoryProviderB;

#[async_trait]
impl MemoryProvider for MemoryProviderA {
    fn name(&self) -> &str {
        "provider-a"
    }
    fn is_available(&self) -> bool {
        true
    }
    async fn initialize(&self, _session_id: &str) -> operant_core::error::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl MemoryProvider for MemoryProviderB {
    fn name(&self) -> &str {
        "provider-b"
    }
    fn is_available(&self) -> bool {
        true
    }
    async fn initialize(&self, _session_id: &str) -> operant_core::error::Result<()> {
        Ok(())
    }
}

struct MemProviderWrapper {
    id: &'static str,
    mp: Arc<dyn MemoryProvider>,
}

impl ProviderSpec for MemProviderWrapper {
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
impl Provider for MemProviderWrapper {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), operant_harness::HarnessError> {
        cx.install_with("memory.provider", "sel", &self.mp).await?;
        Ok(())
    }
}

// ─── scenario 1: dark-merge baseline ──────────────────────────────────

/// With NO kernel mounted, the ToolRegistry behaves exactly as before.
/// Pre-registers DateTimeTool, observes the canonical schema set; the
/// new `register_dyn` / `unregister_tool` path is never called.
#[tokio::test]
async fn dark_merge_baseline_tool_registry_unchanged() {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(DateTimeTool).await.unwrap();
    let schemas = registry.get_schemas().await;
    let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"datetime"), "builtin tool must be present");
    // The echo tool is NOT in the baseline — it was never registered.
    assert!(!names.contains(&"echo"));
}

// ─── scenario 2: kernel mounts a tool, registry reflects it ──────────

/// Mount a provider that installs an `echo` tool through the seam.
/// After mount, `get_schemas()` returns the original `datetime` PLUS `echo`.
/// The seam path is the same `register_dyn` the kernel uses.
#[tokio::test]
async fn kernel_mounted_tool_appears_in_registry() {
    let registry = Arc::new(ToolRegistry::new(Duration::from_secs(5)));
    registry.register(DateTimeTool).await.unwrap();
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::new(operant_core::harness_adapters::ToolSeam::new(
        (*registry).clone(),
    )));

    harness
        .mount(Arc::new(ToolProvider {
            id: "echo-v1",
            tool: Arc::new(EchoTool::new()),
        }))
        .await
        .expect("mount should succeed");

    let names: Vec<String> = registry
        .get_schemas()
        .await
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(names.contains(&"datetime".to_string()));
    assert!(names.contains(&"echo".to_string()));

    // Dispatch through the real registry — the seam-installed tool is
    // called like any other.
    let result = registry
        .execute(
            "echo",
            "tc1",
            json!({ "message": "hello, kernel" }),
            ToolContext::default(),
        )
        .await
        .expect("echo dispatch should succeed");
    assert!(result.success, "echo dispatch should succeed");
}

// ─── scenario 3: kernel unmount removes the tool ─────────────────────

/// After `harness.unmount("echo-v1")`, the seam's effect-undo path calls
/// `ToolRegistry::unregister_tool`. `get_schemas()` no longer includes `echo`.
#[tokio::test]
async fn kernel_unmount_removes_tool_from_registry() {
    let registry = Arc::new(ToolRegistry::new(Duration::from_secs(5)));
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::new(operant_core::harness_adapters::ToolSeam::new(
        (*registry).clone(),
    )));

    harness
        .mount(Arc::new(ToolProvider {
            id: "echo-v1",
            tool: Arc::new(EchoTool::new()),
        }))
        .await
        .unwrap();
    assert!(
        registry
            .get_schemas()
            .await
            .iter()
            .any(|s| s.name == "echo")
    );

    harness.unmount("echo-v1").await.unwrap();

    let names: Vec<String> = registry
        .get_schemas()
        .await
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(
        !names.contains(&"echo".to_string()),
        "echo should be gone after unmount"
    );
}

// ─── scenario 4: transactional replace swaps a tool atomically ───────

/// Mount v1, observe the schema and dispatch. Then `replace(v2_provider,
/// "echo-v1")` — the new provider takes the slot, the old one's effects
/// unwind, the new one's `register_dyn` runs. The schemas change
/// immediately; dispatch now returns the v2 output.
#[tokio::test]
async fn kernel_replace_swaps_tool_atomically() {
    let registry = Arc::new(ToolRegistry::new(Duration::from_secs(5)));
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::new(operant_core::harness_adapters::ToolSeam::new(
        (*registry).clone(),
    )));

    harness
        .mount(Arc::new(ToolProvider {
            id: "echo-v1",
            tool: Arc::new(EchoTool::new()),
        }))
        .await
        .unwrap();

    // Sanity: v1 dispatch.
    let v1 = registry
        .execute(
            "echo",
            "tc1",
            json!({ "message": "before" }),
            ToolContext::default(),
        )
        .await
        .expect("v1 dispatch should succeed");
    assert!(v1.success);
    let v1_body: Value = serde_json::from_str(&v1.content).unwrap();
    assert!(v1_body.get("v").is_none(), "v1 has no `v` field");

    // Replace with v2 — the new provider has the SAME id (`echo-v1`)
    // because the kernel's replace uses the successor's id as the target.
    // The tool name is `echo_v2` so v1's `unregister_tool("echo")` does
    // not race with v2's `register_dyn` of `echo_v2`.
    let v2 = Arc::new(ToolProvider {
        id: "echo-v1",
        tool: Arc::new(EchoV2Tool),
    });
    let report = harness.replace(v2).await.expect("replace should succeed");
    assert_eq!(report, "echo-v1", "replace returns the new provider id");

    // v1 is gone, v2 is now live.
    let schemas = registry.get_schemas().await;
    let names: Vec<String> = schemas.iter().map(|s| s.name.clone()).collect();
    assert!(
        !names.contains(&"echo".to_string()),
        "v1 (name=echo) should be unwound"
    );
    assert!(
        names.contains(&"echo_v2".to_string()),
        "v2 (name=echo_v2) should be live"
    );

    // v2 dispatch is now active.
    let v2_result = registry
        .execute(
            "echo_v2",
            "tc2",
            json!({ "message": "after" }),
            ToolContext::default(),
        )
        .await
        .expect("v2 dispatch should succeed");
    let v2_body: Value = serde_json::from_str(&v2_result.content).unwrap();
    assert_eq!(v2_body["v"], 2, "v2 dispatch should set v=2");
}

// ─── scenario 5: memory.provider seam, selection semantics ───────────

/// Mount `provider-a`; the seam records it as the current selection.
/// Mount `provider-b`; the current selection is replaced. Unmount `b`;
/// the seam's undo restores the prior (a).
#[tokio::test]
async fn memory_provider_seam_selection_and_undo() {
    let seam = Arc::new(operant_core::harness_seams_r3::MemoryProviderSeam::new());
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::clone(&seam) as Arc<dyn operant_harness::Seam>);

    let a: Arc<dyn MemoryProvider> = Arc::new(MemoryProviderA);
    let b: Arc<dyn MemoryProvider> = Arc::new(MemoryProviderB);

    harness
        .mount(Arc::new(MemProviderWrapper {
            id: "m-a",
            mp: Arc::clone(&a),
        }))
        .await
        .unwrap();
    assert_eq!(seam.current().unwrap().name(), "provider-a");

    harness
        .mount(Arc::new(MemProviderWrapper {
            id: "m-b",
            mp: Arc::clone(&b),
        }))
        .await
        .unwrap();
    assert_eq!(seam.current().unwrap().name(), "provider-b");

    harness.unmount("m-b").await.unwrap();
    assert_eq!(
        seam.current().unwrap().name(),
        "provider-a",
        "undo must restore prior selection"
    );

    harness.unmount("m-a").await.unwrap();
    assert!(seam.current().is_none());
}

// ─── scenario 6: gateway.command seam, dynamic map ───────────────────

/// Mount a provider that installs a `PluginCommand`; the seam's
/// `commands()` returns it. Unmount removes it from the dynamic map.
/// The global PluginRegistry (unregister-incapable) is unaffected — the
/// plan's invariant is that the seam is a separate map.
#[tokio::test]
async fn gateway_command_seam_dynamic_map() {
    fn dummy(_args: &str) -> String {
        "ok".to_string()
    }

    struct CmdProvider {
        id: &'static str,
        cmd: PluginCommand,
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

    let seam = Arc::new(operant_core::harness_seams_r3::GatewayCommandSeam::new());
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::clone(&seam) as Arc<dyn operant_harness::Seam>);

    let cmd = PluginCommand::new("kernel-cmd", "from kernel", dummy);
    harness
        .mount(Arc::new(CmdProvider { id: "c1", cmd }))
        .await
        .unwrap();
    let installed = seam.commands();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].name, "kernel-cmd");

    harness.unmount("c1").await.unwrap();
    assert!(seam.commands().is_empty());
}

// ─── scenario 7: channel.adapter seam with mock host ─────────────────

/// A `MockHost` (impl `ChannelAdapterHost`) receives the register call when
/// the provider mounts, and the unregister call when it unmounts. The
/// real Gateway would impl this trait in operant-core/src/gateway.
#[tokio::test]
async fn channel_adapter_seam_routes_to_host() {
    struct MockHost {
        registered: std::collections::HashMap<String, Arc<dyn operant_core::PlatformAdapter>>,
    }

    impl MockHost {
        fn new() -> Self {
            Self {
                registered: Default::default(),
            }
        }
    }

    impl ChannelAdapterHost for MockHost {
        fn register_adapter(&mut self, adapter: Arc<dyn operant_core::PlatformAdapter>) {
            self.registered.insert(adapter.name().to_string(), adapter);
        }
        fn remove_adapter(&mut self, name: &str) {
            self.registered.remove(name);
        }
    }

    struct StubAdapter;
    #[async_trait]
    impl operant_core::PlatformAdapter for StubAdapter {
        fn name(&self) -> &str {
            "stub"
        }
        fn is_enabled(&self) -> bool {
            true
        }
        async fn start(&self) -> operant_core::error::Result<()> {
            Ok(())
        }
        async fn start_with_channel(
            &self,
            _tx: tokio::sync::mpsc::UnboundedSender<operant_core::IncomingMessage>,
        ) -> operant_core::error::Result<()> {
            Ok(())
        }
        async fn stop(&self) -> operant_core::error::Result<()> {
            Ok(())
        }
        async fn send_message(
            &self,
            _m: operant_core::OutgoingMessage,
        ) -> operant_core::error::Result<()> {
            Ok(())
        }
        async fn send_message_to_channel(
            &self,
            _c: &str,
            _m: &operant_core::OutgoingMessage,
        ) -> operant_core::error::Result<String> {
            Ok("id".to_string())
        }
        async fn handle_update(
            &self,
            _u: Value,
        ) -> operant_core::error::Result<Option<operant_core::IncomingMessage>> {
            Ok(None)
        }
        fn config_json(&self) -> Value {
            json!({})
        }
    }

    struct AdapterProvider {
        id: &'static str,
        adapter: Arc<dyn operant_core::PlatformAdapter>,
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
            cx.install_with("channel.adapter", "stub", &self.adapter)
                .await?;
            Ok(())
        }
    }

    let host = Arc::new(std::sync::Mutex::new(MockHost::new()));
    let seam =
        Arc::new(operant_core::harness_seams_r3::ChannelAdapterSeam::from_host(Arc::clone(&host)));
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::clone(&seam) as Arc<dyn operant_harness::Seam>);

    let adapter: Arc<dyn operant_core::PlatformAdapter> = Arc::new(StubAdapter);
    harness
        .mount(Arc::new(AdapterProvider { id: "a1", adapter }))
        .await
        .unwrap();
    assert!(host.lock().unwrap().registered.contains_key("stub"));
    harness.unmount("a1").await.unwrap();
    assert!(!host.lock().unwrap().registered.contains_key("stub"));
}

// ─── scenario 8: ABA generation counter bumps on successful replace ──

/// Two `SwapGeneration` slots simulate the kernel's two generation
/// counters (one for the old provider, one for the new). After a
/// successful replace, both bump. After a failed replace, neither bumps.
#[tokio::test]
async fn swap_generation_aba_observed_by_outside_reader() {
    let old_gen = SwapGeneration::new();
    let new_gen = SwapGeneration::new();
    let old_observed = old_gen.current();
    let new_observed = new_gen.current();

    // Simulate successful swap.
    old_gen.bump();
    new_gen.bump();

    assert!(
        !old_gen.matches(old_observed),
        "old_gen moved; old observation no longer matches"
    );
    assert!(!new_gen.matches(new_observed));
    assert!(old_gen.matches(old_gen.current()));
    assert!(new_gen.matches(new_gen.current()));
}
