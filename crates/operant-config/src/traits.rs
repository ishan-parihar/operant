//! Runtime type classification for config property values.
//!
//! The rest of this file's former contents described the `Configurable`
//! derive's property machinery (`SecretFieldInfo`, `PropFieldInfo`,
//! `IntegrationDescriptor`, `ChannelConfig`, `ConfigHandle`, `SelectItem`,
//! `Answer`, `OnboardUi`, the `MapKey*` kinds). The derive is GONE (it
//! existed only for the retired config aggregate — RT-141), and the two
//! items that outlived it are here: the property-kind classification and
//! the trait that maps a Rust type onto it.

/// Runtime type classification for config property values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    /// A string field.
    String,
    /// A boolean field.
    Bool,
    /// An integer field.
    Integer,
    /// A float field.
    Float,
    /// An enum or other serde-serializable type (parsed as TOML string).
    Enum,
    /// A `Vec<String>` field; set via comma-separated input.
    StringArray,
    /// A `Vec<T>` field where `T` is a serializable struct (e.g. `Vec<McpServerConfig>`,
    /// `Vec<PeripheralBoardConfig>`). Round-tripped on the wire as a JSON array of
    /// objects; the dashboard renders a per-row sub-form using the JSON Schema
    /// from `OPTIONS /api/config` to discover the element type's field shape.
    /// Schema v3 / #5947 will migrate the load-bearing ones (mcp.servers etc.)
    /// to `HashMap<String, T>` keyed tables; until then this kind covers them.
    ObjectArray,
    /// A struct-shaped scalar field (e.g. `Option<ModelPricing>`). Round-tripped
    /// on the wire as a JSON object; the dashboard renders a sub-form for the
    /// inner fields using the JSON Schema from `OPTIONS /api/config`. Distinct
    /// from `String`, which inserts the raw value as a TOML string and breaks
    /// the serde round-trip for typed structs (#6357 review).
    Object,
}

/// Maps Rust types to PropKind at compile time.
/// Scalars have explicit impls; the blanket impl catches everything
/// else as `PropKind::Enum`.
pub trait HasPropKind {
    /// The runtime property-kind classification for this type.
    const PROP_KIND: PropKind;
}

macro_rules! impl_prop_kind {
    ($kind:expr, $($ty:ty),+) => {
        $(impl HasPropKind for $ty { const PROP_KIND: PropKind = $kind; })+
    };
}

impl_prop_kind!(PropKind::Bool, bool);
impl_prop_kind!(PropKind::String, String);
impl_prop_kind!(PropKind::Float, f64, f32);
impl_prop_kind!(
    PropKind::Integer,
    u8,
    u16,
    u32,
    u64,
    usize,
    i8,
    i16,
    i32,
    i64,
    isize
);
impl HasPropKind for Vec<String> {
    const PROP_KIND: PropKind = PropKind::StringArray;
}
