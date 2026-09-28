//! Structural runtime identity — the replacement for free-form provider
//! display strings.
//!
//! A [`RuntimeKey`] is `{ vendor, model, revision }`. Two spellings that name
//! the same runtime produce the same key, so cache entries, cooldown
//! bookkeeping, and failover decisions key off something structural instead of
//! off whatever string a config file happened to contain.
//!
//! The key is *identity only* — the wire string sent to a provider is still
//! carried verbatim by the request, so a normalized key never changes what a
//! provider is asked to serve.
//!
//! # Normalization rules
//!
//! Applied in order by [`RuntimeKey::parse`]:
//!
//! 1. **Trim** surrounding whitespace.
//! 2. **Case-fold** the whole spelling (`ASCII` lower) — model ids are
//!    case-insensitive on every provider operant talks to.
//! 3. **Vendor prefix** — the segment before the *first* `/` becomes the
//!    vendor when it looks like a vendor slug (non-empty, no further `/`).
//!    `anthropic/claude-sonnet-4` → vendor `anthropic`; the remainder keeps
//!    any further slashes, because aggregator model ids legitimately contain
//!    them (`openrouter/anthropic/claude-sonnet-4`). A bare slug leaves the
//!    vendor [`VendorId::UNKNOWN`] until [`RuntimeKey::parse_in`] recovers it
//!    from the registered providers' model prefixes.
//! 4. **Revision peel** — a trailing `-latest` becomes
//!    [`ModelRevision::Floating`]; a trailing `-YYYYMMDD` or `-YYYY-MM-DD`
//!    becomes [`ModelRevision::Pinned`] (dashes stripped, so both spellings of
//!    one date are one revision). The stamp is removed from the model slug, so
//!    `claude-sonnet-4-20250514` is the same *model* as `claude-sonnet-4` at a
//!    different revision.
//!
//! # Deliberately NOT resolved
//!
//! * **`-latest` vs a bare slug.** `claude-sonnet-4-latest` is the vendor's
//!   *floating* alias — it can point at a different model tomorrow. Merging it
//!   with the bare slug would let a cached response or a cooldown be reused
//!   across a revision bump. Two keys is the honest answer; a wrong merge is a
//!   silent wrong answer.
//! * **An unknown vendor.** A bare slug no registered provider claims stays
//!   vendor-less rather than being attributed to a guess. Attributing it would
//!   merge two different vendors' runtimes into one key.
//! * **A slug two providers claim.** When model-prefix inference is ambiguous,
//!   [`ProviderCatalog::infer_vendor`] returns `None` and the vendor stays
//!   unknown, for the same reason.
//! * **A date stamp that is not at the tail**, and any trailing token that
//!   merely looks date-ish but is neither 8 digits nor `YYYY-MM-DD`.

use std::borrow::Cow;
use std::fmt;

use super::chat_provider::ProviderCatalog;

/// Normalized vendor slug — the `anthropic` in `anthropic/claude-sonnet-4`.
///
/// Replaces the raw `String` that used to key cooldowns, failure counts, and
/// client lookups, so `Anthropic`, `anthropic`, and `anthropic/` are one vendor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct VendorId(Cow<'static, str>);

impl VendorId {
    /// The empty slug: a vendor that could not be determined.
    pub const UNKNOWN: &'static str = "";

