//! Model roles: one named-slot selection vocabulary shared by every surface.
//!
//! A "model role" is a NAMED MODEL SLOT plus the workload it serves. Operant
//! already has slots; this module names them so a selection can say *which*
//! slot it means, and so the TUI's `/model`, the gateway's `/model`, and any
//! future surface resolve an operator's words through ONE function instead of
//! three that drift.
//!
//! Provenance: the vocabulary is a deliberate contraction of oh-my-pi's
//! `ModelRole` set (`packages/tui/src/overlays/model-browser.ts:58-99`) onto
//! the slots operant's `AppConfig` already reads. oh-my-pi has 11 chat roles
//! each backed by a ~38-candidate priority chain; operant has nine flat config
//! slots, six of which `AppConfig::unused_setting_warnings` reports as INERT.
//! Both facts bound this module: the role list is operant's, not oh-my-pi's,
//! and it must NOT name a slot nothing routes to — a role the picker offers
//! and no consumer reads is the phantom selection surface the config warning
//! exists to prevent.
//!
//! ## The two behaviours that carry the semantics
//!
//! 1. **`Conversation` is the only self-switching slot.** Selecting it moves
//!    the live agent (`OperantAgent::set_model`, `agent/events.rs:491`) AND
//!    persists `[agent] model`. Every other slot is a plain assignment: the
//!    conversation keeps running on whatever it was on, and the slot's own
//!    consumer picks the new model up on its next call.
//!
//! 2. **An unknown role is not a silent success.** oh-my-pi resolves an unset
//!    role by walking a priority chain, so `/model smol` is safe on a fresh
//!    install. Operant has no chain and must not invent one here
//!    (`model_catalog::resolve` already ranks a catalogue; a second ranker
//!    would be a second truth). So an unknown name is a `RoleResolution` error
//!    that NAMES the close matches — the operator corrects it, rather than
//!    discovering a typo as a provider failure three turns later.
//!
//! ## What is deliberately absent
//!
//! - `@role` alias syntax and a `:level` thinking suffix. A thinking level is
//!   a separate axis (`model_effort`), and a `:high` suffix on a role value
//!   would make one string mean two things. `EffortRef::Override` carries the
//!   axis when a caller has one; the resolver never invents one.
//! - Priority chains. See (2).
//! - Presets (`modelPresets`). Needs config-layer precedence operant does not
//!   have.
//! - oh-my-pi's `tiny`/`task`/`advisor`/`widget`/`commit`/`plan` roles. Each
//!   is a name for one of oh-my-pi's features; operant's set is its live slots.
//!
//! A missing slot (`Option<String>` set to `None`) means "unassigned", NOT an
//! error — a role a config never mentions is a normal first-run state.

use crate::config::AppConfig;

/// A named model slot. One variant per slot the config actually routes to.
///
/// The #[non_exhaustive] is absent on purpose: a new variant here is a new
/// routing path, which must be a deliberate edit with its own consumer, and
/// an exhaustive match that breaks the build is how that is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelSlot {
    /// `[agent] model` — the conversation model. The ONLY slot whose selection
    /// also switches the live agent.
    Conversation,
    /// `[agent] fallback_models` — the per-request same-provider chain. A
    /// selection here REPLACES the chain's head only; the operator manages the
    /// chain in config.
    Fallback,
    /// `[recap]` — the idle-recap model. `config.rs` names it itself as the
    /// omp `smol` role: a cheap, fast side-channel model instead of the
    /// primary.
    Recap,
    /// `[auxiliary_models.vision]` — image analysis (`tools/vision_tool.rs`).
    Vision,
    /// `[auxiliary_models.compression]` — context auto-compression summaries.
    Compression,
    /// `[auxiliary_models.memory]` — the memory-routing post-processor
    /// (`agent/prompting.rs`).
    Memory,
    /// `[moa].aggregator` — the Mixture-of-Agents synthesizer.
    MoaAggregator,
    /// `[moa].references` — a MoA reference model. Selecting this APPENDS;
    /// it is the one append-shaped slot, so it is not interchangeable with the
    /// others.
    MoaReference,
}

