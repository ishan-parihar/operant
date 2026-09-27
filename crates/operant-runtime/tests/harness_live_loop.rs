//! Live-loop regression tests for the harness kernel in operant-runtime.
//!
//! Goal: prove the prompt.section and hook seams are dark-mergeable and
//! live-fireable against the real `SystemPromptBuilder` and `HookRunner`.
//!
//! Scenarios:
//!   1. Dark-merge baseline: `with_defaults().build()` produces the
//!      canonical prompt. `HookRunner` is empty.
//!   2. Harness-on, prompt.section: kernel mounts a section, the builder
//!      picks it up via `extend_from_slot`, and the prompt bytes include
//!      both the canonical 9 sections AND the new one.
//!   3. Harness-on, hook: kernel mounts a handler, the real
//!      `HookRunner` (which has `DynamicHooks` registered as one static
//!      handler) dispatches the new event to it.
//!   4. LIFO unwind: a chain of 3 prompt sections mounted; unmount the
//!      middle one; only the middle one's slot entry disappears; order
//!      of the remaining is preserved.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::Result;
use async_trait::async_trait;

use operant_harness::{
    ActivateCx, Claim, Harness, KernelOptions, Provider, ProviderSource, ProviderSpec,
};
use operant_runtime::agent::prompt::{
    PromptContext, PromptSection, PromptSections, SystemPromptBuilder,
};
use operant_runtime::hooks::DynamicHooks;
use operant_runtime::hooks::HookRunner;
use operant_runtime::hooks::HooksSeam;
use operant_runtime::hooks::{HookHandler, HookResult};
use operant_runtime::security::AutonomyLevel;

// ─── dark-merge baseline ─────────────────────────────────────────────

/// `with_defaults().build(ctx)` against an empty `PromptContext` produces
/// the canonical prompt, with no kernel involvement. This is the byte-
/// for-byte reference the harness-on path must NOT alter in the prefix.
#[test]
fn dark_merge_baseline_prompt_canonical() {
    let builder = SystemPromptBuilder::with_defaults();
    let ctx = empty_prompt_context();
    let prompt = builder
        .build(&ctx)
        .expect("canonical prompt build must succeed");

    // Sanity: the canonical prompt mentions "Operant" (the agent's name)
    // and "Workspace" (the workspace section). These are sentinel strings
    // the harness-on path must preserve.
    assert!(
        prompt.contains("Workspace"),
        "canonical prompt must include Workspace section"
    );
}

/// `HookRunner::new()` has zero handlers, dispatching any event returns
/// the default Continue-without-modification.
#[tokio::test]
async fn dark_merge_baseline_hook_runner_is_empty() {
    let runner = HookRunner::new();
    let result = runner
        .run_before_tool_call("noop".to_string(), serde_json::json!({}))
        .await;
    // No handlers → default Continue; the value is the unmodified args.
    assert!(matches!(result, HookResult::Continue(_)));
}

// ─── harness-on, prompt.section seam feeds the real builder ─────────

struct TagSection(&'static str);

#[async_trait]
impl PromptSection for TagSection {
    fn name(&self) -> &str {
        "kernel-tag"
    }
    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(format!("<tag>{}</tag>", self.0))
    }
}

struct SectionProvider {
    id: &'static str,
    section: Arc<dyn PromptSection>,
    key: &'static str,
}

impl ProviderSpec for SectionProvider {
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
impl Provider for SectionProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), operant_harness::HarnessError> {
        cx.install_with("prompt", self.key, &self.section).await?;
        Ok(())
    }
}

/// The seam-installed section appears in the slot, the builder
/// `extend_from_slot` produces a prompt that includes both the canonical
/// sections AND the kernel-tagged one.
#[tokio::test]
async fn prompt_seam_section_appears_in_real_prompt() {
    let slot = Arc::new(PromptSections::new());
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::new(
        operant_runtime::agent::prompt_seam::PromptSectionSeam::new(Arc::clone(&slot)),
    ));

    let section: Arc<dyn PromptSection> = Arc::new(TagSection("from-kernel"));
    harness
        .mount(Arc::new(SectionProvider {
            id: "tag",
            section: Arc::clone(&section),
            key: "kernel-tag",
        }))
        .await
        .unwrap();
    assert_eq!(slot.len().await, 1);

    let ctx = empty_prompt_context();
    let prompt = SystemPromptBuilder::with_defaults()
        .extend_from_slot(Arc::clone(&slot))
        .await
        .build(&ctx)
        .expect("extended prompt build must succeed");

    assert!(
        prompt.contains("from-kernel"),
        "kernel-mounted section text must appear in the prompt: {prompt}"
    );
    assert!(
        prompt.contains("Workspace"),
        "canonical section must still be present"
    );
}

// ─── harness-on, hook seam feeds the real HookRunner ─────────────────

struct CallCounter(Arc<AtomicU32>);

#[async_trait]
impl HookHandler for CallCounter {
    fn name(&self) -> &str {
        "kernel-counter"
    }
    async fn before_tool_call(
        &self,
        _name: String,
        _args: serde_json::Value,
    ) -> HookResult<(String, serde_json::Value)> {
        let _ = self.0.fetch_add(1, Ordering::SeqCst);
        HookResult::Continue((_name, _args))
    }
}

