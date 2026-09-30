//! Pool manifest compiler — reads real organism `_org.yaml` / `_pool.yaml`
//! files and compiles them to a vector of
//! [`crate::composition::ArchitectureRow`]s the kernel's
//! [`crate::composition::Builder`] can mount.
//!
//! ## Two real dialects, one parser
//!
//! Measured 2026-09-30 over `~/.hermes/organism/` (physical files, deduped
//! by realpath): 94 `_org.yaml` and 32 `_pool.yaml`. The organism's own
//! resolver (`cortex/meta-governance/lib/core.py:217 find_pool_config`)
//! treats `_org.yaml` as canonical and `_pool.yaml` as a legacy fallback.
//! **All 24 live top-level pools carry `_org.yaml`**; exactly one
//! (`foundations/presentation-engine`) carries both. Both dialects are
//! therefore modelled here, on the shared [`OrgManifest`] struct, because
//! they are near-identical: the legacy files are an earlier revision of the
//! same genome, not a different contract.
//!
//! Three schema facts the previous v1 struct got wrong, all reproduced here:
//!
//! 1. **The name is nested.** There is no top-level `name`; it is
//!    `pool.name`. A top-level `#[serde(default)] name: String` silently
//!    yields `""`, and the failure surfaces as "pool manifest has empty
//!    `name`" — an error naming a symptom the operator cannot act on.
//! 2. **The service key is `id`, not `name`.** Real entries are
//!    `{id, entry_point | interface, accepts, returns}`. A *required*
//!    `name: String` makes serde hard-fail on every real file.
//! 3. **Real service ids carry no verb prefix.** `axe-feedback`,
//!    `knowledge-graph`, `ad-rg-lifecycle` are plain slugs. A
//!    [`READ_ONLY_VERBS`] gate therefore rejects every real manifest.
//!
//! ## Read-only is a property of the access mode, not the verb name
//!
//! See [`AccessMode`] and [`WritePolicy`]. A service is read-only when the
//! consumer edge's declared `access:` mode is read-shaped, or when the
//! caller widens [`WritePolicy::capabilities`] with an explicit approval
//! token. [`READ_ONLY_VERBS`] is retained as an *additional* check, applied
//! only to manifests that actually use dotted verb-style service names —
//! which, measured across all 126 real manifests, is **none of them**.
//!
//! ## Bundling is symlink-based
//!
//! AD-060 expresses pooling as exact-name symlinks inside `sub-systems/`.
//! [`compile_dir`] walks that directory and emits one `pool.bundle` row per
//! entry actually present, rather than trusting a declared field that a
//! real pool may not carry (only 1 live `_pool.yaml` and 1 live `_org.yaml`
//! declare any sub-system list at all).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::composition::ArchitectureRow;
use crate::error::HarnessError;

/// The verbs v1 accepted as proof of read-only. **No real organism manifest
/// uses a dotted service id** (measured: 0 of 126 physical files), so this
/// check is applied only when a service id actually looks verb-style. It is
/// kept because the legacy schema and `operant-harness/tests/hermes_pilot.rs`
/// still generate dotted names, and because other code references it.
pub const READ_ONLY_VERBS: &[&str] = &[
    "query.", "fetch.", "list.", "search.", "get.", "read.", "lookup.",
];

/// Canonical organism manifest file name (AD-011 Am.3 org consolidation).
pub const ORG_CONFIG_NAME: &str = "_org.yaml";

/// Legacy organism manifest file name, still shipped by 32 pools.
pub const LEGACY_CONFIG_NAME: &str = "_pool.yaml";

// ---------------------------------------------------------------------------
// Real schema
// ---------------------------------------------------------------------------

/// Real organism pool manifest (`_org.yaml` canonical, `_pool.yaml` legacy).
///
/// Unknown fields are preserved rather than dropped: serde is not in
/// `deny_unknown_fields` mode anywhere in the organism, and a pool that
/// carries a block we do not model yet must still parse. Everything except
/// `pool` is `#[serde(default)]` because the measured variance is large —
/// e.g. `graveyard/_org.yaml` has no `tier` at all, and
/// `graveyard/revenue-engine-pre-split/_pool.yaml` has no `services_offered`
/// key whatsoever.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OrgManifest {
    /// Pool identity block. Required in every real manifest (94/94 `_org.yaml`
    /// with a `pool` key, 32/32 `_pool.yaml`).
    #[serde(default)]
    pub pool: PoolBlock,

    /// Services offered to other pools. Each `id` becomes a claim.
    #[serde(default, deserialize_with = "de_services_offered")]
    pub services_offered: Vec<OfferedEntry>,

    /// Services consumed from other pools. Each becomes a `requires` claim.
    #[serde(default, deserialize_with = "de_services_consumed")]
    pub services_consumed: Vec<ConsumedEntry>,

    /// Cross-system relationships (AD-062). `depends_on` is a pool-name
    /// list; `loops` is the same shape and names the pools this one feeds
    /// back into.
    #[serde(default)]
    pub relationships: Relationships,

    /// Genome declaration (4 layers, AD-074 + CP-AD-017). Values are free
    /// form: `presentation-engine` uses nested maps of strings, others use
    /// string lists, so this is a `Value` map rather than a typed struct.
    #[serde(default)]
    pub genome: BTreeMap<String, serde_yaml::Value>,

    /// Genome organs this pool pools from elsewhere. **Two shapes exist in
    /// the wild**: a list of bare strings (the `_org.yaml` dialect, 26
    /// occurrences) and a list of maps with `name` / `target_pool` /
    /// `sub_system` / `note` (the `_pool.yaml` dialect, 1 occurrence).
    #[serde(default)]
    pub pooled_sub_systems: Vec<PooledSubSystem>,

    /// Native sub-system organs. Same two shapes as
    /// `pooled_sub_systems`.
    #[serde(default)]
    pub sub_systems: Vec<PooledSubSystem>,

    /// Directory genome (RG-027 routing contract). Values carry `suffix`
    /// (string or list), `prefix`, `allow_any`, `required`, `pattern`,
    /// `frontmatter_required`, `purposes`, `allow_root_files`, `shebang`,
    /// `description` and `sub_dirs`. Kept as opaque `Value` — the shapes are
    /// too varied (both block and flow style, nested to any depth) for a
    /// typed struct to be an honest model.
    #[serde(default)]
    pub directories: BTreeMap<String, serde_yaml::Value>,

    /// Files that must exist at the pool root.
    #[serde(default)]
    pub required_files: Vec<String>,

    /// Glob patterns that must not exist anywhere in the pool.
    #[serde(default)]
    pub forbidden: Vec<String>,

    /// Root-level file allowlist (AD-074 structural allowlist).
    #[serde(default)]
    pub allowed_root_files: Vec<String>,

    /// Department declaration (AXE-AD-011 Am.3). `class` is
    /// `staffed | utility | dormant`; the tree also carries `active`.
    #[serde(default)]
    pub department: Option<Department>,

    /// Compass-graduate block. The `_pool.yaml` dialect spells this at top
    /// level; the `_org.yaml` dialect nests the same data under
    /// `pool.graduation`.
    #[serde(default)]
    pub compass_graduate: Option<Graduation>,

    /// Free-form operational rules (the organism's own linter does not
    /// enforce these; they are documentation).
    #[serde(default)]
    pub rules: Vec<serde_yaml::Value>,

    /// Named external surfaces with their gate state.
    #[serde(default)]
    pub surface_registry: Vec<SurfaceRegistryEntry>,
}

/// The `pool:` identity block. Every field is `default` because the measured
/// variance is total: `tier` is absent from 459 of 620 glob-resolved
/// `_org.yaml` files, `role`/`domain` from 2, `ad_scope` from 209.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PoolBlock {
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(rename = "type", default)]
    pub pool_type: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub ad_scope: Option<String>,
    #[serde(default)]
    pub ad_prefix: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
    /// `null` on every top-level pool; a pool name on nested sub-systems.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub genome_layer: Option<String>,
    #[serde(default)]
    pub layer: Option<String>,
    #[serde(default)]
    pub status: Option<serde_yaml::Value>,
    #[serde(default)]
    pub graduation: Option<Graduation>,
}

/// Compass-graduate provenance (`_pool.yaml: compass_graduate` /
/// `_org.yaml: pool.graduation`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Graduation {
    #[serde(default)]
    pub status: Option<serde_yaml::Value>,
    #[serde(default)]
    pub path_ref: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub graduated_on: Option<String>,
    #[serde(default)]
    pub graduation_date: Option<String>,
    #[serde(default)]
    pub graduation_stage: Option<serde_yaml::Value>,
    #[serde(default)]
    pub parent_compass: Option<String>,
    #[serde(default)]
    pub self_reports_via: Option<String>,
    #[serde(default)]
    pub pooled_foundations: Vec<String>,
    #[serde(default)]
    pub shared_services: Vec<String>,
    #[serde(default)]
    pub strategy_refs: Vec<String>,
    /// The genome-template gate (`G1..G9: bool`). Keys are not a fixed set
    /// in the wild, so this is a map.
    #[serde(default)]
    pub genome_template_passed: BTreeMap<String, serde_yaml::Value>,
}

