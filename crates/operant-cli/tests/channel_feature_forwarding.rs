//! Lock on the channel-feature forwarding contract.
//!
//! The regression: `operant-gateway` requested 21 `channel-*` features directly
//! on its `operant-channels` dependency, and `operant-cli` forwarded its own
//! `channel-*` features only to `operant-channels`. Feature selection is a
//! *union* under cargo's resolver, so the hardcoded list won unconditionally —
//! all 24 forwarding features were decorative, proven by a byte-identical
//! 51,026,384-byte binary after seven channels were dropped from the CLI's
//! defaults. Deleting the list alone would have dropped every channel, because
//! `gateway = ["dep:operant-gateway"]` passed no channel flags at all.
//!
//! Selection is a build-time property of the manifests, so it cannot be
//! asserted from inside a compiled binary. These tests read the two manifests
//! and enforce the invariants that make the flags meaningful; the observable
//! half of the proof is the resolver:
//!
//! ```text
//! cargo tree -p operant-gateway --no-default-features -e features \
//!     -f "{p} [{f}]" -i operant-channels --depth 1
//! ```
//!
//! With the fix that resolves to the six channels the gateway's own source
//! needs and nothing else; adding `channel-telegram` to the same query's
//! feature set adds exactly one channel.

// Integration test binaries are not covered by the `#![cfg_attr(test, ...)]`
// exemption in main.rs, so the gate's -D flags reach here. `expect` is used
// only where a broken manifest makes the assertion meaningless.
#![expect(
    clippy::expect_used,
    reason = "tests assert on workspace manifests; a parse failure is a hard error"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

fn manifest(crate_name: &str) -> toml::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(crate_name)
        .join("Cargo.toml");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// `feature -> value` list pairs of the manifest's `[features]` table.
fn features(crate_name: &str) -> BTreeMap<String, Vec<String>> {
    let value = manifest(crate_name);
    value
        .get("features")
        .and_then(toml::Value::as_table)
        .expect("manifest has a [features] table")
        .iter()
        .map(|(name, value)| {
            let members = value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            (name.clone(), members)
        })
        .collect()
}

/// How a manifest requests `operant-channels`.
fn channels_dep(crate_name: &str) -> (bool, Vec<String>) {
    let value = manifest(crate_name);
    let dep = value
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .and_then(|deps| deps.get("operant-channels"))
        .expect("manifest depends on operant-channels");
    let requested = dep
        .get("features")
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    (
        dep.get("default-features")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true),
        requested,
    )
}

/// The channels that have to be on the `operant-gateway` -> `operant-channels`
/// dependency even when no feature is enabled, split by why.
///
/// `GATEWAY_SOURCE`: named by `operant-gateway`'s own source with no `cfg` gate
/// — `WhatsAppChannel`, `WatiChannel`, `GmailPushChannel`,
/// `NextcloudTalkChannel` and `LinqChannel` (lib.rs) and `AcpServer` (acp.rs)
/// are AppState fields with handlers and routes attached.
///
/// `CHANNELS_SOURCE`: required by `operant-channels` itself —
/// `orchestrator/mod.rs` re-exports Discord, DiscordHistory, Slack and
/// Webhook with no `cfg` gate, so that crate does not compile without them.
///
/// Nothing else may sit on that dependency: a pinned name can never be
/// switched off, which is exactly what made the forwarding features
/// decorative.
const GATEWAY_SOURCE: [&str; 6] = [
    "channel-acp-server",
    "channel-email",
    "channel-linq",
    "channel-nextcloud",
    "channel-wati",
    "channel-whatsapp-cloud",
];
const CHANNELS_SOURCE: [&str; 3] = ["channel-discord", "channel-slack", "channel-webhook"];

/// Everything the gateway may pin, as the names appear on the dependency.
fn pinned_channel_floor() -> Vec<String> {
    GATEWAY_SOURCE
        .iter()
        .chain(CHANNELS_SOURCE.iter())
        .map(|s| (*s).to_string())
        .collect()
}

#[test]
fn every_cli_channel_feature_reaches_both_consumers() {
    for (name, members) in features("operant-cli") {
        let Some(channel) = name.strip_prefix("channel-") else {
            continue;
        };
        for hop in [
            format!("operant-channels/channel-{channel}"),
            format!("operant-gateway?/channel-{channel}"),
        ] {
            assert!(
                members.contains(&hop),
                "operant-cli feature `{name}` must forward to `{hop}`; \
                 forwarding to operant-channels alone left operant-gateway's \
                 own copy of the flag dead"
            );
        }
    }
}

#[test]
fn cli_does_not_pull_the_channels_default_set() {
    let (default_features, requested) = channels_dep("operant-cli");
    assert!(
        !default_features,
        "operant-channels' own default set enables 21 channels on its own; \
         leaving it on makes every operant-cli channel-* flag decorative"
    );
    assert!(
        requested.is_empty(),
        "operant-cli's operant-channels dependency is a feature carrier, not a \
         consumer; hardcoding channel-* on it re-creates the operant-gateway bug \
         (found: {requested:?})"
    );
}

#[test]
fn gateway_pins_only_the_channels_its_sources_require() {
    let (default_features, requested) = channels_dep("operant-gateway");
    assert!(
        !default_features,
        "operant-gateway inherits nothing implicitly: leaving the channels \
         default set on would re-decorate every forwarding feature"
    );
    let floor = pinned_channel_floor();
    for channel in &requested {
        assert!(
            floor.contains(channel),
            "`{channel}` is requested unconditionally on the dependency, so the \
             `{channel}` forwarding feature can never switch it off. Only the \
             channels named unconditionally by operant-gateway/src or \
             operant-channels/src/orchestrator may appear here"
        );
    }
    for channel in &floor {
        assert!(
            requested.contains(channel),
            "`{channel}` is named unconditionally in source and must stay on the \
             dependency, or a build with every feature off stops compiling"
        );
    }
}