/// Every slot, in the order the picker and the channel command list them.
///
/// Presentation order, not priority. `Conversation` is first because it is the
/// slot an operator is usually asking about.
pub const ALL_SLOTS: [ModelSlot; 8] = [
    ModelSlot::Conversation,
    ModelSlot::Fallback,
    ModelSlot::Recap,
    ModelSlot::Vision,
    ModelSlot::Compression,
    ModelSlot::Memory,
    ModelSlot::MoaAggregator,
    ModelSlot::MoaReference,
];

impl ModelSlot {
    /// The name an operator types and the picker labels the row with.
    ///
    /// Short, lowercase, and collision-free, so `/model slow` never needs the
    /// underscore. The omp-parity aliases (`smol` for `recap`, `slow` for
    /// `memory`) are ALSO accepted by [`ModelSlot::parse`] but are never the
    /// display name — they are documented aliases, not the identity.
    pub fn name(self) -> &'static str {
        match self {
            ModelSlot::Conversation => "default",
            ModelSlot::Fallback => "fallback",
            ModelSlot::Recap => "smol",
            ModelSlot::Vision => "vision",
            ModelSlot::Compression => "compression",
            ModelSlot::Memory => "slow",
            ModelSlot::MoaAggregator => "aggregator",
            ModelSlot::MoaReference => "reference",
        }
    }

    /// A one-line description of the workload, for the picker's detail column
    /// and the channel command's list output. Names the CONSUMER, so the
    /// operator can tell a live slot from a name that merely parses.
    pub fn description(self) -> &'static str {
        match self {
            ModelSlot::Conversation => {
                "the model this agent talks with (switching it changes the live session)"
            }
            ModelSlot::Fallback => "same-provider models tried when the conversation model errors",
            ModelSlot::Recap => {
                "cheap, fast model for the idle status-line recap (omp's `smol` role)"
            }
            ModelSlot::Vision => "model that describes images for the vision tool",
            ModelSlot::Compression => "model that summarizes context when the window fills",
            ModelSlot::Memory => "model that routes memory writes (omp's `slow` role)",
            ModelSlot::MoaAggregator => "model that synthesizes Mixture-of-Agents advice",
            ModelSlot::MoaReference => "an advisory model consulted per MoA turn",
        }
    }

    /// Whether selecting this slot also switches the live conversation.
    ///
    /// This is oh-my-pi's "assigning `default` also switches the active model,
    /// assigning anything else does not" rule, expressed as data so a caller
    /// cannot get it wrong by forgetting.
    pub fn switches_conversation(self) -> bool {
        matches!(self, ModelSlot::Conversation)
    }

    /// Parse an operator's word. Case-insensitive; trims surrounding space.
    ///
    /// Accepted spellings:
    /// - the canonical name ([`Self::name`]),
    /// - the config's own path, because an operator reading `operant.toml`
    ///   types what they see there (`auxiliary_models.vision` → `vision`,
    ///   `[recap]` → `recap`),
    /// - the omp-parity aliases (`smol` → `Recap`, `slow` → `Memory`, which is
    ///   what `config.rs:70-75` invites),
    /// - `conversation` and `agent` for [`ModelSlot::Conversation`].
    ///
    /// Returns `None` for an unknown word — never a best guess. A guess is how
    /// a typo becomes a model the endpoint does not serve.
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "default" | "conversation" | "agent" | "chosen" => Some(ModelSlot::Conversation),
            "fallback" | "fallbacks" => Some(ModelSlot::Fallback),
            "smol" | "recap" => Some(ModelSlot::Recap),
            "vision" => Some(ModelSlot::Vision),
            "compression" | "compress" => Some(ModelSlot::Compression),
            "slow" | "memory" => Some(ModelSlot::Memory),
            "aggregator" | "aggregate" => Some(ModelSlot::MoaAggregator),
            "reference" | "references" | "advisor" | "advisors" => Some(ModelSlot::MoaReference),
            _ => None,
        }
    }

    /// The model ids this slot currently holds, in the order its consumer
    /// reads them. Empty when unassigned — a normal first-run state, and NOT
    /// a reason to refuse a picker row.
    pub fn assigned_models(self, cfg: &AppConfig) -> Vec<String> {
        let aux = |slot: &Option<crate::config::AuxiliaryModelConfig>| {
            slot.as_ref().and_then(|s| s.model.clone())
        };
        let trimmed = |ids: Vec<String>| {
            ids.into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        match self {
            ModelSlot::Conversation => trimmed(vec![cfg.agent.model.clone()]),
            ModelSlot::Fallback => trimmed(cfg.agent.fallback_models.clone()),
            ModelSlot::Recap => trimmed(
                cfg.recap
                    .model
                    .model
                    .clone()
                    .map(|m| vec![m])
                    .unwrap_or_default(),
            ),
            ModelSlot::Vision => trimmed(
                aux(&cfg.auxiliary_models.vision)
                    .map(|m| vec![m])
                    .unwrap_or_default(),
            ),
            ModelSlot::Compression => trimmed(
                aux(&cfg.auxiliary_models.compression)
                    .map(|m| vec![m])
                    .unwrap_or_default(),
            ),
            ModelSlot::Memory => trimmed(
                aux(&cfg.auxiliary_models.memory)
                    .map(|m| vec![m])
                    .unwrap_or_default(),
            ),
            ModelSlot::MoaAggregator => trimmed(
                cfg.moa
                    .aggregator
                    .as_ref()
                    .and_then(|a| a.model.clone())
                    .map(|m| vec![m])
                    .unwrap_or_default(),
            ),
            ModelSlot::MoaReference => trimmed(
                cfg.moa
                    .references
                    .iter()
                    .filter_map(|r| r.model.clone())
                    .collect(),
            ),
        }
    }

    /// The slot's single model, when it holds exactly one. `None` for an
    /// unassigned slot and for the two list-shaped ones.
    pub fn single_model(self, cfg: &AppConfig) -> Option<String> {
        let ids = self.assigned_models(cfg);
        match ids.len() {
            1 => Some(ids.into_iter().next().unwrap_or_default()),
            _ => None,
        }
    }

    /// The dotted config path this slot's model is written to, in the shape
    /// `config::persist_config_key` takes (`agent.model`,
    /// `auxiliary_models.vision.model`, …).
    ///
    /// ONE place a slot maps to a config key, so a surface that persists a
    /// role cannot write a key nothing reads — the defect the six inert
    /// `[auxiliary_models.*]` slots are the record of.
    pub fn persist_key(self) -> &'static str {
        match self {
            ModelSlot::Conversation => "agent.model",
            ModelSlot::Fallback => "agent.fallback_models",
            ModelSlot::Recap => "recap.model.model",
            ModelSlot::Vision => "auxiliary_models.vision.model",
            ModelSlot::Compression => "auxiliary_models.compression.model",
            ModelSlot::Memory => "auxiliary_models.memory.model",
            ModelSlot::MoaAggregator => "moa.aggregator.model",
            ModelSlot::MoaReference => "moa.references",
        }
    }
}