/// `department:` block (AXE-AD-011 Am.3).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Department {
    #[serde(default)]
    pub charter: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
}

/// A `services_offered` entry. The key is `id`; `name` is *not* a real field.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OfferedService {
    #[serde(default)]
    pub id: String,
    /// Executable entry point. The `_org.yaml` dialect uses `entry_point`,
    /// the legacy `_pool.yaml` dialect uses `interface` instead — and the
    /// only live legacy file (`presentation-engine`) uses `interface` with
    /// no `entry_point` at all. Both are carried; see
    /// [`OfferedService::contract`].
    #[serde(default)]
    pub entry_point: Option<String>,
    #[serde(default)]
    pub interface: Option<String>,
    /// Optional sub-system this service lives in. `(root)` is a sentinel
    /// used by 3 real services meaning "the pool root itself".
    #[serde(default)]
    pub sub_system: Option<String>,
    #[serde(default)]
    pub accepts: StringList,
    /// `returns` is a list in 62 real services and a bare string in 3
    /// (`meta-governance`, `research-vault`, `living-architecture` — all
    /// describing the service in prose). Both parse; a bare string becomes
    /// a single-element list rather than being dropped.
    #[serde(default)]
    pub returns: StringList,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    /// Legacy-dialect service-level objective (`presentation-engine`).
    #[serde(default)]
    pub sla: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

impl OfferedService {
    /// The executable contract for this service, whichever field carried it.
    ///
    /// AD-070: "Entry points are executable verbatim." Prefer the explicit
    /// `entry_point`; fall back to the legacy `interface` spelling.
    pub fn contract(&self) -> Option<&str> {
        self.entry_point.as_deref().or(self.interface.as_deref())
    }

    /// True when this service lives in the pool root rather than a
    /// sub-system. The `(root)` sentinel counts as root.
    pub fn is_root(&self) -> bool {
        match self.sub_system.as_deref() {
            None | Some("(root)") => true,
            Some(_) => false,
        }
    }
}

/// A `services_consumed` entry — the consumer edge. `access` is what decides
/// read-only (see [`AccessMode`]).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsumedService {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub purpose: Option<String>,
    /// Access mode. Absent in 18 of 72 real consumed edges; absent is
    /// treated as the least-privileged mode.
    #[serde(default)]
    pub access: Option<String>,
}

/// Keys that mark a consumer mapping as the STRUCT form rather than the
/// `{service: provider}` pair form.
const CONSUMED_STRUCT_KEYS: &[&str] = &["id", "provider"];

/// A `{service: provider}` map, as it appears in four real
/// `content-campaigns` sub-systems.
///
/// The real shape is always a single entry
/// (`feedback_engine: campaigns/feedback-engine`). A longer map is not
/// rejected — the compiler takes the first entry — because inventing a
/// failure mode the organism does not have would break a pool the organism
/// itself loads.
#[derive(Debug, Clone, Default)]
pub struct ServiceProviderPair {
    /// service name -> provider path, in document order.
    pub entry: Vec<(String, String)>,
}

impl ServiceProviderPair {
    /// The first `service: provider` pair, if any.
    pub fn first(&self) -> Option<(&str, &str)> {
        self.entry.first().map(|(s, p)| (s.as_str(), p.as_str()))
    }
}

/// Parse one `services_consumed` entry from an already-resolved
/// `serde_yaml::Value`.
///
/// Written by hand rather than via `#[serde(untagged)]` because untagged
/// cannot discriminate here: every field of `ConsumedService` defaults, so
/// the struct variant matches *everything* and beats the pair variant
/// regardless of ordering. The discriminator is key presence — a mapping
/// carrying `id:` or `provider:` is the struct form, anything else is a
/// `{service: provider}` pair.
fn parse_consumed_entry(value: &serde_yaml::Value) -> ConsumedEntry {
    match value {
        serde_yaml::Value::String(name) => ConsumedEntry::Bare(name.clone()),
        serde_yaml::Value::Mapping(m) => {
            let is_struct = m.keys().any(|k| {
                k.as_str()
                    .is_some_and(|s| CONSUMED_STRUCT_KEYS.contains(&s))
            });
            if is_struct {
                match serde_yaml::from_value::<ConsumedService>(value.clone()) {
                    Ok(s) => ConsumedEntry::Full(s),
                    // Unreachable for a well-formed struct, but never panic:
                    // an unexpected value must not take the process down.
                    Err(_) => ConsumedEntry::Full(ConsumedService::default()),
                }
            } else {
                ConsumedEntry::Pair(ServiceProviderPair {
                    entry: m
                        .iter()
                        .filter_map(|(k, v)| Some((k.as_str()?.to_string(), scalar_to_string(v)?)))
                        .collect(),
                })
            }
        }
        // A number/bool/null where a service is expected: name it by its own
        // text so the claim survives instead of vanishing.
        other => ConsumedEntry::Bare(scalar_to_string(other).unwrap_or_default()),
    }
}

/// A YAML scalar as the string the organism's reader would produce.
fn scalar_to_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Null => Some(String::new()),
        _ => None,
    }
}

/// Deserialize a `services_consumed` list, per entry. A null (or absent)
/// list means the pool consumes nothing.
fn de_services_consumed<'de, D>(d: D) -> Result<Vec<ConsumedEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = match Option::<Vec<serde_yaml::Value>>::deserialize(d)? {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };
    Ok(raw.iter().map(parse_consumed_entry).collect())
}

/// Deserialize a `services_offered` list, per entry.
///
/// Three real archived pools (`archive/scratch-test{,2,3}/_pool.yaml`) declare
/// `services_offered:` followed only by comment lines — a documented template
/// with nothing filled in. YAML reads that as a null scalar, which means "no
/// services", so it defaults to empty rather than failing.
fn de_services_offered<'de, D>(d: D) -> Result<Vec<OfferedEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = match Option::<Vec<serde_yaml::Value>>::deserialize(d)? {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };
    Ok(raw
        .iter()
        .map(|v| match v {
            serde_yaml::Value::String(name) => OfferedEntry::Bare(name.clone()),
            other => match serde_yaml::from_value::<OfferedService>(other.clone()) {
                Ok(s) => OfferedEntry::Full(s),
                Err(_) => OfferedEntry::Bare(scalar_to_string(other).unwrap_or_default()),
            },
        })
        .collect())
}

/// A `services_offered` entry, in either of the two real shapes.
///
/// 125 real entries are mappings (`id:` / `entry_point:` / `accepts:` …).
/// 4 are bare strings — `content-campaigns/sub-systems/{alignment-engine,
/// content-seeding, feedback-engine, research-protocols}/_org.yaml` each list
/// a bare service name under `services_offered:`. A bare name still declares
/// a claim, so it is not dropped; it is widened into a mapping whose `id` is
/// that name.
// `large_enum_variant`: the mapping form is the richer one by nature;
// boxing it would allocate on every parse of what is usually a bare
// string, to shave bytes off a type that is not hot.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum OfferedEntry {
    /// The normal mapping form.
    Full(OfferedService),
    /// A bare service name.
    Bare(String),
}

impl OfferedEntry {
    /// The service, in either form.
    pub fn service(&self) -> OfferedService {
        match self {
            OfferedEntry::Full(s) => s.clone(),
            OfferedEntry::Bare(id) => OfferedService {
                id: id.clone(),
                ..OfferedService::default()
            },
        }
    }

    /// The declared service id, whichever form carried it.
    pub fn id(&self) -> &str {
        match self {
            OfferedEntry::Full(s) => s.id.as_str(),
            OfferedEntry::Bare(s) => s.as_str(),
        }
    }
}

/// One `services_consumed` entry, in any of the three real shapes.
///
/// 71 real entries are mappings with `id:`/`provider:` fields; 1 is a bare
/// string (`strategy-incubator/sub-systems/forge/_org.yaml` lists
/// `trajectory`); and 4 are single-entry maps of the form
/// `service_name: pool/provider-path`
/// (`content-campaigns/sub-systems/{alignment-engine, content-seeding,
/// feedback-engine, research-protocols}/_org.yaml`). The single-entry map is
/// its own variant because `id`/`provider` are both absent there and the
/// struct would otherwise deserialize to an empty edge.
#[derive(Debug, Clone)]
pub enum ConsumedEntry {
    /// A bare provider name.
    Bare(String),
    /// A single-entry `{service: provider}` map.
    Pair(ServiceProviderPair),
    /// The normal mapping form, with explicit `id:`/`provider:` fields.
    Full(ConsumedService),
}

impl ConsumedEntry {
    /// The edge, in any form. A bare name is treated as the provider; a
    /// single-entry map is read as `service: provider`.
    pub fn service(&self) -> ConsumedService {
        match self {
            ConsumedEntry::Full(s) => s.clone(),
            ConsumedEntry::Pair(p) => match p.first() {
                Some((service, provider)) => ConsumedService {
                    id: service.to_string(),
                    provider: provider.to_string(),
                    ..ConsumedService::default()
                },
                None => ConsumedService::default(),
            },
            ConsumedEntry::Bare(p) => ConsumedService {
                id: p.clone(),
                provider: p.clone(),
                ..ConsumedService::default()
            },
        }
    }