    /// Normalize a vendor spelling. A candidate that cannot be a vendor slug
    /// (empty, contains `/`, or contains whitespace) normalizes to
    /// [`VendorId::UNKNOWN`] rather than to a plausible-looking lie.
    pub fn new(raw: &str) -> Self {
        let trimmed = raw.trim().trim_end_matches('/').trim();
        if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains(char::is_whitespace) {
            return Self(Cow::Borrowed(Self::UNKNOWN));
        }
        Self(Cow::Owned(trimmed.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_unknown(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for VendorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for VendorId {
    fn from(raw: &str) -> Self {
        Self::new(raw)
    }
}

/// Which revision of a model a key names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelRevision {
    /// A bare slug — whatever the vendor serves for it today.
    Default,
    /// The vendor's floating `-latest` alias. Deliberately a *different* key
    /// from [`ModelRevision::Default`]; see the module docs.
    Floating,
    /// A dated release peeled off the tail of the slug: `-20250514` or
    /// `-2025-05-14`. Both spellings of one date normalize to the same stamp.
    Pinned(String),
}

/// Structural identity of one runtime: vendor + model + pinned revision.
///
/// `Hash` + `Eq` are what make it usable as a cache key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeKey {
    vendor: VendorId,
    model: String,
    revision: ModelRevision,
}

impl RuntimeKey {
    /// Build a key from already-normalized parts.
    pub fn new(vendor: VendorId, model: impl Into<String>, revision: ModelRevision) -> Self {
        Self {
            vendor,
            model: model.into(),
            revision,
        }
    }

    /// Normalize a `vendor/model` or bare `model` spelling.
    ///
    /// A bare slug keeps [`VendorId::UNKNOWN`] as its vendor — use
    /// [`RuntimeKey::parse_in`] when a [`ProviderCatalog`] is available to
    /// recover the vendor from the registered providers' model prefixes.
    pub fn parse(spelling: &str) -> Self {
        let trimmed = spelling.trim();
        if trimmed.is_empty() {
            return Self {
                vendor: VendorId::default(),
                model: String::new(),
                revision: ModelRevision::Default,
            };
        }
        let lowered = trimmed.to_ascii_lowercase();
        let (vendor_hint, rest) = match lowered.split_once('/') {
            Some((head, tail)) => (VendorId::new(head), tail),
            None => (VendorId::default(), lowered.as_str()),
        };
        let (model, revision) = peel_revision(rest);
        Self {
            vendor: vendor_hint,
            model,
            revision,
        }
    }

    /// Like [`RuntimeKey::parse`], but recovers the vendor of a bare slug from
    /// the registered providers.
    ///
    /// A slug no provider claims — or one two providers claim — keeps the
    /// unknown vendor rather than guessing. See the module docs.
    pub fn parse_in(spelling: &str, catalog: &ProviderCatalog) -> Self {
        let key = Self::parse(spelling);
        if key.vendor.is_unknown()
            && !key.model.is_empty()
            && let Some(vendor) = catalog.infer_vendor(&key.model)
        {
            return Self {
                vendor,
                model: key.model,
                revision: key.revision,
            };
        }
        key
    }

    pub fn vendor(&self) -> &VendorId {
        &self.vendor
    }

    /// The model slug with any revision stamp removed.
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn revision(&self) -> &ModelRevision {
        &self.revision
    }

    /// Canonical `vendor/model` text, or just the slug when the vendor is
    /// unknown. Stable for a given key — safe as a map/cache key.
    pub fn as_str(&self) -> String {
        if self.vendor.is_unknown() {
            self.model.clone()
        } else {
            format!("{}/{}", self.vendor, self.model)
        }
    }
}

impl fmt::Display for RuntimeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

/// Peel a revision off the tail of a model slug.
///
/// `-latest` is checked before the date forms; they are mutually exclusive
/// suffixes, so the order is only a readability choice.
fn peel_revision(slug: &str) -> (String, ModelRevision) {
    if let Some(base) = slug.strip_suffix("-latest") {
        return (base.to_string(), ModelRevision::Floating);
    }
    // `-YYYYMMDD` — 9 trailing characters, the leading one being the separator.
    if slug.len() > 9 {
        let (base, stamp) = slug.split_at(slug.len() - 9);
        if stamp.starts_with('-') && stamp[1..].bytes().all(|b| b.is_ascii_digit()) {
            return (
                base.to_string(),
                ModelRevision::Pinned(stamp[1..].to_string()),
            );
        }
    }
    // `-YYYY-MM-DD` — 11 trailing characters.
    if slug.len() > 11 {
        let (base, stamp) = slug.split_at(slug.len() - 11);
        let b = stamp.as_bytes();
        if stamp.starts_with('-')
            && b[1..5].iter().all(u8::is_ascii_digit)
            && b[5] == b'-'
            && b[6..8].iter().all(u8::is_ascii_digit)
            && b[8] == b'-'
            && b[9..11].iter().all(u8::is_ascii_digit)
        {
            // Dashes stripped so `-2025-05-14` and `-20250514` are one revision.
            let compact: String = b[1..11]
                .iter()
                .copied()
                .filter(|c| *c != b'-')
                .map(char::from)
                .collect();
            return (base.to_string(), ModelRevision::Pinned(compact));
        }
    }
    (slug.to_string(), ModelRevision::Default)
}

/// Per-route cost estimate, USD per million tokens.
///
/// Reuses the models.dev catalog `agent::compress::emit_usage_and_cost`
/// already prices requests with — there is no second pricing table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteCost {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

impl RouteCost {
    /// Routing proxy only: the 50/50 blended rate. Used to *order* candidate
    /// fallbacks, never to bill — real spend follows the actual token mix.
    pub fn blended_per_million(&self) -> f64 {
        (self.input_per_million + self.output_per_million) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::chat_provider::ChatProvider;

    /// Test-local vendors so these assertions hold with or without the
    /// `anthropic` cargo feature.
    struct FakeClaude;
    impl ChatProvider for FakeClaude {
        fn vendor(&self) -> &str {
            "anthropic"
        }
        fn vendor_aliases(&self) -> &'static [&'static str] {
            &["claude"]
        }
        fn model_prefixes(&self) -> &'static [&'static str] {
            &["claude-"]
        }
    }
    struct FakeGpt;
    impl ChatProvider for FakeGpt {
        fn vendor(&self) -> &str {
            "openai"
        }
        fn model_prefixes(&self) -> &'static [&'static str] {
            &["gpt-"]
        }
    }

    fn catalog() -> ProviderCatalog {
        let mut c = ProviderCatalog::new();
        c.register(FakeClaude);
        c.register(FakeGpt);
        c
    }

    #[test]
    fn runtime_key_should_normalize_equivalent_spellings() {
        let cat = catalog();
        // The whole point: a prefixed slug, a bare slug, and a cased/prepped
        // spelling of the same model are one runtime.
        let prefixed = RuntimeKey::parse_in("Anthropic/Claude-Sonnet-4", &cat);
        let bare = RuntimeKey::parse_in("claude-sonnet-4", &cat);
        let padded = RuntimeKey::parse_in("  claude-sonnet-4  ", &cat);
        assert_eq!(prefixed, bare, "prefixed and bare spellings must agree");
        assert_eq!(padded, bare, "whitespace must not fork the key");
        assert_eq!(bare.model(), "claude-sonnet-4");
        assert_eq!(bare.vendor().as_str(), "anthropic");
        assert_eq!(bare.revision(), &ModelRevision::Default);
        assert_eq!(bare.as_str(), "anthropic/claude-sonnet-4");

        // Vendor-slug spellings fold too: case and a trailing `/` are noise.
        assert_eq!(VendorId::new("  OpenAI "), VendorId::new("openai"));
        assert_eq!(VendorId::new("Anthropic/"), VendorId::new("anthropic"));
        // The same slug under another vendor is a different runtime.
        assert_ne!(bare, RuntimeKey::parse_in("openai/claude-sonnet-4", &cat));
    }

    #[test]
    fn runtime_key_should_not_merge_distinct_revisions() {
        let cat = catalog();
        let bare = RuntimeKey::parse_in("claude-sonnet-4", &cat);
        let floating = RuntimeKey::parse_in("claude-sonnet-4-latest", &cat);
        let dated = RuntimeKey::parse_in("claude-sonnet-4-20250514", &cat);
        let dashed = RuntimeKey::parse_in("claude-sonnet-4-2025-05-14", &cat);

        // Same model, three revisions — three keys, on purpose.
        assert_ne!(bare, floating, "a floating alias is not a pinned model");
        assert_ne!(bare, dated, "a dated release is not a bare slug");
        assert_ne!(floating, dated);
        // The two spellings of one date are the same release, so they do merge.
        assert_eq!(dated, dashed);
        assert_eq!(dated.revision(), &ModelRevision::Pinned("20250514".into()));
        // …and the stamp is not part of the model slug.
        assert_eq!(dated.model(), bare.model());
        assert_eq!(floating.model(), bare.model());
        assert_eq!(
            RuntimeKey::parse_in("gpt-4o-2024-08-06", &cat).revision(),
            &ModelRevision::Pinned("20240806".into())
        );
        // A non-date tail is a real model segment, not a revision.
        assert_eq!(
            RuntimeKey::parse("gpt-4-32k").revision(),
            &ModelRevision::Default
        );
        assert_eq!(RuntimeKey::parse("gpt-4-32k").model(), "gpt-4-32k");
        assert_eq!(RuntimeKey::parse("gpt-4-1").model(), "gpt-4-1");
    }

    #[test]
    fn aggregator_slashes_survive_in_the_model_slug() {
        let cat = catalog();
        let key = RuntimeKey::parse_in("openrouter/anthropic/claude-sonnet-4", &cat);
        assert_eq!(key.vendor().as_str(), "openrouter");
        assert_eq!(key.model(), "anthropic/claude-sonnet-4");
    }

    #[test]
    fn bare_slug_no_provider_claims_stays_vendorless() {
        let cat = catalog();
        let key = RuntimeKey::parse_in("some-unknown-model-1", &cat);
        assert!(key.vendor().is_unknown());
        assert_eq!(key.model(), "some-unknown-model-1");
        assert_eq!(key.as_str(), "some-unknown-model-1");
    }

    #[test]
    fn vendor_inference_refuses_ambiguous_prefixes() {
        struct Left;
        struct Right;
        impl ChatProvider for Left {
            fn vendor(&self) -> &str {
                "left"
            }
            fn model_prefixes(&self) -> &'static [&'static str] {
                &["shared-"]
            }
        }
        impl ChatProvider for Right {
            fn vendor(&self) -> &str {
                "right"
            }
            fn model_prefixes(&self) -> &'static [&'static str] {
                &["shared-"]
            }
        }
        let mut cat = ProviderCatalog::new();
        cat.register(Left);
        cat.register(Right);
        assert!(cat.infer_vendor("shared-thing").is_none());
        assert!(
            RuntimeKey::parse_in("shared-thing", &cat)
                .vendor()
                .is_unknown()
        );
    }

    #[test]
    fn longest_matching_prefix_wins() {
        struct Broad;
        struct Narrow;
        impl ChatProvider for Broad {
            fn vendor(&self) -> &str {
                "broad"
            }
            fn model_prefixes(&self) -> &'static [&'static str] {
                &["m-"]
            }
        }
        impl ChatProvider for Narrow {
            fn vendor(&self) -> &str {
                "narrow"
            }
            fn model_prefixes(&self) -> &'static [&'static str] {
                &["m-special-"]
            }
        }
        let mut cat = ProviderCatalog::new();
        cat.register(Broad);
        cat.register(Narrow);
        assert_eq!(
            cat.infer_vendor("m-special-1").map(|v| v.to_string()),
            Some("narrow".into())
        );
        assert_eq!(
            cat.infer_vendor("m-other-1").map(|v| v.to_string()),
            Some("broad".into())
        );
    }

    #[test]
    fn vendor_aliases_fold_onto_one_vendor() {
        let cat = catalog();
        assert_eq!(
            cat.resolve_vendor("OpenAI").map(|v| v.to_string()),
            Some("openai".into())
        );
        assert_eq!(
            cat.resolve_vendor("claude").map(|v| v.to_string()),
            Some("anthropic".into())
        );
        assert!(cat.resolve_vendor("not-a-vendor").is_none());
        assert!(cat.resolve_vendor("  ").is_none());
    }

    #[test]
    fn empty_and_whitespace_spellings_do_not_panic() {
        assert_eq!(RuntimeKey::parse("").as_str(), "");
        assert!(RuntimeKey::parse("   ").vendor().is_unknown());
        assert_eq!(RuntimeKey::parse("Anthropic/").model(), "");
        assert_eq!(
            RouteCost {
                input_per_million: 3.0,
                output_per_million: 15.0
            }
            .blended_per_million(),
            9.0
        );
    }
}
