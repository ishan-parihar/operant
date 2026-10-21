//! `HasPropKind` impls for the config enums.
//!
//! They lived at the end of `schema/config_impl.rs`, which was deleted with
//! the config aggregate it belonged to (RT-141): the aggregate's property
//! machinery needed them, and the section structs that carry the enums still
//! derive them.

use crate::traits::{HasPropKind, PropKind};

use super::*;

// Scalars (bool, String, integers, floats) are covered by impl_prop_kind! in
// traits.rs. Config enums serialize as TOML strings and are classified as
// PropKind::Enum.
macro_rules! impl_enum_prop_kind {
    ($($ty:ty),+ $(,)?) => {
        $(impl HasPropKind for $ty { const PROP_KIND: PropKind = PropKind::Enum; })+
    };
}

impl_enum_prop_kind!(ProxyScope, SearchMode,);

impl HasPropKind for ModelPricing {
    // ModelPricing is a 2-field struct (`input`, `output`). Wire form is a
    // JSON object (e.g. `{"input": 1.0, "output": 2.5}`); the dashboard
    // renders a sub-form for the inner fields. `PropKind::Object` (vs
    // `String`) is what makes the round-trip through `Config::set_prop`
    // succeed — `parse_prop_value` parses the JSON into a TOML table so
    // serde deserializes it back into the typed `ModelPricing` instead
    // of failing on a TOML string (#6357 review).
    const PROP_KIND: PropKind = PropKind::Object;
}

impl HasPropKind for serde_json::Value {
    // `serde_json::Value` is an arbitrary JSON document, not an enum.
    // Classifying it as `Enum` previously made `enum_variants_for::<Value>()`
    // hand back the literal placeholder `"(unknown variants)"`, and the
    // dashboard form rendered fields like `providers.models.<key>.provider_extra`
    // as a single-option dropdown. `String` is the closest scalar kind —
    // the form renders a text input where the user pastes raw JSON.
    // Round-trip via `set_prop` stays correct: serde deserializes the TOML
    // string back into `Value::String(...)`. Power users editing complex
    // objects still use `operant config set --json` or hand-edit the
    // `config.toml`.
    const PROP_KIND: PropKind = PropKind::String;
}