    /// The declared provider, whichever form carried it.
    pub fn provider(&self) -> &str {
        match self {
            ConsumedEntry::Full(s) => {
                if s.provider.trim().is_empty() {
                    s.id.as_str()
                } else {
                    s.provider.as_str()
                }
            }
            ConsumedEntry::Pair(p) => p.first().map(|(_, provider)| provider).unwrap_or_default(),
            ConsumedEntry::Bare(s) => s.as_str(),
        }
    }
}

/// A YAML list of strings that may instead be a single bare string.
///
/// Measured across the live tree: `accepts` is always a list (67 edges), but
/// `returns` is a bare string in 3 real services — prose where the schema
/// expects a list. Refusing those 3 would break `meta-governance`,
/// `research-vault` and `living-architecture`, so the scalar form widens
/// into a one-element list instead.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum StringList {
    /// A sequence, the normal form.
    Many(Vec<String>),
    /// A single scalar, used by `returns` in 3 real manifests.
    One(String),
}

impl StringList {
    /// The values as a slice-like iterator.
    pub fn iter(&self) -> std::slice::Iter<'_, String> {
        match self {
            StringList::Many(v) => v.iter(),
            StringList::One(s) => std::slice::from_ref(s).iter(),
        }
    }

    /// The values as a plain vector.
    pub fn to_vec(&self) -> Vec<String> {
        self.iter().cloned().collect()
    }

    /// Number of values.
    pub fn len(&self) -> usize {
        match self {
            StringList::Many(v) => v.len(),
            StringList::One(_) => 1,
        }
    }

    /// True when there are no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for StringList {
    fn default() -> Self {
        StringList::Many(Vec::new())
    }
}

impl From<Vec<String>> for StringList {
    fn from(v: Vec<String>) -> Self {
        StringList::Many(v)
    }
}

/// `relationships:` block (AD-062).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Relationships {
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub loops: Vec<String>,
}

/// A pooled or native sub-system. Untagged so a file carrying BOTH shapes
/// (`presentation-engine` declares both, in two different shapes) parses.
// `large_enum_variant`: the `Detailed` variant is the big one by nature —
// boxing it would allocate on every parse of a shape that is usually a bare
// string, to save a few dozen bytes in a type that is not hot.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PooledSubSystem {
    /// `{name, target_pool, sub_system, note}` — the `_pool.yaml` dialect.
    Detailed(PooledSubSystemDetail),
    /// A bare name — the `_org.yaml` dialect.
    Name(String),
}

/// Structured form of [`PooledSubSystem`].
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PooledSubSystemDetail {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub target_pool: Option<String>,
    #[serde(default)]
    pub sub_system: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub version: Option<serde_yaml::Value>,
    #[serde(default)]
    pub note: Option<String>,
}

impl PooledSubSystem {
    /// The name this sub-system is known by, whichever shape it arrived in.
    pub fn name(&self) -> &str {
        match self {
            PooledSubSystem::Name(n) => n,
            PooledSubSystem::Detailed(d) => &d.name,
        }
    }

    /// True when the entry declares a symlink target (pooled organs are
    /// symlink-based per AD-060; native ones are real directories).
    pub fn is_pooled(&self) -> bool {
        match self {
            PooledSubSystem::Name(_) => false,
            PooledSubSystem::Detailed(d) => {
                matches!(d.kind.as_deref(), Some("pooled"))
                    || d.target.is_some()
                    || d.target_pool.is_some()
            }
        }
    }

    /// The declared target, when the entry carries one.
    pub fn target(&self) -> Option<&str> {
        match self {
            PooledSubSystem::Name(_) => None,
            PooledSubSystem::Detailed(d) => d
                .target
                .as_deref()
                .or(d.target_pool.as_deref())
                .or(d.sub_system.as_deref()),
        }
    }
}

/// A `surface_registry` entry.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SurfaceRegistryEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub gate: Option<SurfaceGate>,
}

/// A named surface's gate state.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SurfaceGate {
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    #[serde(default)]
    pub gate_ad: Option<String>,
}

// ---------------------------------------------------------------------------
// Access mode + write policy
// ---------------------------------------------------------------------------

/// How a consumer edge reaches a provider. Measured values across the real
/// tree: `cli` (41), `symlink` (16), `import` (11), `file` (2),
/// `symlink+cli` (4), and absent (24). Unrecognised values fall back to the
/// least-privileged mode rather than being rejected — a new access mode must
/// not silently become a write path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessMode {
    /// Reading through a data symlink (AD-060: pooled symlinks are read
    /// references, writes go through the provider's own CLI).
    Symlink,
    /// Driving the provider's CLI surface.
    Cli,
    /// Linking a library into the consumer's own process.
    Import,
    /// Reading a provider-owned file directly.
    File,
    /// Nothing declared. Least privilege.
    #[default]
    Undeclared,
    /// An access mode we do not recognize. Held read-only here, and refused
    /// outright by the compiler unless a [`WritePolicy`] grants it.
    Unknown,
    /// A write path. Never produced by [`AccessMode::parse`] — a manifest
    /// cannot declare its way into this variant. Only a host that
    /// constructs an [`AccessMode`] by hand may.
    Write,
}

impl AccessMode {
    /// Classify a raw `access:` string.
    pub fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return AccessMode::Undeclared;
        };
        // `symlink+cli` is a compound the organism actually uses: the edge
        // reads through a symlink AND drives the CLI. Both are read-shaped
        // for our purposes — a write goes through the provider's CLI under
        // its own approval, not through the consumer's pool row.
        let mut mode = AccessMode::Undeclared;
        for part in raw.split('+') {
            match part.trim() {
                "symlink" => mode = AccessMode::Symlink,
                "cli" => mode = AccessMode::Cli,
                "import" => mode = AccessMode::Import,
                "file" => mode = AccessMode::File,
                _ => return AccessMode::Unknown,
            }
        }
        if matches!(mode, AccessMode::Undeclared) {
            AccessMode::Unknown
        } else {
            mode
        }
    }

    /// True when the mode is a read path.
    ///
    /// Every mode the organism declares is a read path, because AD-060 §4
    /// puts writes behind the provider's own CLI contract. An *unclassified*
    /// mode is also read-only here — being unrecognized is not evidence of a
    /// write path — but [`compile_organism_in`] refuses to compile such an
    /// edge at all unless a [`WritePolicy`] grants it, so an unrecognized
    /// mode can never reach a mounted row.
    pub fn is_read_only(self) -> bool {
        !matches!(self, AccessMode::Write)
    }
}

// ---------------------------------------------------------------------------
// Legacy v1 schema (preserved for back-compat)
// ---------------------------------------------------------------------------

/// Legacy v1 pool manifest shape. Preserved verbatim: the struct fields,
/// their types, and their serde attributes are exactly what shipped, so
/// existing `PoolManifest` construction sites in other crates and tests keep
/// compiling unchanged.
///
/// The v1 dialect (`name` at top level, services keyed by `name`, dotted verb
/// service names) is a real published shape — it is what
/// `operant-harness/tests/hermes_pilot.rs` generates and what the original
/// Phase-6 plan described. It is not a schema any organism pool uses.
#[derive(Debug, Clone, Deserialize)]
pub struct PoolManifest {
    /// Pool display name.
    #[serde(default)]
    pub name: String,
    /// Services this pool offers to other pools / the loop. Each name
    /// becomes a Claim the kernel can depend on.
    #[serde(default)]
    pub services_offered: Vec<PoolService>,
    /// Services this pool needs from other pools. Each name becomes a
    /// `requires` claim on the family provider.
    #[serde(default)]
    pub services_consumed: Vec<PoolService>,
    /// Bundled sub-systems. Each becomes a `kind=pool.bundle`
    /// config_row that the host's boot pass routes to a read-only
    /// adapter.
    #[serde(default)]
    pub pooled_sub_systems: Vec<PoolSubSystem>,
}

/// Legacy v1 service entry — keyed by `name`, not `id`.
#[derive(Debug, Clone, Deserialize)]
pub struct PoolService {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// Legacy v1 sub-system entry.
#[derive(Debug, Clone, Deserialize)]
pub struct PoolSubSystem {
    pub name: String,
    pub path: String,
}

// ---------------------------------------------------------------------------
// Compiler output
// ---------------------------------------------------------------------------

/// A compiled service contract. The three fields that make a service
/// claim *executable* (AD-070) are carried, not dropped: the previous
/// compiler discarded `entry_point` / `accepts` / `returns` entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledService {
    /// The service id (real) or name (legacy). This is the claim key.
    pub id: String,
    /// Executable contract: `entry_point` or the legacy `interface`.
    pub contract: Option<String>,
    /// Accepted inputs.
    pub accepts: Vec<String>,
    /// Declared outputs.
    pub returns: Vec<String>,
    /// Owning sub-system, `(root)` sentinel normalized to `None`.
    pub sub_system: Option<String>,
    /// How the consumer edge reaches this service.
    pub access: AccessMode,
}

