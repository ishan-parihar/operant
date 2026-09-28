//! PKCE ([RFC 7636](https://www.rfc-editor.org/rfc/rfc7636)) and the OAuth
//! `state` parameter ([RFC 6749 §10.12](https://www.rfc-editor.org/rfc/rfc6749#section-10.12)).
//!
//! Two secrets are minted per authorization attempt:
//!
//! * `code_verifier` — sent only on the token request. `code_challenge` is
//!   its `S256` digest (base64url, no padding) and travels on the
//!   authorization URL, so an attacker who intercepts the authorization code
//!   cannot redeem it without the verifier.
//! * `state` — echoed back on the redirect. It is CSPRNG-random and
//!   compared against the value we issued, so a callback we did not start
//!   (or a code injected into our listener) is rejected. Verification lives
//!   in [`crate::oauth::loopback`]; this module only mints the value.
//!
//! ## Entropy source
//!
//! [`uuid::Uuid::new_v4`] draws its 16 bytes from `getrandom`, which is the
//! platform CSPRNG (`getrandom(2)`/`/dev/urandom` on Linux,
//! `BCryptGenRandom` on Windows, `arc4random` on Apple). Each UUID pins 6 of
//! its 128 bits (the version and variant nibbles), so
//! [`VERIFIER_RANDOM_BYTES`] = 64 raw bytes (four UUIDs) carry
//! 512 − 24 = **488 bits** of entropy — comfortably above the 256-bit
//! recommendation in RFC 7636 §4.1 — and encode to 86 base64url characters,
//! inside the 43–128 length window §4.1 requires. No new dependency: `uuid`
//! (with `v4`), `sha2` and `base64` are all already dependencies of this
//! crate.

use base64::Engine as _;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{OAuthFlowError, Result};

/// CSPRNG bytes behind a PKCE `code_verifier` (488 bits of entropy).
pub const VERIFIER_RANDOM_BYTES: usize = 64;

/// CSPRNG bytes behind a `state` parameter (244 bits, above the 128-bit
/// minimum of RFC 6749 §10.12).
pub const STATE_RANDOM_BYTES: usize = 32;

/// Shortest `code_verifier` RFC 7636 §4.1 allows.
pub const MIN_VERIFIER_LEN: usize = 43;

/// Longest `code_verifier` RFC 7636 §4.1 allows.
pub const MAX_VERIFIER_LEN: usize = 128;

/// The only `code_challenge_method` this crate emits.
///
/// RFC 7636 §4.2 permits `plain` for compatibility; `S256` is mandatory to
/// use unless the server cannot support it, and `plain` degrades PKCE to a
/// no-op, so there is no `plain` code path here.
pub const CHALLENGE_METHOD_S256: &str = "S256";

/// Return `random_bytes` of base64url-no-pad encoded CSPRNG output.
///
/// The base64url alphabet (`A-Z a-z 0-9 - _`) is a subset of the
/// unreserved characters RFC 7636 §4.1 requires, so no escaping is needed.
///
/// The `Result` is currently infallible — [`uuid::Uuid::new_v4`] panics
/// rather than returning an error when the OS CSPRNG is unavailable — but it
/// keeps the call sites total if the source is ever swapped for a fallible
/// reader.
pub fn random_token(random_bytes: usize) -> Result<String> {
    if random_bytes == 0 {
        return Err(OAuthFlowError::Config(
            "random_token requires a non-zero byte count".to_string(),
        ));
    }
    let uuids = random_bytes.div_ceil(16);
    let mut bytes = Vec::with_capacity(uuids * 16);
    for _ in 0..uuids {
        bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    }
    bytes.truncate(random_bytes);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes))
}

/// Derive the `S256` `code_challenge` for `verifier` (RFC 7636 §4.2):
/// `BASE64URL-ENCODE(SHA256(ASCII(code_verifier)))` without padding.
pub fn derive_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// A minted PKCE pair. `verifier` is secret and must never be logged or
/// persisted next to the authorization URL; only `challenge` is public.
#[derive(Debug, Clone)]
pub struct Pkce {
    /// High-entropy secret, sent on the token request only.
    pub verifier: String,
    /// `S256` digest of `verifier`, sent on the authorization request.
    pub challenge: String,
    /// Always [`CHALLENGE_METHOD_S256`].
    pub method: &'static str,
}

/// Mint a fresh PKCE pair.
pub fn generate_pkce() -> Result<Pkce> {
    let verifier = random_token(VERIFIER_RANDOM_BYTES)?;
    Ok(Pkce {
        challenge: derive_challenge(&verifier),
        verifier,
        method: CHALLENGE_METHOD_S256,
    })
}

/// Mint a fresh `state` value for one authorization attempt.
pub fn generate_state() -> Result<String> {
    random_token(STATE_RANDOM_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn pkce_should_generate_distinct_verifier_and_challenge() {
        let a = generate_pkce().expect("pkce");
        let b = generate_pkce().expect("pkce");

        assert_ne!(a.verifier, b.verifier, "verifiers must not repeat");
        assert_ne!(a.challenge, b.challenge, "challenges must not repeat");
        assert_ne!(a.verifier, a.challenge, "challenge is a digest, not a copy");
        assert_eq!(a.method, CHALLENGE_METHOD_S256);
        assert_eq!(a.method, b.method);

        // RFC 7636 §4.1 length window, and the base64url alphabet only.
        for v in [&a.verifier, &b.verifier] {
            assert!(
                (MIN_VERIFIER_LEN..=MAX_VERIFIER_LEN).contains(&v.len()),
                "verifier length {} outside 43..=128",
                v.len()
            );
            assert!(
                v.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
                "verifier must use the unreserved base64url alphabet: {v}"
            );
        }

        // 64 raw bytes → 86 base64url chars, no padding.
        assert_eq!(a.verifier.len(), 86);

        // `state` is a separate draw and must not repeat either.
        let states: HashSet<String> = (0..16).map(|_| generate_state().expect("state")).collect();
        assert_eq!(states.len(), 16, "state values must be distinct");
        let state = states.iter().next().expect("one state");
        assert_eq!(state.len(), 43, "32 raw bytes → 43 base64url chars");
    }

    #[test]
    fn pkce_challenge_should_be_derived_s256_from_verifier() {
        // RFC 7636 Appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(derive_challenge(verifier), expected);

        // And the live path agrees with the pure function.
        let pkce = generate_pkce().expect("pkce");
        assert_eq!(pkce.challenge, derive_challenge(&pkce.verifier));

        // Deterministic: same verifier always yields the same challenge.
        assert_eq!(
            derive_challenge(&pkce.verifier),
            derive_challenge(&pkce.verifier)
        );
    }
}