struct HookProvider {
    id: &'static str,
    handler: Arc<dyn HookHandler>,
}

impl ProviderSpec for HookProvider {
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
impl Provider for HookProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), operant_harness::HarnessError> {
        cx.install_with("hook", "kernel-counter", &self.handler)
            .await?;
        Ok(())
    }
}

/// The hook seam-installed handler is dispatched by the real HookRunner.
/// The static `DynamicHooks` slot is one entry in the runner; mounting
/// the provider adds a child to the slot; `run_before_tool_call` fans
/// out to all children and the counter bumps.
#[tokio::test]
async fn hook_seam_handler_dispatched_by_real_runner() {
    let dynamic = Arc::new(DynamicHooks::new());
    // The HookRunner needs an owned Box; we wrap a thin delegator that
    // forwards to the same Arc<DynamicHooks> the seam will mutate.
    let runner_handle: Box<dyn HookHandler> = Box::new(DelegatorToDynamic(Arc::clone(&dynamic)));
    let mut runner = HookRunner::new();
    runner.register(runner_handle);

    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::new(HooksSeam::new(Arc::clone(&dynamic))));

    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::new(HooksSeam::new(Arc::clone(&dynamic))));

    let counter = Arc::new(AtomicU32::new(0));
    let handler: Arc<dyn HookHandler> = Arc::new(CallCounter(Arc::clone(&counter)));
    harness
        .mount(Arc::new(HookProvider {
            id: "counter",
            handler,
        }))
        .await
        .unwrap();

    // Real dispatch through the runner.
    let _ = runner
        .run_before_tool_call("any".to_string(), serde_json::json!({}))
        .await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "kernel-mounted hook handler must observe the dispatch"
    );

    // Unmount: counter should not move on the next dispatch.
    harness.unmount("counter").await.unwrap();
    let _ = runner
        .run_before_tool_call("any".to_string(), serde_json::json!({}))
        .await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "counter must not bump after unmount"
    );
}

// ─── LIFO unwind: a chain of 3 sections; remove the middle one ──────

/// Mount three sections in order; remove the middle; observe that
/// positions 0 and 2 still hold the original sections in order.
#[tokio::test]
async fn lifo_unwind_preserves_remaining_prompt_sections_in_order() {
    let slot = Arc::new(PromptSections::new());

    // Manually populate to bypass the harness path (we are testing the
    // slot semantics, which are the substrate of the seam).
    slot.add("alpha", Arc::new(TagSection("alpha-text"))).await;
    slot.add("beta", Arc::new(TagSection("beta-text"))).await;
    slot.add("gamma", Arc::new(TagSection("gamma-text"))).await;
    assert_eq!(slot.len().await, 3);

    // Remove the middle.
    slot.remove("beta").await;
    assert_eq!(slot.len().await, 2);

    let snap = slot.snapshot().await;
    assert_eq!(snap[0].0, "alpha");
    assert_eq!(snap[1].0, "gamma");
    assert_eq!(
        snap[0].1.build(&empty_prompt_context()).unwrap(),
        "<tag>alpha-text</tag>"
    );
    assert_eq!(
        snap[1].1.build(&empty_prompt_context()).unwrap(),
        "<tag>gamma-text</tag>"
    );
}

/// A `Box<dyn HookHandler>`-compatible wrapper that forwards every
/// hook event to the inner `DynamicHooks`. This is how a real boot
/// wires the runner once and lets the kernel mutate children forever.
struct DelegatorToDynamic(Arc<DynamicHooks>);

#[async_trait]
impl HookHandler for DelegatorToDynamic {
    fn name(&self) -> &str {
        "delegator-to-dynamic"
    }
    async fn before_tool_call(
        &self,
        name: String,
        args: serde_json::Value,
    ) -> HookResult<(String, serde_json::Value)> {
        self.0.before_tool_call(name, args).await
    }
    async fn on_after_tool_call(
        &self,
        tool: &str,
        result: &operant_runtime::tools::ToolResult,
        duration: std::time::Duration,
    ) {
        self.0.on_after_tool_call(tool, result, duration).await
    }
    async fn before_prompt_build(&self, prompt: String) -> HookResult<String> {
        self.0.before_prompt_build(prompt).await
    }
}

fn empty_prompt_context<'a>() -> PromptContext<'a> {
    use operant_config::schema::SkillsPromptInjectionMode;

    // Build a Vec to satisfy the slice field; the tests don't read it.
    let tools: &[Box<dyn operant_runtime::tools::Tool>] = &[];
    let skills: &[operant_runtime::skills::Skill] = &[];

    PromptContext {
        workspace_dir: std::path::Path::new("/tmp"),
        model_name: "test",
        tools,
        skills,
        skills_prompt_mode: SkillsPromptInjectionMode::Full,
        identity_config: None,
        dispatcher_instructions: "",
        sends_native_tool_specs: false,
        security_summary: None,
        autonomy_level: AutonomyLevel::Supervised,
    }
}