impl CompiledService {
    /// Whether this service is reachable without a write grant. True for
    /// every access mode the organism declares.
    pub fn is_read_only(&self) -> bool {
        self.access.is_read_only()
    }
}

/// A compiled sub-system bundle. One row per entry actually walked out of
/// `sub-systems/`, or per declared entry when no directory is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledSubSystem {
    pub name: String,
    /// Absolute (or caller-relative) path to the bundle.
    pub path: String,
    /// True when the entry is a symlink (AD-060 pooled organs are).
    /// The previous compiler hardcoded `read_only: true` without ever
    /// checking anything; provenance is carried instead.
    pub is_symlink: bool,
    /// Read-only verdict, with its provenance, so a consumer can tell an
    /// asserted property from a verified one.
    pub read_only: bool,
    /// How `read_only` was established.
    pub read_only_reason: &'static str,
}

/// Compiler output. Each entry is a row the kernel can mount.
#[derive(Debug, Clone)]
pub struct CompiledPool {
    pub name: String,
    pub family_row: ArchitectureRow,
    pub bundle_rows: Vec<ArchitectureRow>,
    pub claims: Vec<String>,
    /// Executable service contracts, parallel to nothing — consumers
    /// resolve by id via [`CompiledPool::service`].
    pub services: Vec<CompiledService>,
    /// Bundled sub-systems with provenance.
    pub sub_systems: Vec<CompiledSubSystem>,
    /// Which dialect this pool was compiled from.
    pub dialect: ManifestDialect,
    /// `pool.parent` — `None` for a top-level pool.
    pub parent: Option<String>,
}

impl CompiledPool {
    /// Look up a compiled service by its claim key.
    pub fn service(&self, id: &str) -> Option<&CompiledService> {
        self.services.iter().find(|s| s.id == id)
    }
}

/// Which manifest dialect a pool was compiled from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ManifestDialect {
    /// Real organism `_org.yaml` / `_pool.yaml` (nested `pool:` block,
    /// `id`-keyed services).
    #[default]
    Organism,
    /// Legacy v1 `_pool.yaml` (top-level `name`, `name`-keyed services,
    /// dotted verb names). Preserved for back-compat only.
    LegacyV1,
}

impl ManifestDialect {
    pub fn as_str(self) -> &'static str {
        match self {
            ManifestDialect::Organism => "organism",
            ManifestDialect::LegacyV1 => "legacy-v1",
        }
    }
}

// ---------------------------------------------------------------------------
// Write policy
// ---------------------------------------------------------------------------

/// The write-capability grant required before a pool row may assert a write
/// path.
///
/// Empty by default and **not constructible from YAML**: the only way to
/// widen it is [`WritePolicy::with_approval`], which demands an explicit
/// token. This is the "empty-by-default list that cannot be widened without
/// an explicit approval token" the design calls for — a manifest cannot
/// grant itself write access by declaring `access: write`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WritePolicy {
    capabilities: Vec<String>,
    approved_by: Option<String>,
}

impl WritePolicy {
    /// The default: no write capabilities, no approver.
    pub fn read_only() -> Self {
        Self::default()
    }

    /// Widen the policy, but only with an explicit non-empty approval token.
    /// Returns `None` when the token is empty — a blank approval is not an
    /// approval, and silently defaulting here would be a fail-open.
    pub fn with_approval(
        capabilities: impl IntoIterator<Item = String>,
        approved_by: impl Into<String>,
    ) -> Option<Self> {
        let approved_by = approved_by.into();
        if approved_by.trim().is_empty() {
            return None;
        }
        Some(Self {
            capabilities: capabilities.into_iter().collect(),
            approved_by: Some(approved_by),
        })
    }

    /// Whether `capability` has been granted.
    pub fn allows(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// The granted capabilities.
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// Who approved the widening, if anyone.
    pub fn approved_by(&self) -> Option<&str> {
        self.approved_by.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// How to read a manifest file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoadMode {
    /// Auto-detect: parse as the real organism schema, and fall back to the
    /// legacy v1 schema only when the real parse fails *and* the file looks
    /// like a v1 manifest (no `pool:` block, top-level `name`).
    #[default]
    Auto,
    /// Force the real organism schema. A v1 file fails to parse.
    Organism,
    /// Force the legacy v1 schema. A real file fails to parse.
    LegacyV1,
}

/// What a manifest file turned out to be.
#[derive(Debug, Clone)]
pub enum ParsedManifest {
    /// Real organism schema.
    Organism(Box<OrgManifest>),
    /// Legacy v1 schema.
    Legacy(Box<PoolManifest>),
}

impl ParsedManifest {
    /// Which dialect this is.
    pub fn dialect(&self) -> ManifestDialect {
        match self {
            ParsedManifest::Organism(_) => ManifestDialect::Organism,
            ParsedManifest::Legacy(_) => ManifestDialect::LegacyV1,
        }
    }

    /// Parse from a YAML string under an explicit mode.
    pub fn from_str_with(raw: &str, mode: LoadMode) -> Result<Self, HarnessError> {
        match mode {
            LoadMode::Organism => Ok(ParsedManifest::Organism(Box::new(parse_organism(raw)?))),
            LoadMode::LegacyV1 => Ok(ParsedManifest::Legacy(Box::new(parse_legacy(raw)?))),
            LoadMode::Auto => Self::from_str(raw),
        }
    }

    /// Parse from a YAML string, auto-detecting the dialect.
    ///
    /// Named `from_str` rather than implementing `FromStr` because the
    /// autodetecting form takes no extra input and this is a constructor,
    /// not a string conversion. The clippy `should_implement_trait` lint
    /// fires here; it is not an error worth renaming a stable public API
    /// for.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(raw: &str) -> Result<Self, HarnessError> {
        // Real schema first: it is the canonical one and it accepts every
        // legacy field it shares, so this only fails on a v1-only shape.
        match parse_organism(raw) {
            Ok(m) => Ok(ParsedManifest::Organism(Box::new(m))),
            Err(real_err) => {
                let looks_legacy = is_legacy_v1(raw);
                if !looks_legacy {
                    return Err(real_err);
                }
                parse_legacy(raw)
                    .map(|m| ParsedManifest::Legacy(Box::new(m)))
                    .map_err(|legacy_err| {
                        HarnessError::CompositionError(format!(
                            "manifest parses as neither the organism schema ({real_err}) \
                             nor the legacy v1 schema ({legacy_err})"
                        ))
                    })
            }
        }
    }
}

/// Cheap structural test for "is this a v1 manifest": a top-level `name:`
/// and no top-level `pool:` block. A real organism manifest always has
/// `pool:`; a v1 manifest never does.
fn is_legacy_v1(raw: &str) -> bool {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(raw) else {
        return false;
    };
    let Some(map) = value.as_mapping() else {
        return false;
    };
    let has_pool = map.contains_key(serde_yaml::Value::String("pool".into()));
    let has_name = map.contains_key(serde_yaml::Value::String("name".into()));
    !has_pool && has_name
}

/// Parse the real organism schema.
///
/// Tolerates duplicate mapping keys: 9 of the 126 physical manifests in the
/// live tree contain them. `serde_yaml` rejects a duplicate outright;
/// PyYAML — the reader the organism's own tooling is built on — resolves it
/// **last-wins**. Matching PyYAML is the only way to read the same file the
/// organism reads, so the collapse happens before the typed parse.
///
/// Two of the nine are genuinely divergent: `relationship-crm/sub-systems/
/// {contacts,outreach}/_org.yaml` declare `directories.core/.required`
/// as `false` and then `true`. PyYAML takes the `true`; so do we. Callers
/// that want the divergence surfaced rather than silently resolved use
/// [`parse_organism_strict`].
pub fn parse_organism(raw: &str) -> Result<OrgManifest, HarnessError> {
    // Collapse first: `serde_yaml` refuses duplicate keys even when parsing
    // into `Value`, and 9 real manifests carry them.
    let collapsed = collapse_duplicate_keys(raw)?;

    // Every field of `OrgManifest` is `#[serde(default)]`, so WITHOUT this
    // structural check a legacy v1 manifest (top-level `name:`, no `pool:`)
    // would parse "successfully" as an Organism with an empty name, and
    // autodetection would never reach the v1 parser. The `pool:` block is the
    // boundary between the two dialects; require it.
    let value: serde_yaml::Value = serde_yaml::from_str(&collapsed).map_err(|e| {
        HarnessError::CompositionError(format!("yaml parse organism manifest: {e}"))
    })?;
    let has_pool = value
        .as_mapping()
        .is_some_and(|m| m.contains_key(serde_yaml::Value::String("pool".into())));
    if !has_pool {
        return Err(HarnessError::CompositionError(
            "not an organism manifest: no top-level `pool:` block (this is the signature of a \
             legacy v1 manifest — parse it with `parse_legacy`)"
                .to_string(),
        ));
    }
    serde_yaml::from_str(&collapsed)
        .map_err(|e| HarnessError::CompositionError(format!("yaml parse organism manifest: {e}")))
}

/// Collapse duplicate mapping keys last-wins, as PyYAML does.
///
/// Returns the source text with duplicate keys already resolved. The text is
/// unparsed and re-emitted through `serde_yaml::Value`, so every value type
/// is preserved exactly; only the duplicated keys are removed.
pub fn collapse_duplicate_keys(raw: &str) -> Result<String, HarnessError> {
    match serde_yaml::from_str::<serde_yaml::Value>(raw) {
        Ok(value) => serde_yaml::to_string(&value).map_err(|e| {
            HarnessError::CompositionError(format!("yaml re-encode organism manifest: {e}"))
        }),
        // The document either has a hard syntax error or duplicate keys.
        // Duplicates are recoverable and must not surface as an error; a
        // genuine syntax error must. Distinguish them by whether the error
        // is specifically about a duplicate entry.
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("duplicate entry with key") || msg.contains("duplicate entry") {
                let (collapsed, conflicts) = collapse_dupes_by_line(raw);
                if conflicts.is_empty() && collapsed == raw {
                    // Duplicates we did not manage to rewrite: do not pretend.
                    return Err(HarnessError::CompositionError(format!(
                        "yaml parse organism manifest: {msg}"
                    )));
                }
                return serde_yaml::to_string(
                    &serde_yaml::from_str::<serde_yaml::Value>(&collapsed).map_err(|e| {
                        HarnessError::CompositionError(format!("yaml parse organism manifest: {e}"))
                    })?,
                )
                .map_err(|e| {
                    HarnessError::CompositionError(format!("yaml re-encode organism manifest: {e}"))
                });
            }
            Err(HarnessError::CompositionError(format!(
                "yaml parse organism manifest: {msg}"
            )))
        }
    }
}