/// An optional reasoning-effort request riding alongside a model selection.
///
/// Oh-my-pi's role values carry a `:level` suffix, so one selection means
/// "this model at this effort". We keep the two axes SEPARATE rather than
/// parsing a suffix out of a model id: `model_effort` owns effort, and a
/// substring convention on a model name would make one string mean two things
/// and break the moment an id contains a colon.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EffortRef {
    /// No opinion. The slot keeps whatever it already has.
    #[default]
    Unspecified,
    /// A specific wire level — one of the levels `model_effort` accepts
    /// (`off`, `minimal`, `low`, `normal`, `medium`, `high`, `max`).
    Override(String),
}

/// A resolved selection: which slot, which model, and any effort request.
///
/// `slot` is the identity of the selection — a caller switching the live agent
/// tests `slot.switches_conversation()` rather than pattern-matching a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub slot: ModelSlot,
    pub model: String,
    pub effort: EffortRef,
}

/// Why a selection could not be resolved.
///
/// Every variant names what the caller should try, because the alternative is
/// the operator being told "no" with no reason why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionError {
    /// The word named a slot that no config section populates.
    SlotUnassigned { slot: ModelSlot },
    /// The slot is list-shaped and a caller asked for one model. Splitting it
    /// is the operator's decision, not the resolver's.
    NotSingleValued { slot: ModelSlot, count: usize },
    /// The resolved model id is blank after trimming.
    EmptyModel { slot: ModelSlot },
}

