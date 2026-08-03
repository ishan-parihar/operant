#!/usr/bin/env python3
"""Convert vendor-channel feature gates to require the `channels-vendor` cfg.

The channel adapters gated behind the undeclared `channels-vendor` cfg
(lark, line, matrix, nostr, voice-call, voice-wake, wechat, whatsapp-web)
reference vendor SDKs that have never been wired into this workspace. Their
orchestrator body gates must therefore require BOTH the channel feature AND
the `channels-vendor` cfg, so `--all-features` (which enables every declared
channel feature) does not try to compile the never-wired adapter bodies.

Transformation applied (in order, idempotent):
  #[cfg(feature = "X")]            -> #[cfg(all(feature = "X", feature = "channels-vendor"))]
  #[cfg(not(feature = "X"))]       -> #[cfg(not(all(feature = "X", feature = "channels-vendor")))]
where X in VENDOR_FEATURES.

The `channels-vendor` cfg is registered feature-style in build.rs
(cargo:rustc-check-cfg=cfg(feature, values("channels-vendor"))), matching
how operant-hardware registers `hardware-vendor`.
"""

import re
import sys

VENDOR_FEATURES = [
    "channel-lark",
    "channel-line",
    "channel-matrix",
    "channel-nostr",
    "channel-voice-call",
    "voice-wake",
    "channel-wechat",
    "whatsapp-web",
]


def transform_text(text: str) -> str:
    for feat in VENDOR_FEATURES:
        # positive gate
        text = re.sub(
            re.escape(f'#[cfg(feature = "{feat}")]'),
            f'#[cfg(all(feature = "{feat}", feature = "channels-vendor"))]',
            text,
        )
        # negative gate
        text = re.sub(
            re.escape(f'#[cfg(not(feature = "{feat}"))]'),
            f'#[cfg(not(all(feature = "{feat}", feature = "channels-vendor")))]',
            text,
        )
    return text


def main() -> None:
    path = sys.argv[1]
    with open(path, encoding="utf-8") as f:
        text = f.read()
    transformed = transform_text(text)
    if transformed == text:
        print(f"no changes: {path}")
        return
    with open(path, "w", encoding="utf-8") as f:
        f.write(transformed)
    print(f"transformed: {path}")


if __name__ == "__main__":
    main()