/// Parse the real organism schema, rejecting any file whose duplicate keys
/// carry *different* values.
///
/// A duplicate with the same value twice is harmless. A duplicate that
/// disagrees with itself is a data-hygiene defect the organism should fix,
/// and a host running `arch check` gets to say so. Identical duplicates are
/// collapsed silently.
pub fn parse_organism_strict(raw: &str) -> Result<OrgManifest, HarnessError> {
    if let Some(conflict) = first_conflicting_duplicate(raw) {
        return Err(HarnessError::CompositionError(format!(
            "conflicting duplicate key in organism manifest: {conflict} (PyYAML would keep the \
             last; delete the earlier one)"
        )));
    }
    parse_organism(raw)
}

/// Report the first duplicate mapping key in `raw` whose two values differ,
/// as `path: first | second`.
pub fn first_conflicting_duplicate(raw: &str) -> Option<String> {
    duplicate_key_report(raw)
        .into_iter()
        .find(|(_, identical)| !identical)
        .map(|(path, _)| path)
}

/// Collapse duplicate mapping keys last-wins, by rewriting the text.
///
/// `serde_yaml` raises `duplicate entry with key` from the *parser*, and
/// `Deserializer::into_iter` raises the same error (measured) — there is no
/// lenient entry point in 0.9. PyYAML, the reader the organism's own tooling
/// is built on, resolves duplicates last-wins, so to read the same file the
/// organism reads we have to do the same.
///
/// The scan tracks real block structure (an indentation stack, with sequence
/// items contributing their own path segment) so a key only counts as a
/// duplicate when it repeats in the *same* mapping, never in a sibling list
/// item that happens to share an indent. The superseded line is replaced by
/// a comment, which preserves every subsequent line number for diagnostics.
///
/// Returns the rewritten text plus every divergence as `(path, identical)`.
/// If the superseded value was a multi-line block scalar, commenting it out
/// would orphan its continuation lines, so the text is returned UNCHANGED
/// and the caller surfaces the error rather than guessing.
fn collapse_dupes_by_line(raw: &str) -> (String, Vec<(String, bool)>) {
    const SUPERSEDED: &str = "# [operant] superseded duplicate key; last value wins (as PyYAML)";
    let lines: Vec<&str> = raw.lines().collect();
    // `None` = drop the line entirely (never used; every case comments).
    let mut out: Vec<&str> = lines.clone();
    // Open blocks, innermost last: (indent, path segment).
    let mut stack: Vec<(usize, String)> = Vec::new();
    // Dotted path -> (line index, value text).
    let mut seen: std::collections::HashMap<String, (usize, &str)> =
        std::collections::HashMap::new();
    let mut conflicts: Vec<(String, bool)> = Vec::new();
    let mut bail = false;

    for (i, line) in lines.iter().enumerate() {
        let indent = line.len() - line.trim_start().len();
        let body = line.trim_start();

        // A sequence item. `- key: v` opens a block owned by that item, so two
        // entries of the same list never collide on their nested keys.
        if body == "-" || body.starts_with("- ") {
            if let Some(rest) = body.strip_prefix("- ") {
                match split_mapping_key(rest) {
                    Some((key, value)) if opens_block(value) => {
                        stack.push((indent, format!("[{key}]")));
                        continue;
                    }
                    Some((key, _)) => {
                        stack.push((indent, format!("[{key}]")));
                        continue;
                    }
                    None => {}
                }
            }
            stack.push((indent, "[-]".to_string()));
            continue;
        }
        // Blanks, comments, and block-scalar headers open no mapping.
        if body.is_empty() || body.starts_with('#') || is_block_scalar_header(body) {
            continue;
        }
        let Some((key, value)) = split_mapping_key(body) else {
            continue;
        };

        // Close every block at or deeper than this line.
        while stack.last().is_some_and(|(ind, _)| *ind >= indent) {
            stack.pop();
        }
        let path = format!(
            "{}.{}",
            stack
                .iter()
                .map(|(_, seg)| seg.as_str())
                .collect::<Vec<_>>()
                .join("."),
            key
        );

        if let Some((prev_i, prev_value)) = seen.get(path.as_str()).copied() {
            let identical = prev_value == value;
            conflicts.push((path.clone(), identical));
            if is_block_scalar_header(prev_value.trim_start()) {
                bail = true;
                continue;
            }
            out[prev_i] = SUPERSEDED;
        }
        seen.insert(path, (i, value));

        if opens_block(value) {
            stack.push((indent, key.to_string()));
        }
    }

    if bail {
        return (raw.to_string(), conflicts);
    }
    let mut text = out.join("\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    (text, conflicts)
}

/// Split `key: value`, accepting only plain keys. Quoted keys, flow
/// collections, and anchors are not mapping keys and are skipped.
fn split_mapping_key(body: &str) -> Option<(&str, &str)> {
    let (key, value) = body.split_once(':')?;
    let key = key.trim_end();
    if key.is_empty()
        || !key.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | '+' | '*' | '@' | ' ')
        })
    {
        return None;
    }
    Some((key, value.trim()))
}

/// True when the value is empty, so the key opens a nested block.
fn opens_block(value: &str) -> bool {
    value.is_empty()
}

/// True when the line's value is a YAML block scalar (`|` / `>`), whose
/// content lives on later lines and cannot be commented out alone.
fn is_block_scalar_header(value: &str) -> bool {
    let v = value.trim_start();
    v == "|"
        || v == ">"
        || v.starts_with("|-")
        || v.starts_with(">-")
        || v.starts_with("|+")
        || v.starts_with(">+")
}

/// Every duplicate mapping key found in `raw`, as `(path, identical)`.
pub fn duplicate_key_report(raw: &str) -> Vec<(String, bool)> {
    // The clean fast path: `serde_yaml` accepted it, so there are none.
    if serde_yaml::from_str::<serde_yaml::Value>(raw).is_ok() {
        return Vec::new();
    }
    collapse_dupes_by_line(raw).1
}

