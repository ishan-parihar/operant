// model_effort.rs — the reasoning-effort ladder, and its one wire spelling.
//
// (2026-11-15, model plan M2) This module used to declare FOUR levels —
// Low, Normal, High, Max — while the wire the agent speaks has SIX plus off.
// The enum could not represent the vocabulary it was sending, so the mapping
// was written out by hand at four call sites and each one collapsed it
// differently:
//
//   `/effort-set`  accepted  "" | minimal | low | medium | high | xhigh
//                  and folded `minimal` and `low` into one variant
//   init.rs (settings.json) accepted only low | high | max, so `minimal` and
//                  `medium` silently became Normal
//   init.rs (TOML)  folded minimal|low together, sent xhigh|max together, and
//                  turned anything UNRECOGNISED into High — the most expensive
//                  rung, as the fallback for a typo
//   config.rs       documented a fourth spelling: low | normal | high | max
//
// Four spellings of one vocabulary is the defect, not any one of them. This
// file is now the only place the strings exist: `wire()` produces the value the
// agent receives, `from_wire()` is the only parser, and every call site asks
// the type instead of spelling a string.
//
// The capability checks this header used to promise —
// `model_supports_effort` / `model_supports_max_effort` — do not exist here and
// never did; they were named in this comment and in no code. They are model
// plan M3, which is where the model metadata they need comes from.

/// One rung of the reasoning ladder, named as the wire names it.
///
/// The order is least to most intensive, and it is the order the ladder cycles
/// and the picker lists. `Off` is a rung rather than a separate flag: it has a
/// wire value (`""`, meaning "send no reasoning field"), and before this it had
/// NO representation — cycling past the top set the level to `Low` while
/// sending the off value, so the operator turned reasoning off and the display
/// said "low".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum EffortLevel {
    /// Send no reasoning field at all.
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl EffortLevel {
    /// Every rung, least to most intensive. The one list the ladder, the picker
    /// and the parser all iterate, so a new rung cannot be added to one and
    /// forgotten by the others.
    pub const ALL: [Self; 7] = [
        Self::Off,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::XHigh,
        Self::Max,
    ];

    /// The default a fresh session starts on.
    ///
    /// `Medium` is `Normal` renamed: the old default was `Normal` and its wire
    /// value was `medium`, so this is byte-identical on the wire. A rename that
    /// also changed the default would ship a behaviour change under a refactor's
    /// label.
    pub const DEFAULT: Self = Self::Medium;

    /// The value the agent receives. The ONLY producer of the string.
    ///
    /// `Off` is the empty string, which the drain already reads as "clear the
    /// field" (`tui_app.rs`: `if effort.is_empty() { None } else { Some(..) }`).
    pub fn wire(self) -> &'static str {
        match self {
            Self::Off => "",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// Parse a wire value. The ONLY parser.
    ///
    /// Case-insensitive and whitespace-trimmed, because the value arrives from a
    /// TOML file and a command line. Returns `None` for anything else: a caller
    /// that wants a fallback must say which one, rather than inheriting the
    /// silent `High` the TOML path used to substitute for a typo.
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" => Some(Self::Off),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    /// The next rung up, wrapping past the top to `Off`.
    ///
    /// Wrapping to `Off` rather than clamping at `Max` is what makes one key able
    /// to turn reasoning off at all — the ladder is the only control most
    /// operators reach.
    #[cfg(test)]
    pub fn next_up(self) -> Self {
        match self {
            Self::Off => Self::Minimal,
            Self::Minimal => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::XHigh,
            Self::XHigh => Self::Max,
            Self::Max => Self::Off,
        }
    }

    /// A filling-circle glyph, ordered the same way the ladder is, so the shape
    /// of the symbol says where the rung sits.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Off => "\u{2205}",     // ∅  empty set — nothing sent
            Self::Minimal => "\u{25cb}", // ○  empty circle
            Self::Low => "\u{25d4}",     // ◔  quarter
            Self::Medium => "\u{25d0}",  // ◐  half
            Self::High => "\u{25d5}",    // ◕  three-quarter
            Self::XHigh => "\u{25cf}",   // ●  full
            Self::Max => "\u{2b24}",     // ⬤  large full
        }
    }

    /// The name shown to the operator. Identical to `wire()` except for `Off`,
    /// which reads as a word rather than as an empty cell.
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            other => other.wire(),
        }
    }
}

// ---------------------------------------------------------------------------
// Model capability helpers
// ---------------------------------------------------------------------------

/// What a model can do with a reasoning effort.
///
/// (2026-11-15, model plan M3) The three states matter separately. oh-my-pi
/// answers this with a boolean because it ships a generated catalogue of every
/// model; operant resolves capabilities from models.dev's cache, which covers
/// mapped providers only. So a model nobody has metadata for is `Unknown`, and
/// `Unknown` must NOT block — a control that refuses to open for a model it
/// simply has no data on is worse than one that offers a level the provider may
/// ignore.
///
/// `None` is the only blocking state, and it means the metadata was found and
/// says `reasoning: false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortSupport {
    /// Metadata was found and the model does not reason.
    None,
    /// Metadata was found and the model reasons, so every rung is offered.
    ///
    /// The per-model rung LIST is the second half of M3 and is not in this
    /// tranche: it needs models.dev's `thinking` payload, whose shape has not
    /// been verified against a live response, and a guessed schema would parse
    /// to an empty ladder on every model. The gate and the capability check are
    /// what the verified data supports.
    Full,
    /// No metadata for this model. Nothing is blocked.
    Unknown,
}

impl EffortSupport {
    /// Whether the effort controls should open at all.
    #[cfg(test)]
    pub fn allows_effort(self) -> bool {
        !matches!(self, Self::None)
    }

    /// The one-line reason to show the operator when effort is refused.
    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::None => Some("this model does not reason, so there is no effort to set"),
            _ => None,
        }
    }
}

/// What `model` can do with effort, at `provider`.
///
/// `provider` is the OPERANT provider name (the caller resolves it from the live
/// base URL); models.dev keys its capability rows by provider and operant's model
/// ids are bare, so the provider has to be supplied rather than inferred here.
pub fn effort_support(provider: &str, model: &str) -> EffortSupport {
    let trimmed = model.trim();
    if trimmed.is_empty() || provider.trim().is_empty() {
        return EffortSupport::Unknown;
    }
    match operant_core::models_dev::cached_capabilities(provider, trimmed) {
        Some(caps) if caps.supports_reasoning => EffortSupport::Full,
        Some(_) => EffortSupport::None,
        None => EffortSupport::Unknown,
    }
}

/// Clamp a requested level to what a model can do.
///
/// `None` means the model is known not to reason: the caller refuses and says why
/// rather than sending a field the provider will reject. Every other case passes
/// the request through unchanged — without a per-model rung list there is nothing
/// narrower to clamp to, and inventing one would clamp the operator's choice on
/// no evidence.
pub fn clamp_effort(provider: &str, model: &str, requested: EffortLevel) -> Option<EffortLevel> {
    match effort_support(provider, model) {
        EffortSupport::None => None,
        EffortSupport::Full | EffortSupport::Unknown => Some(requested),
    }
}
