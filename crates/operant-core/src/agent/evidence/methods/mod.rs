//! The compression methods, and the chains that compose them.
//!
//! The split is by **safety**, not by ecosystem:
//!
//! * [`lossless`] — a method that cannot lose information. These are always
//!   safe to run.
//! * [`lossy`] — a method that can lose information, and therefore must
//!   declare how the information comes back.
//!
//! A chain is selected by the payload's [`ContentFormat`], not by the tool
//! that produced it: the same chain serves a shell result and a subprocess
//! result, because the *shape* is what determines the safe reduction.

pub mod code;
pub mod lossless;
pub mod lossy;

use super::types::{ContentFormat, Method};

pub use code::{CodeFilter, CodeFilterAggressive, CodeLevel};
pub use lossless::{
    Ansi, Base64Elide, Dedup, MdSlim, MinifyJson, Progress, Rle, Shape, StripLineNumbers,
};
pub use lossy::{Cap, GrepGroup, Lockfile, Toon, Truncate, WebMd};

/// The methods that cannot lose information.
pub static LOSSLESS: &[&dyn Method] = &[
    &Ansi,
    &Progress,
    &Dedup,
    &MinifyJson,
    &MdSlim,
    &StripLineNumbers,
    &Base64Elide,
    &Shape,
    &Rle,
];

/// The methods that can lose information, and declare a recovery.
pub static LOSSY: &[&dyn Method] = &[
    &Toon,
    &GrepGroup,
    &Cap,
    &Truncate,
    &WebMd,
    &Lockfile,
    &CodeFilter,
    &CodeFilterAggressive,
];

/// Every method the registry knows, by name.
///
/// The operator's deny-list (`families.<subject>.methods`) names methods by
/// their `name()`. A name that is not in this list is a typo, and a typo
/// that is silently ignored looks exactly like a setting that does nothing.
pub fn all_names() -> Vec<&'static str> {
    LOSSLESS
        .iter()
        .chain(LOSSY.iter())
        .map(|m| m.name())
        .collect()
}

/// The chain for a payload shape.
///
/// `Json` and `Web` finish with a format conversion that short-circuits, so
/// the cap in `Term` never truncates a converted document into invalid JSON.
/// `Code` strips line-number prefixes and ANSI, and then filters the source
/// itself: a read of a source file is the largest uncompressed surface in
/// the stack, and the code filter is the direct answer. That last step loses
/// information, so it declares a recall recovery like every other lossy
/// method.
pub fn chain_for(format: ContentFormat) -> &'static [&'static dyn Method] {
    static JSON_CHAIN: &[&dyn Method] = &[&Base64Elide, &Toon];
    static WEB_CHAIN: &[&dyn Method] = &[&Ansi, &Base64Elide, &WebMd, &Toon, &MdSlim];
    static MATCHES_CHAIN: &[&dyn Method] = &[&GrepGroup];
    static LOCKFILE_CHAIN: &[&dyn Method] = &[&Lockfile];
    static TERM_CHAIN: &[&dyn Method] = &[&Ansi, &Progress, &Dedup, &Base64Elide, &Cap];
    static CODE_CHAIN: &[&dyn Method] = &[&Ansi, &StripLineNumbers, &Base64Elide, &CodeFilter];
    static PLAIN_CHAIN: &[&dyn Method] = &[&Ansi, &Dedup];

    match format {
        ContentFormat::Json => JSON_CHAIN,
        ContentFormat::Web => WEB_CHAIN,
        ContentFormat::Matches => MATCHES_CHAIN,
        ContentFormat::Lockfile => LOCKFILE_CHAIN,
        ContentFormat::Term => TERM_CHAIN,
        ContentFormat::Code => CODE_CHAIN,
        ContentFormat::Plain => PLAIN_CHAIN,
    }
}

/// The named chain, for configuration that names a chain directly.
pub fn named_chain(name: &str) -> Option<&'static [&'static dyn Method]> {
    // The aggressive code filter: comments and function bodies go, the
    // signatures stay.
    static CODE_AGGRESSIVE_CHAIN: &[&dyn Method] = &[
        &Ansi,
        &StripLineNumbers,
        &Base64Elide,
        &CodeFilterAggressive,
    ];
    match name {
        "json" => Some(chain_for(ContentFormat::Json)),
        "web" => Some(chain_for(ContentFormat::Web)),
        "matches" => Some(chain_for(ContentFormat::Matches)),
        "lockfile" => Some(chain_for(ContentFormat::Lockfile)),
        "term" => Some(chain_for(ContentFormat::Term)),
        "code" => Some(chain_for(ContentFormat::Code)),
        "code-aggressive" => Some(CODE_AGGRESSIVE_CHAIN),
        "plain" => Some(chain_for(ContentFormat::Plain)),
        "lossless" => Some(LOSSLESS),
        _ => None,
    }
}