/// Parse the legacy v1 schema.
pub fn parse_legacy(raw: &str) -> Result<PoolManifest, HarnessError> {
    serde_yaml::from_str(raw)
        .map_err(|e| HarnessError::CompositionError(format!("yaml parse legacy manifest: {e}")))
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

/// Compile a real organism manifest.
///
/// `policy` decides write access. The default (empty) policy allows every
/// access mode the organism declares, because every declared mode is a read
/// path (AD-060 §4: pooled symlinks are read references; writes go through
/// the provider's own CLI contract).
pub fn compile_organism(
    manifest: &OrgManifest,
    policy: &WritePolicy,
) -> Result<CompiledPool, HarnessError> {
    compile_organism_in(manifest, policy, None)
}

/// As [`compile_organism`], but with the pool's on-disk directory so
/// sub-system bundling can walk `sub-systems/` for real.
pub fn compile_organism_in(
    manifest: &OrgManifest,
    policy: &WritePolicy,
    pool_dir: Option<&Path>,
) -> Result<CompiledPool, HarnessError> {
    let name = manifest.pool.name.trim().to_string();
    if name.is_empty() {
        return Err(HarnessError::CompositionError(
            "organism manifest has no `pool.name` — every real manifest nests the \
             identity block under `pool:`, so a top-level `name` is the signature \
             of a legacy v1 manifest (parse it with `compile_legacy`)"
                .to_string(),
        ));
    }

    // --- services_offered -> claims, with contracts carried ---------------
    let mut services: Vec<CompiledService> = Vec::new();
    let mut claims: Vec<String> = Vec::new();
    for entry in &manifest.services_offered {
        let svc = entry.service();
        let id = svc.id.trim();
        if id.is_empty() {
            return Err(HarnessError::CompositionError(format!(
                "pool `{name}` has a services_offered entry with an empty `id`"
            )));
        }
        // The v1 verb gate stays as an ADDITIONAL check, applied only to
        // ids that actually look verb-style. No real organism service id
        // contains a `.` (measured: 0 of 126 manifests), so this is inert
        // on real input and load-bearing on v1 input.
        if looks_verb_style(id) && !READ_ONLY_VERBS.iter().any(|v| id.starts_with(v)) {
            return Err(HarnessError::CompositionError(format!(
                "pool `{name}` offers service `{id}` with a dotted verb-style name that \
                 is not a read-only verb ({})",
                READ_ONLY_VERBS.join(", ")
            )));
        }
        if claims.contains(&id.to_string()) {
            return Err(HarnessError::CompositionError(format!(
                "pool `{name}` declares service `{id}` twice"
            )));
        }
        claims.push(id.to_string());
        services.push(CompiledService {
            id: id.to_string(),
            contract: svc.contract().map(str::to_string),
            accepts: svc.accepts.to_vec(),
            returns: svc.returns.to_vec(),
            sub_system: svc
                .sub_system
                .as_deref()
                .filter(|s| *s != "(root)")
                .map(str::to_string),
            // An offered service has no consumer edge of its own; the pool
            // root is the only accessor until a consumer declares an access
            // mode for it.
            access: AccessMode::Undeclared,
        });
    }

    // --- services_consumed -> requires ------------------------------------
    let mut requires: Vec<String> = Vec::new();
    for entry in &manifest.services_consumed {
        let edge = entry.service();
        let edge = &edge;
        let mode = AccessMode::parse(edge.access.as_deref());
        // Fail-closed: an access mode the organism has not classified cannot
        // be treated as a read path AND it cannot silently become a write
        // path either. It needs an explicit grant.
        if matches!(mode, AccessMode::Unknown) && !policy.allows(&format!("access:{}", edge.id)) {
            return Err(HarnessError::CompositionError(format!(
                "pool `{name}` consumes `{}` with unclassified access mode `{}`; \
                 classify it in the organism or grant it via WritePolicy::with_approval",
                edge.id,
                edge.access.as_deref().unwrap_or("<empty>")
            )));
        }
        let key = if edge.id.trim().is_empty() {
            edge.provider.trim()
        } else {
            edge.id.trim()
        };
        if key.is_empty() {
            return Err(HarnessError::CompositionError(format!(
                "pool `{name}` has a services_consumed entry with neither `id` nor `provider`"
            )));
        }
        if !requires.contains(&key.to_string()) {
            requires.push(key.to_string());
        }
    }

    // --- sub-systems: walk the filesystem, fall back to declarations ------
    let sub_systems = collect_sub_systems(manifest, pool_dir, policy);
    let bundle_rows: Vec<ArchitectureRow> = sub_systems
        .iter()
        .map(|s| ArchitectureRow {
            id: format!("pool.{}.{}", name, s.name),
            source: "pool".to_string(),
            disabled: false,
            config: serde_json::json!({
                "path": s.path,
                "read_only": s.read_only,
                "read_only_reason": s.read_only_reason,
                "symlink": s.is_symlink,
            }),
            kind: Some("pool.bundle".to_string()),
        })
        .collect();

    let family_row = ArchitectureRow {
        id: format!("pool.{name}"),
        source: "pool".to_string(),
        disabled: false,
        config: serde_json::json!({
            "name": name,
            "claims": claims,
            "requires": requires,
            "dialect": ManifestDialect::Organism.as_str(),
            "tier": manifest.pool.tier,
            "category": manifest.pool.category,
            "role": manifest.pool.role,
            "domain": manifest.pool.domain,
            "type": manifest.pool.pool_type,
            "ad_scope": manifest.pool.ad_scope,
            "parent": manifest.pool.parent,
            "department_class": manifest
                .department
                .as_ref()
                .and_then(|d| d.class.clone()),
            "services": services
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "contract": s.contract,
                        "accepts": s.accepts,
                        "returns": s.returns,
                        "sub_system": s.sub_system,
                        "access": format!("{:?}", s.access).to_lowercase(),
                    })
                })
                .collect::<Vec<_>>(),
            "relationships": {
                "depends_on": manifest.relationships.depends_on,
                "loops": manifest.relationships.loops,
            },
            "required_files": manifest.required_files,
            "forbidden": manifest.forbidden,
        }),
        kind: Some("pool.family".to_string()),
    };

    Ok(CompiledPool {
        name,
        family_row,
        bundle_rows,
        claims,
        services,
        sub_systems,
        dialect: ManifestDialect::Organism,
        parent: manifest.pool.parent.clone(),
    })
}

/// A service id that looks like the v1 dotted-verb style.
fn looks_verb_style(id: &str) -> bool {
    id.contains('.')
}

/// Enumerate sub-systems for bundling.
///
/// AD-060: pooling is expressed as exact-name symlinks inside
/// `sub-systems/`. When the pool directory is available we walk it and emit
/// one row per entry actually on disk — including native organs, which the
/// declared-field model missed. When it is not available (an in-memory
/// manifest, as in a unit test) we fall back to the declared fields.
fn collect_sub_systems(
    manifest: &OrgManifest,
    pool_dir: Option<&Path>,
    policy: &WritePolicy,
) -> Vec<CompiledSubSystem> {
    if let Some(dir) = pool_dir {
        let sub_dir = dir.join("sub-systems");
        if let Ok(entries) = std::fs::read_dir(&sub_dir) {
            let mut out: Vec<CompiledSubSystem> = entries
                .flatten()
                .filter(|e| {
                    let n = e.file_name();
                    let n = n.to_string_lossy();
                    !n.starts_with('.') && !n.starts_with('_')
                })
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let path = e.path();
                    let is_symlink = path
                        .symlink_metadata()
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);
                    CompiledSubSystem {
                        name,
                        path: path.to_string_lossy().to_string(),
                        is_symlink,
                        read_only: true,
                        read_only_reason: "ad-060-pooled-symlink-is-a-read-reference",
                    }
                })
                .collect();
            out.sort_by(|a, b| a.name.cmp(&b.name));
            return out;
        }
    }

    // No directory to walk: use the declared fields. `pooled_sub_systems`
    // first (they are the symlinked organs), then `sub_systems`.
    let mut out: Vec<CompiledSubSystem> = Vec::new();
    for entry in manifest
        .pooled_sub_systems
        .iter()
        .chain(manifest.sub_systems.iter())
    {
        let name = entry.name().to_string();
        if name.is_empty() || out.iter().any(|s| s.name == name) {
            continue;
        }
        out.push(CompiledSubSystem {
            name,
            path: entry.target().unwrap_or_default().to_string(),
            is_symlink: entry.is_pooled(),
            read_only: true,
            read_only_reason: "ad-060-declared-pooled-symlink",
        });
    }
    let _ = policy; // policy does not widen a bundle to a write path
    out
}

/// Compile a legacy v1 manifest. Behaviour is unchanged from the shipped
/// compiler, including the v1 read-only verb gate.
pub fn compile_legacy(manifest: &PoolManifest) -> Result<CompiledPool, HarnessError> {
    if manifest.name.is_empty() {
        return Err(HarnessError::CompositionError(
            "pool manifest has empty `name`".to_string(),
        ));
    }
    // Validate offered services are read-only verbs
    for svc in &manifest.services_offered {
        let n = &svc.name;
        if !READ_ONLY_VERBS.iter().any(|v| n.starts_with(v)) {
            return Err(HarnessError::CompositionError(format!(
                "pool `{}` offers non-read-only service `{}`; v1 is read-only",
                manifest.name, n
            )));
        }
    }
    // Build the family row
    let mut claims: Vec<String> = Vec::new();
    for svc in &manifest.services_offered {
        claims.push(svc.name.clone());
    }
    let requires: Vec<String> = manifest
        .services_consumed
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let family_id = format!("pool.{}", manifest.name);
    let family_row = ArchitectureRow {
        id: family_id.clone(),
        source: "pool".to_string(),
        disabled: false,
        config: serde_json::json!({
            "name": manifest.name,
            "claims": claims,
            "requires": requires,
            "dialect": ManifestDialect::LegacyV1.as_str(),
        }),
        kind: Some("pool.family".to_string()),
    };
    // Build the bundle rows
    let mut bundle_rows = Vec::new();
    for sub in &manifest.pooled_sub_systems {
        bundle_rows.push(ArchitectureRow {
            id: format!("pool.{}.{}", manifest.name, sub.name),
            source: "pool".to_string(),
            disabled: false,
            config: serde_json::json!({
                "path": sub.path,
                "read_only": true,
                "read_only_reason": "legacy-v1-default-read-only",
                "symlink": false,
            }),
            kind: Some("pool.bundle".to_string()),
        });
    }
    let services: Vec<CompiledService> = manifest
        .services_offered
        .iter()
        .map(|s| CompiledService {
            id: s.name.clone(),
            contract: None,
            accepts: Vec::new(),
            returns: Vec::new(),
            sub_system: None,
            access: AccessMode::Undeclared,
        })
        .collect();
    let sub_systems: Vec<CompiledSubSystem> = manifest
        .pooled_sub_systems
        .iter()
        .map(|s| CompiledSubSystem {
            name: s.name.clone(),
            path: s.path.clone(),
            is_symlink: false,
            read_only: true,
            read_only_reason: "legacy-v1-default-read-only",
        })
        .collect();
    Ok(CompiledPool {
        name: manifest.name.clone(),
        family_row,
        bundle_rows,
        claims,
        services,
        sub_systems,
        dialect: ManifestDialect::LegacyV1,
        parent: None,
    })
}