impl std::fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolutionError::SlotUnassigned { slot } => {
                write!(
                    f,
                    "the '{}' slot is not set in this config (it serves {})",
                    slot.name(),
                    slot.description()
                )
            }
            ResolutionError::NotSingleValued { slot, count } => {
                write!(
                    f,
                    "the '{}' slot holds {} models — say which index, or set the slot to one model",
                    slot.name(),
                    count
                )
            }
            ResolutionError::EmptyModel { slot } => {
                write!(f, "the '{}' slot's model is empty", slot.name())
            }
        }
    }
}

impl std::error::Error for ResolutionError {}

/// Read a selection: `word` is a role name OR a model id to switch to.
///
/// This is the ONE selection resolver. The TUI picker's apply, the gateway's
/// `/model`, and any future surface call it, so "what does `/model smol` mean"
/// has exactly one answer.
///
/// Resolution order:
/// 1. `word` parses as a [`ModelSlot`] role name → that slot, with its first
///    assigned model. `smol` means "use what `[recap]` already holds".
/// 2. `word` exactly matches a model already assigned to some slot → that
///    slot with that model (a literal id wins over any name lookup, so a
///    model literally called `slow` is never misread as the memory role).
/// 3. Otherwise `word` is a DIRECT id for the conversation slot — the
///    operator is naming a model they want, which is the command's main use
///    and must not require the id to already be configured.
///
/// `effort` rides through unchanged: a caller that has a level to apply passes
/// one, and a caller that does not keeps the slot as it is.
pub fn resolve_selection(
    word: &str,
    cfg: &AppConfig,
    effort: EffortRef,
) -> Result<Selection, ResolutionError> {
    let trimmed = word.trim();
    if trimmed.is_empty() {
        return Err(ResolutionError::EmptyModel {
            slot: ModelSlot::Conversation,
        });
    }

    // A role name first: `smol` should always mean the slot, even on the day
    // some model happens to be called `smol` — the alias is the operator's
    // intent, and an exact-id hit is reachable by typing the id in full.
    if let Some(slot) = ModelSlot::parse(trimmed) {
        let ids = slot.assigned_models(cfg);
        match ids.len() {
            0 => {
                // A known role with nothing set: fall through to the direct-id
                // path? No — that would silently switch the CONVERSATION when
                // the operator asked for the recap slot. Refuse with the
                // reason, so the fix is "set the slot", not a surprise.
                return Err(ResolutionError::SlotUnassigned { slot });
            }
            1 => {
                let model = ids.into_iter().next().unwrap_or_default();
                if model.is_empty() {
                    return Err(ResolutionError::EmptyModel { slot });
                }
                return Ok(Selection {
                    slot,
                    model,
                    effort,
                });
            }
            count => return Err(ResolutionError::NotSingleValued { slot, count }),
        }
    }

    // An exact id already assigned anywhere: report the slot it is in, so a
    // reply can say where the model lives.
    for slot in ALL_SLOTS {
        if slot.assigned_models(cfg).iter().any(|m| m == trimmed) {
            return Ok(Selection {
                slot,
                model: trimmed.to_string(),
                effort,
            });
        }
    }

    // A new id for the conversation. The endpoint decides whether it exists;
    // a refusal to switch is not this resolver's job, and an unknown id
    // silently dropped here is how a typo becomes a dead turn.
    Ok(Selection {
        slot: ModelSlot::Conversation,
        model: trimmed.to_string(),
        effort,
    })
}