// ---------------------------------------------------------------------------
// Back-compat entry points
// ---------------------------------------------------------------------------

/// Compile a legacy v1 manifest. **Unchanged signature and behaviour** —
/// this is the function `operant-core` and the CLI already call.
pub fn compile(manifest: &PoolManifest) -> Result<CompiledPool, HarnessError> {
    compile_legacy(manifest)
}

/// Load a manifest from disk and compile it, auto-detecting the dialect.
///
/// This is the drop-in replacement for the shipped
/// `load_and_compile`: same signature, same return type, and it now also
/// handles real organism manifests. When `path` points at a file inside a
/// pool directory, that directory is used to walk `sub-systems/`.
pub fn load_and_compile(path: &Path) -> Result<CompiledPool, HarnessError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        HarnessError::CompositionError(format!("read pool manifest {}: {e}", path.display()))
    })?;
    load_and_compile_str(&raw, path)
}

/// As [`load_and_compile`], but from an in-memory string. `origin` is the
/// path the bytes came from — used only to locate the pool directory.
pub fn load_and_compile_str(raw: &str, origin: &Path) -> Result<CompiledPool, HarnessError> {
    match ParsedManifest::from_str(raw)? {
        ParsedManifest::Organism(m) => {
            let dir = pool_dir_for(origin);
            compile_organism_in(&m, &WritePolicy::read_only(), dir.as_deref())
        }
        ParsedManifest::Legacy(m) => compile_legacy(&m),
    }
}

/// The directory a manifest file lives in, if the file name looks like a
/// pool manifest.
pub fn pool_dir_for(manifest_path: &Path) -> Option<PathBuf> {
    let name = manifest_path.file_name()?.to_str()?;
    if name == ORG_CONFIG_NAME || name == LEGACY_CONFIG_NAME {
        manifest_path.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// Compile an organism manifest with a specific write policy and a known
/// pool directory. The full-control entry point for hosts.
pub fn compile_pool_dir(path: &Path, policy: &WritePolicy) -> Result<CompiledPool, HarnessError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        HarnessError::CompositionError(format!("read pool manifest {}: {e}", path.display()))
    })?;
    let parsed = ParsedManifest::from_str(&raw)?;
    let dir = pool_dir_for(path);
    match parsed {
        ParsedManifest::Organism(m) => compile_organism_in(&m, policy, dir.as_deref()),
        ParsedManifest::Legacy(m) => compile_legacy(&m),
    }
}

// ---------------------------------------------------------------------------
// M2 — the registry is derived, never hand-edited (AD-061)
// ---------------------------------------------------------------------------

/// The derived architecture registry (AD-061). `arch emit` regenerates it
/// from every pool's own manifest; it is a build artifact, so it cannot lie
/// — but only if something checks that it was rebuilt after the last
/// manifest edit. That check is [`check_registry_freshness`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryFreshness {
    /// `true` when the registry is present and at least as new as every
    /// manifest it covers.
    pub fresh: bool,
    /// Why not, when `fresh` is false.
    pub reason: Option<String>,
}

/// Check whether a derived registry is current with respect to a set of
/// manifests (AD-061 M2).
///
/// The organism's own rule: WARN when the registry is **missing**, or when
/// it is **older than any top-level manifest**. Operant is stricter — a stale
/// registry is a REJECT, because in operant the registry is what the kernel
/// routes on, and a stale map silently points a claim at the wrong pool.
/// That is the M2 mechanism turned from a warning into a gate.
///
/// `manifests` are the manifest files the registry claims to cover.
/// `registry` is the derived registry file itself.
pub fn check_registry_freshness(
    registry: &Path,
    manifests: &[PathBuf],
) -> Result<RegistryFreshness, HarnessError> {
    let registry_mtime = std::fs::metadata(registry)
        .map_err(|e| {
            HarnessError::CompositionError(format!(
                "architectural registry {} is unreadable: {e}",
                registry.display()
            ))
        })?
        .modified()
        .map_err(|e| {
            HarnessError::CompositionError(format!(
                "architectural registry {} has no mtime: {e}",
                registry.display()
            ))
        })?;

    let mut oldest_name = String::new();
    let mut newest: Option<std::time::SystemTime> = None;
    for path in manifests {
        let Ok(md) = std::fs::metadata(path) else {
            continue;
        };
        let Ok(mtime) = md.modified() else { continue };
        if newest.is_none_or(|n| mtime > n) {
            newest = Some(mtime);
            oldest_name = path.display().to_string();
        }
    }

    match newest {
        // Registry older than a manifest: the derived map can no longer be
        // trusted for that pool.
        Some(newest_manifest) if registry_mtime < newest_manifest => Ok(RegistryFreshness {
            fresh: false,
            reason: Some(format!(
                "architectural registry {} is older than manifest {} (AD-061: the registry \
                 is derived — re-emit it after editing a manifest)",
                registry.display(),
                oldest_name
            )),
        }),
        _ => Ok(RegistryFreshness {
            fresh: true,
            reason: None,
        }),
    }
}