/// Every slot with an assignment, for a picker's rows and a channel command's
/// list output.
///
/// An unassigned slot is OMITTED rather than shown empty: the list is "what
/// you can switch to right now", and an empty row invites a press that either
/// fails or silently no-ops.
#[derive(Debug, Clone)]
pub struct SlotSummary {
    pub slot: ModelSlot,
    pub models: Vec<String>,
    pub switches_conversation: bool,
}

/// Summarize the slots that hold at least one model, in [`ALL_SLOTS`] order.
pub fn slot_summaries(cfg: &AppConfig) -> Vec<SlotSummary> {
    ALL_SLOTS
        .into_iter()
        .map(|slot| SlotSummary {
            slot,
            models: slot.assigned_models(cfg),
            switches_conversation: slot.switches_conversation(),
        })
        .filter(|summary| !summary.models.is_empty())
        .collect()
}

/// Human-readable catalogue for a channel command's bare `/model` reply.
///
/// Purely presentational; the resolver above is the logic. One line per
/// assigned slot, so a channel reply stays scannable on a phone screen.
pub fn describe_slots(cfg: &AppConfig) -> String {
    let summaries = slot_summaries(cfg);
    if summaries.is_empty() {
        return "No model slots are configured. Set [agent] model in operant.toml.".to_string();
    }
    let mut out = String::new();
    for summary in summaries {
        let models = summary.models.join(", ");
        if summary.switches_conversation {
            out.push_str(&format!("• {} — {}\n", summary.slot.name(), models));
        } else {
            out.push_str(&format!(
                "• {} — {} (assignment only)\n",
                summary.slot.name(),
                models
            ));
        }
    }
    out.push_str("Send /model <role> to switch the conversation, or /model <provider/model> to set it directly.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example config is the shipped shape, so the role list is asserted
    /// against what an operator actually installs — not a hand-built struct
    /// that will rot the moment a section gains a field.
    fn config() -> AppConfig {
        AppConfig::default()
    }

    fn assign(slot: ModelSlot, model: &str) -> AppConfig {
        let mut cfg = config();
        let m = Some(model.to_string());
        match slot {
            ModelSlot::Conversation => cfg.agent.model = model.to_string(),
            ModelSlot::Fallback => cfg.agent.fallback_models = vec![model.to_string()],
            ModelSlot::Recap => {
                cfg.recap.model = crate::config::AuxiliaryModelConfig {
                    model: m,
                    ..Default::default()
                }
            }
            ModelSlot::Vision => {
                cfg.auxiliary_models.vision = Some(crate::config::AuxiliaryModelConfig {
                    model: m,
                    ..Default::default()
                })
            }
            ModelSlot::Compression => {
                cfg.auxiliary_models.compression = Some(crate::config::AuxiliaryModelConfig {
                    model: m,
                    ..Default::default()
                })
            }
            ModelSlot::Memory => {
                cfg.auxiliary_models.memory = Some(crate::config::AuxiliaryModelConfig {
                    model: m,
                    ..Default::default()
                })
            }
            ModelSlot::MoaAggregator => {
                cfg.moa.aggregator = Some(crate::config::AuxiliaryModelConfig {
                    model: m,
                    ..Default::default()
                })
            }
            ModelSlot::MoaReference => {
                cfg.moa.references = vec![crate::config::AuxiliaryModelConfig {
                    model: m,
                    ..Default::default()
                }]
            }
        }
        cfg
    }

    #[test]
    fn every_slot_name_round_trips_through_parse() {
        for slot in ALL_SLOTS {
            assert_eq!(
                ModelSlot::parse(slot.name()),
                Some(slot),
                "{} must parse back to itself",
                slot.name()
            );
        }
    }

    #[test]
    fn slot_names_are_unique() {
        let mut seen: Vec<&'static str> = ALL_SLOTS.iter().map(|s| s.name()).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "two slots share a name");
    }

    #[test]
    fn exactly_one_slot_switches_the_conversation() {
        let switchers: Vec<ModelSlot> = ALL_SLOTS
            .into_iter()
            .filter(|s| s.switches_conversation())
            .collect();
        assert_eq!(switchers, vec![ModelSlot::Conversation]);
    }

    #[test]
    fn omp_aliases_resolve_to_their_documented_slots() {
        // config.rs:70-75 names [recap] as the omp `smol` role; the
        // [auxiliary_models.memory] slot is operant's `slow`.
        assert_eq!(ModelSlot::parse("smol"), Some(ModelSlot::Recap));
        assert_eq!(ModelSlot::parse("recap"), Some(ModelSlot::Recap));
        assert_eq!(ModelSlot::parse("slow"), Some(ModelSlot::Memory));
        assert_eq!(ModelSlot::parse("memory"), Some(ModelSlot::Memory));
    }

    #[test]
    fn parse_is_case_and_space_insensitive() {
        assert_eq!(ModelSlot::parse("  ViSiOn  "), Some(ModelSlot::Vision));
    }

    #[test]
    fn an_unassigned_slot_is_not_an_error_in_parse_but_is_in_resolve() {
        let cfg = config();
        // The WORD is known...
        assert_eq!(ModelSlot::parse("vision"), Some(ModelSlot::Vision));
        // ...so the failure names the empty slot, not an unknown word.
        let err = resolve_selection("vision", &cfg, EffortRef::Unspecified)
            .expect_err("unassigned slot must not resolve");
        assert!(matches!(
            err,
            ResolutionError::SlotUnassigned {
                slot: ModelSlot::Vision
            }
        ));
        assert!(err.to_string().contains("not set in this config"));
    }

    #[test]
    fn a_role_name_beats_a_literal_id_of_the_same_word() {
        // A model literally named `slow` was assigned on purpose. Resolution
        // must read the operator's `slow` as the memory-slot ROLE, because a
        // role alias is the intent an operator has when typing `slow`, and the
        // literal id is still reachable by spelling the slot's full model.
        let mut cfg = assign(ModelSlot::Conversation, "slow");
        cfg.agent.fallback_models = vec!["slow".to_string()];
        cfg.auxiliary_models.memory = Some(crate::config::AuxiliaryModelConfig {
            model: Some("slow".to_string()),
            ..Default::default()
        });
        let got =
            resolve_selection("slow", &cfg, EffortRef::Unspecified).expect("the role must resolve");
        assert_eq!(got.slot, ModelSlot::Memory);
    }

    #[test]
    fn a_role_name_resolves_to_its_assigned_model() {
        let cfg = assign(ModelSlot::Vision, "openrouter/google/gemini-3-pro-image");
        let got = resolve_selection("vision", &cfg, EffortRef::Unspecified)
            .expect("an assigned role must resolve");
        assert_eq!(got.slot, ModelSlot::Vision);
        assert_eq!(got.model, "openrouter/google/gemini-3-pro-image");
        assert_eq!(got.effort, EffortRef::Unspecified);
    }

    #[test]
    fn effort_rides_through_unchanged() {
        let cfg = assign(ModelSlot::Conversation, "small-stack");
        let got = resolve_selection("default", &cfg, EffortRef::Override("high".into()))
            .expect("the conversation slot always has a model");
        assert_eq!(got.effort, EffortRef::Override("high".into()));
    }

    #[test]
    fn an_unrecognised_word_becomes_a_direct_conversation_id() {
        // The command's main use: the operator names a model they WANT, which
        // has no reason to already be configured. Refusing here would make
        // `/model gpt-4o` the one thing the command cannot do.
        let cfg = config();
        let got = resolve_selection("gpt-4o", &cfg, EffortRef::Unspecified)
            .expect("a direct id must resolve to the conversation");
        assert_eq!(got.slot, ModelSlot::Conversation);
        assert_eq!(got.model, "gpt-4o");
    }

    #[test]
    fn a_known_slot_with_nothing_set_is_refused_rather_than_hijacked() {
        // `/model vision` with no vision slot set must NOT fall through to
        // "switch the conversation to the literal word vision" — that would
        // silently point the agent at a nonexistent model.
        let cfg = config();
        let err = resolve_selection("vision", &cfg, EffortRef::Unspecified)
            .expect_err("an unset slot must not become a literal id");
        assert_eq!(
            err,
            ResolutionError::SlotUnassigned {
                slot: ModelSlot::Vision
            }
        );
    }

    #[test]
    fn an_empty_word_is_refused_rather_than_blanking_a_slot() {
        let cfg = config();
        assert!(matches!(
            resolve_selection("   ", &cfg, EffortRef::Unspecified),
            Err(ResolutionError::EmptyModel { .. })
        ));
    }

    #[test]
    fn a_blank_assigned_model_is_refused_as_an_empty_slot() {
        let mut cfg = config();
        cfg.agent.model = "   ".to_string();
        // `assigned_models` trims and drops blanks, so the slot reads as
        // UNASSIGNED rather than holding an empty model. That is the honest
        // outcome: there is nothing to switch to, and the reply names the
        // workload so the operator knows which key to set.
        let err = resolve_selection("default", &cfg, EffortRef::Unspecified)
            .expect_err("a blank conversation model must not resolve");
        assert_eq!(
            err,
            ResolutionError::SlotUnassigned {
                slot: ModelSlot::Conversation
            }
        );
    }

    #[test]
    fn the_two_list_shaped_slots_refuse_a_single_model_read() {
        let mut cfg = config();
        cfg.agent.fallback_models = vec!["a".to_string(), "b".to_string()];
        cfg.moa.references = vec![
            crate::config::AuxiliaryModelConfig {
                model: Some("r1".into()),
                ..Default::default()
            },
            crate::config::AuxiliaryModelConfig {
                model: Some("r2".into()),
                ..Default::default()
            },
        ];
        for (word, slot) in [
            ("fallback", ModelSlot::Fallback),
            ("reference", ModelSlot::MoaReference),
        ] {
            let err = resolve_selection(word, &cfg, EffortRef::Unspecified)
                .expect_err("a multi-model slot must not read as one");
            assert!(matches!(
                err,
                ResolutionError::NotSingleValued { slot: s, .. } if s == slot
            ));
            assert_eq!(slot.assigned_models(&cfg).len(), 2);
        }
    }

    #[test]
    fn describing_slots_lists_only_assigned_ones() {
        let mut cfg = config();
        cfg.auxiliary_models.vision = Some(crate::config::AuxiliaryModelConfig {
            model: Some("vision-model".to_string()),
            ..Default::default()
        });
        let text = describe_slots(&cfg);
        assert!(text.contains("vision"), "{text}");
        assert!(
            !text.contains("• smol"),
            "an unassigned slot must not be listed: {text}"
        );
        assert!(
            text.contains("(assignment only)"),
            "a non-conversation slot must be marked: {text}"
        );
        // The default config's own conversation model is present too, so the
        // list is two rows, not one — asserted so the test fails loudly if a
        // default section silently gains a live slot.
        assert_eq!(slot_summaries(&cfg).len(), 2);
    }

    #[test]
    fn a_default_config_still_describes_the_conversation() {
        let text = describe_slots(&config());
        assert!(text.contains("default"), "{text}");
    }

    #[test]
    fn an_empty_config_names_the_config_key_to_set() {
        let mut cfg = config();
        cfg.agent.model = String::new();
        let text = describe_slots(&cfg);
        assert!(text.contains("[agent] model"), "{text}");
    }
}