/// Reject a manifest whose declared registry is stale (M2 parity).
///
/// This is the same check AD-061 makes, promoted from WARN to a hard
/// refusal: compiling a manifest into rows that a stale registry contradicts
/// would emit a provider tree that disagrees with the organism's own map.
pub fn require_fresh_registry(registry: &Path, manifests: &[PathBuf]) -> Result<(), HarnessError> {
    let freshness = check_registry_freshness(registry, manifests)?;
    if freshness.fresh {
        return Ok(());
    }
    Err(HarnessError::CompositionError(format!(
        "registry staleness (AD-061 M2): {}",
        freshness.reason.unwrap_or_default()
    )))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn manifest_yaml() -> &'static str {
        r#"
name: relationship-intel
services_offered:
  - name: query.contacts
    description: Find contacts by name/email
  - name: search.history
    description: Search communication history
services_consumed:
  - name: auth.identity
    description: Identity provider
pooled_sub_systems:
  - name: contacts
    path: ~/.hermes/systems/relationship-intel/contacts
  - name: history
    path: ~/.hermes/systems/relationship-intel/history
"#
    }

    #[test]
    fn compiles_read_only_pool() {
        let m: PoolManifest = serde_yaml::from_str(manifest_yaml()).unwrap();
        let compiled = compile(&m).unwrap();
        assert_eq!(compiled.name, "relationship-intel");
        assert_eq!(compiled.claims, vec!["query.contacts", "search.history"]);
        assert_eq!(compiled.family_row.id, "pool.relationship-intel");
        assert_eq!(compiled.family_row.source, "pool");
        assert_eq!(compiled.family_row.kind.as_deref(), Some("pool.family"));
        assert_eq!(compiled.bundle_rows.len(), 2);
        assert_eq!(compiled.bundle_rows[0].kind.as_deref(), Some("pool.bundle"));
        assert_eq!(
            compiled.bundle_rows[0]
                .config
                .get("read_only")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(compiled.dialect, ManifestDialect::LegacyV1);
    }

    #[test]
    fn rejects_write_verbs() {
        let yaml = r#"
name: bad-pool
services_offered:
  - name: delete.contacts
    description: Delete a contact (forbidden in v1)
"#;
        let m: PoolManifest = serde_yaml::from_str(yaml).unwrap();
        let err = compile(&m).unwrap_err();
        assert!(err.to_string().contains("non-read-only"));
        assert!(err.to_string().contains("delete.contacts"));
    }

    #[test]
    fn rejects_empty_name() {
        let yaml = "name: \"\"";
        let m: PoolManifest = serde_yaml::from_str(yaml).unwrap();
        let err = compile(&m).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn load_and_compile_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("_pool.yaml");
        std::fs::write(&path, manifest_yaml()).unwrap();
        let compiled = load_and_compile(&path).unwrap();
        assert_eq!(compiled.name, "relationship-intel");
        assert_eq!(compiled.dialect, ManifestDialect::LegacyV1);
    }

    // --- real-schema unit tests (fixtures live in tests/pool_real_manifests.rs)

    #[test]
    fn parses_nested_org_pool_block() {
        let raw = r#"
pool:
  name: task-grid
  type: system
  tier: swarm
  category: task-execution
  ad_scope: AXE-AD-001
  parent: null
  role: swarm
  domain: task-execution
services_offered:
- id: axe-feedback
  sub_system: (root)
  entry_point: 'Python: scripts/axe_feedback.py'
  accepts: [component, summary]
services_consumed:
- id: infra-utilities
  provider: platform-infra
  purpose: needs infra utilities
  access: import
relationships:
  depends_on: [platform-infra]
  loops: [platform-infra]
required_files: [AGENTS.md]
forbidden: ['__pycache__/']
department:
  charter: 'AXE agent command center'
  class: staffed
"#;
        let m = parse_organism(raw).unwrap();
        assert_eq!(m.pool.name, "task-grid");
        assert_eq!(m.pool.tier.as_deref(), Some("swarm"));
        assert_eq!(m.services_offered[0].id(), "axe-feedback");
        assert!(m.services_offered[0].service().is_root());
        assert_eq!(
            m.services_offered[0].service().contract(),
            Some("Python: scripts/axe_feedback.py")
        );
        assert_eq!(
            m.services_consumed[0].service().access.as_deref(),
            Some("import")
        );
        assert_eq!(m.relationships.depends_on, vec!["platform-infra"]);
        assert_eq!(m.relationships.loops, vec!["platform-infra"]);
        assert_eq!(m.department.unwrap().class.as_deref(), Some("staffed"));
    }

    #[test]
    fn legacy_and_real_dialects_are_distinguished() {
        let v1 = ParsedManifest::from_str(manifest_yaml()).unwrap();
        assert_eq!(v1.dialect(), ManifestDialect::LegacyV1);
        let real = ParsedManifest::from_str("pool:\n  name: x\n").unwrap();
        assert_eq!(real.dialect(), ManifestDialect::Organism);
    }

    #[test]
    fn real_compile_carries_contract_fields() {
        let raw = r#"
pool:
  name: identity-core
  type: system
  tier: foundational
  category: identity
  ad_scope: ID-AD-001
  parent: null
  role: foundational
  domain: identity
services_offered:
- id: identity
  entry_point: "CLI: identity"
  accepts: [section]
  returns: [identity_doc]
- id: values
  entry_point: 'CLI: identity values'
  accepts: []
  returns: [values_md]
services_consumed:
- id: logbook
  provider: flight-ledger
  purpose: subjective channel intake
  access: cli
"#;
        let m = parse_organism(raw).unwrap();
        let c = compile_organism(&m, &WritePolicy::read_only()).unwrap();
        assert_eq!(c.name, "identity-core");
        assert_eq!(c.claims, vec!["identity", "values"]);
        let values = c.service("values").unwrap();
        assert_eq!(values.contract.as_deref(), Some("CLI: identity values"));
        assert!(values.accepts.is_empty());
        assert_eq!(values.returns, vec!["values_md"]);
        // contract fields survive into the emitted row
        let row = &c.family_row.config;
        let svc = row["services"][0].as_object().unwrap();
        assert_eq!(svc["id"], "identity");
        assert_eq!(svc["contract"], "CLI: identity");
        assert_eq!(row["requires"][0], "logbook");
        assert_eq!(c.parent, None);
    }

    #[test]
    fn interface_is_used_when_entry_point_absent() {
        let raw = r#"
pool:
  name: presentation-engine
  role: foundational
  ad_prefix: PR-AD
services_offered:
  - id: pe-render
    description: Render an artifact
    interface: "pe render <type> <spec.json>"
    sla: "< 5s"
"#;
        let m = parse_organism(raw).unwrap();
        let c = compile_organism(&m, &WritePolicy::read_only()).unwrap();
        let r = c.service("pe-render").unwrap();
        assert_eq!(r.contract.as_deref(), Some("pe render <type> <spec.json>"));
    }

    #[test]
    fn access_modes_classify_from_the_real_tree() {
        assert_eq!(AccessMode::parse(Some("cli")), AccessMode::Cli);
        assert_eq!(AccessMode::parse(Some("symlink")), AccessMode::Symlink);
        assert_eq!(AccessMode::parse(Some("import")), AccessMode::Import);
        assert_eq!(AccessMode::parse(Some("file")), AccessMode::File);
        assert_eq!(AccessMode::parse(Some("symlink+cli")), AccessMode::Cli);
        assert_eq!(AccessMode::parse(None), AccessMode::Undeclared);
        assert_eq!(AccessMode::parse(Some("")), AccessMode::Unknown);
        assert_eq!(AccessMode::parse(Some("shellout")), AccessMode::Unknown);
        // fail-closed: an unclassified mode is read-only, not write
        assert!(AccessMode::Unknown.is_read_only());
    }

    #[test]
    fn write_policy_cannot_be_widened_without_a_token() {
        assert!(WritePolicy::with_approval(vec!["w".into()], "  ").is_none());
        let p = WritePolicy::with_approval(vec!["access:weird".into()], "AD-1").unwrap();
        assert!(p.allows("access:weird"));
        assert_eq!(p.approved_by(), Some("AD-1"));
        // the default is empty and grants nothing
        let d = WritePolicy::read_only();
        assert!(!d.allows("access:weird"));
        assert!(d.capabilities().is_empty());
        assert!(d.approved_by().is_none());
    }

    #[test]
    fn unclassified_access_is_rejected_without_approval() {
        let raw = r#"
pool:
  name: weird
  type: system
services_consumed:
  - id: thing
    provider: elsewhere
    purpose: unknown mode
    access: shellout
"#;
        let m = parse_organism(raw).unwrap();
        let err = compile_organism(&m, &WritePolicy::read_only()).unwrap_err();
        assert!(err.to_string().contains("unclassified access mode"));
        // with an explicit approval token it compiles
        let policy = WritePolicy::with_approval(vec!["access:thing".into()], "AD-99").unwrap();
        let ok = compile_organism(&m, &policy).unwrap();
        assert_eq!(ok.name, "weird");
    }

    #[test]
    fn duplicate_service_ids_are_rejected() {
        let raw = r#"
pool:
  name: dup
  type: system
services_offered:
  - id: same
    entry_point: a
  - id: same
    entry_point: b
"#;
        let m = parse_organism(raw).unwrap();
        let err = compile_organism(&m, &WritePolicy::read_only()).unwrap_err();
        assert!(err.to_string().contains("twice"));
    }

    #[test]
    fn missing_pool_name_names_the_real_cause() {
        // `pool.name` is REQUIRED (unlike every other field), so the parse
        // itself refuses a nameless pool — and says which field is missing.
        let err = parse_organism("pool:\n  type: system\nservices_offered: []\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing field `name`"), "got: {msg}");
        assert!(msg.contains("pool"), "got: {msg}");
    }

    /// The dialect boundary is the presence of the `pool:` block. A file
    /// without one is a legacy v1 manifest, and must be refused as such
    /// rather than parsed into an Organism with an empty name.
    #[test]
    fn no_pool_block_is_refused_as_the_legacy_dialect() {
        let err = parse_organism("services_offered: []\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no top-level `pool:` block"), "got: {msg}");
        assert!(msg.contains("legacy v1"), "got: {msg}");
    }

    #[test]
    fn dotted_verb_gate_fires_only_on_verb_style_ids() {
        let raw = r#"
pool:
  name: v1ish
  type: system
services_offered:
  - id: delete.contacts
    entry_point: nope
"#;
        let m = parse_organism(raw).unwrap();
        let err = compile_organism(&m, &WritePolicy::read_only()).unwrap_err();
        assert!(err.to_string().contains("read-only verb"));
        // a real slug passes
        let ok = compile_organism(
            &parse_organism(
                "pool:\n  name: ok\nservices_offered:\n  - id: axe-feedback\n    entry_point: x\n",
            )
            .unwrap(),
            &WritePolicy::read_only(),
        )
        .unwrap();
        assert_eq!(ok.claims, vec!["axe-feedback"]);
    }

    #[test]
    fn bundles_come_from_sub_systems_directory() {
        let dir = tempfile::tempdir().unwrap();
        let pool = dir.path().join("pool-x");
        std::fs::create_dir_all(pool.join("sub-systems")).unwrap();
        std::fs::create_dir_all(pool.join("sub-systems/native")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            dir.path().join("elsewhere"),
            pool.join("sub-systems/pooled"),
        )
        .unwrap();
        let raw = "pool:\n  name: pool-x\n  type: system\n";
        let m = parse_organism(raw).unwrap();
        let c = compile_organism_in(&m, &WritePolicy::read_only(), Some(&pool)).unwrap();
        let names: Vec<&str> = c.sub_systems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["native", "pooled"]);
        // provenance: the symlink is reported as a symlink
        let pooled = c.sub_systems.iter().find(|s| s.name == "pooled").unwrap();
        assert!(pooled.is_symlink);
        assert!(pooled.read_only);
        assert!(!pooled.read_only_reason.is_empty());
        assert_eq!(c.bundle_rows.len(), 2);
    }
}
