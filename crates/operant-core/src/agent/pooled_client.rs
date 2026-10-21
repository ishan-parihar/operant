//! Client-layer per-provider credential rotation.
//!
//! [`PooledModelClient`] wraps a [`ModelClient`] together with the provider's
//! [`CredentialPool`] and performs the hermes `mark_exhausted_and_rotate`
//! loop *inside* the client: on a classified credential error (429 rate
//! limit, 401 auth, billing), the key that failed is marked exhausted (with
//! an error-class cooldown — 401 → 5m, 429 → 1h, sole-credential transient →
//! 60s) and the next available key from the pool is tried, bounded by the
//! pool size. When every key is benched, the original error propagates so
//! the outer layers (model fallback chain / provider registry / agent loop)
//! can switch providers — the same composition hermes uses (pool rotation
//! first, `load_pool(fb_provider)` on provider switch).
//!
//! Key selection + runtime key swap on the shared inner client are
//! serialized under an internal mutex so concurrent callers can't race the
//! client's runtime key.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use tracing::warn;

use super::fallback::FallbackModelClient;
use super::model_client::{ChatRequest, ModelClient, StreamChunk};
use crate::client::ChatResponse;
use crate::credential_pool::{CredentialPool, PooledCredential};
use crate::error::{Error, Result};

/// A [`ModelClient`] wrapper that rotates credentials within a per-provider
/// pool on classified credential errors (hermes pool parity).
#[derive(Clone)]
pub struct PooledModelClient {
    inner: Arc<dyn ModelClient>,
    pool: Arc<CredentialPool>,
    /// Serializes key selection + runtime swap + request so concurrent
    /// callers can't race the shared client's runtime key.
    gate: Arc<tokio::sync::Mutex<()>>,
    /// An EMPTY pool means "unauthenticated server" rather than "nothing was
    /// configured" only for a local endpoint (ollama, lm-studio, a LAN
    /// gateway); there the call passes straight through. For a remote host an
    /// empty pool keeps the actionable "run `operant setup`" error.
    keyless_ok: Arc<std::sync::atomic::AtomicBool>,
    /// The id of the pool credential the client last selected.
    ///
    /// A runtime rotation replaces the credential the client is ACTUALLY
    /// using, not one chosen by source label. The pool's seeds are deduped
    /// by value (`add_seed` skips an identical value), and at startup
    /// `config.client.api_key` already holds the environment's value — so
    /// the `env:`-sourced seed is usually SKIPPED and the credential in use
    /// is the config-sourced one. Replacing "the env one" would therefore
    /// find nothing, and adding a new credential would put it BEHIND the
    /// stale primary under `FillFirst`, so the rotation would still not
    /// take effect. Tracking the credential actually in use is the only
    /// reliable anchor.
    last_credential_id: Arc<std::sync::Mutex<Option<String>>>,
}

impl PooledModelClient {
    /// Create a pooled client over `inner` with the provider's pool.
    ///
    /// An empty pool is valid — the wrapper degenerates to a plain passthrough
    /// (select returns `None` on the first iteration and the error surfaces
    /// immediately), so callers can wrap unconditionally.
    pub fn new(inner: Arc<dyn ModelClient>, pool: Arc<CredentialPool>) -> Self {
        Self {
            inner,
            pool,
            gate: Arc::new(tokio::sync::Mutex::new(())),
            keyless_ok: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_credential_id: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Allow an empty pool to pass through (see `Self::keyless_ok`).
    pub fn with_keyless_ok(self, ok: bool) -> Self {
        self.keyless_ok
            .store(ok, std::sync::atomic::Ordering::Relaxed);
        self
    }

    fn passthrough(&self) -> bool {
        self.pool.is_empty() && self.keyless_ok.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Record the pool credential the client just selected.
    ///
    /// This is the anchor a runtime rotation replaces. The pool's seeds are
    /// deduped by value, so the credential in use is usually the
    /// config-sourced one even when the value came from the environment —
    /// and the selection strategy (`FillFirst` by default) decides which
    /// one that is.
    fn remember_credential(&self, cred: &PooledCredential) {
        if let Ok(mut slot) = self.last_credential_id.lock() {
            *slot = Some(cred.id.clone());
        }
    }

    /// Extract a provider-supplied recovery timestamp from the error, when
    /// present (hermes `last_error_reset_at` parity). The gateway's
    /// `retry-after` beats the error-class TTL so a free-tier per-minute
    /// throttle benches the key for seconds, not an hour.
    fn reset_at_from_error(err: &Error) -> Option<chrono::DateTime<chrono::Utc>> {
        let retry_after = match err {
            Error::RateLimited { retry_after } => Some(*retry_after),
            Error::Provider {
                retry_after: Some(d),
                ..
            } => Some(*d),
            _ => None,
        };
        retry_after.map(|d| chrono::Utc::now() + chrono::Duration::from_std(d).unwrap_or_default())
    }

    /// Bench the given credential for a classified credential error (hermes
    /// `mark_exhausted_and_rotate`): sizes the cooldown by error class and
    /// honors a provider-supplied retry-after.
    fn bench_credential(&self, cred: &PooledCredential, err: &Error) {
        let classified = FallbackModelClient::classify_error(err);
        let reset_at = Self::reset_at_from_error(err);
        self.pool.invalidate_with_reset(
            &cred.id,
            classified.status_code.map(|s| s as i32),
            Some(classified.reason.as_str()),
            Some(&classified.message),
            reset_at,
            false,
        );
        warn!(
            provider = %self.pool.provider(),
            credential = %cred.name,
            status = ?classified.status_code,
            reason = %classified.reason,
            "Credential exhausted — benched"
        );
    }

    /// Build the error surfaced when the pool has no available credential.
    ///
    /// If a classified error was recorded during this rotation loop it is
    /// returned as-is (it carries the underlying error class — normally
    /// `RateLimited`). Otherwise the pool was *already* exhausted — e.g. all
    /// keys benched by a previous call in the same turn — so surface the
    /// earliest remaining bench as a `RateLimited` instead of an opaque
    /// `Agent("no available keys")` error (hermes pool-exhaustion parity):
    /// the outer fallback chain can then act on the rate-limit class and
    /// switch providers instead of failing on an unclassifiable error.
    fn pool_exhausted_error(&self, last_err: Option<Error>) -> Error {
        if let Some(err) = last_err {
            return err;
        }
        let now = chrono::Utc::now();
        let earliest = self
            .pool
            .list()
            .iter()
            .filter_map(|c| c.error_reset_at.map(|r| (r - now).num_seconds().max(0)))
            .min();
        if let Some(secs) = earliest {
            return Error::RateLimited {
                retry_after: Duration::from_secs(secs.max(1) as u64),
            };
        }
        // Actionable because this is the FIRST error a new user sees. On a fresh
        // machine `operant run --query "hi"` printed this verbatim, with no hint
        // that a provider was never configured — it reads as an internal pool
        // failure rather than a missing setup step. `operant doctor` gets this
        // right; the path a new user actually takes did not.
        //
        // Reaching here genuinely means "nothing was configured", not "configured
        // but busy": the rate-limited case returns `Error::RateLimited` above, and
        // any transport error returns earlier still. So naming the remedy is
        // accurate rather than a guess.
        //
        // The original wording is kept as a prefix so existing log greps still
        // match.
        Error::Agent(format!(
            "Credential pool for provider '{}' has no available keys — no API key is \
             configured for it. Run `operant setup` to configure a provider, or set \
             that provider's key in the environment (for example OPENAI_API_KEY).",
            self.pool.provider()
        ))
    }

    /// The non-streaming rotation loop.
    ///
    /// Bounded by the pool size: each iteration consumes one available key
    /// (benched on failure), so the loop terminates when the pool is empty.
    /// Non-rotate errors (4xx other than 429/401-billing, format, policy)
    /// propagate immediately.
    async fn rotate_loop_chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let max_attempts = self.pool.len().max(1);
        let mut last_err: Option<Error> = None;

        for attempt in 0..max_attempts {
            let Some(cred) = self.pool.select() else {
                break; // no available key left
            };
            self.remember_credential(&cred);
            self.inner.set_api_key(&cred.value);
            match self.inner.chat(request.clone()).await {
                Ok(value) => return Ok(value),
                Err(e) => {
                    let classified = FallbackModelClient::classify_error(&e);
                    if !classified.should_rotate_credential {
                        return Err(e);
                    }
                    warn!(
                        attempt = attempt + 1,
                        max = max_attempts,
                        "Credential exhausted — rotating to next key"
                    );
                    self.bench_credential(&cred, &e);
                    last_err = Some(e);
                }
            }
        }

        Err(self.pool_exhausted_error(last_err))
    }
}

#[async_trait]
impl ModelClient for PooledModelClient {
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let _guard = self.gate.lock().await;
        if self.passthrough() {
            // Keyless local endpoint: nothing to rotate, call straight through.
            return self.inner.chat(request).await;
        }
        self.rotate_loop_chat(&request).await
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        // The gate covers key selection + stream establishment; the returned
        // stream is self-contained and doesn't hold the lock.
        let _guard = self.gate.lock().await;
        if self.passthrough() {
            return self.inner.chat_streaming(request).await;
        }
        let max_attempts = self.pool.len().max(1);
        let mut last_err: Option<Error> = None;

        for attempt in 0..max_attempts {
            let Some(cred) = self.pool.select() else {
                break; // no available key left
            };
            self.remember_credential(&cred);
            self.inner.set_api_key(&cred.value);
            match self.inner.chat_streaming(request.clone()).await {
                Ok(stream) => {
                    // Wrap the stream so a rotate-classified MID-STREAM error
                    // (e.g. a 429 chunk after the connection was established)
                    // benches the selected credential. The agent's mid-stream
                    // recovery re-issues the request, and the re-issue picks
                    // the next available key — rotation fires on the retry
                    // (hermes `mark_exhausted_and_rotate` parity for the
                    // stream lifecycle, not just establishment). Transport
                    // drops (Network) and format errors are not benched.
                    let pool = self.pool.clone();
                    let cred_id = cred.id.clone();
                    let cred_name = cred.name.clone();
                    let provider = self.pool.provider().to_string();
                    let wrapped = stream.map(move |item| {
                        if let Err(e) = &item {
                            let classified = FallbackModelClient::classify_error(e);
                            if classified.should_rotate_credential {
                                let reset_at = Self::reset_at_from_error(e);
                                pool.invalidate_with_reset(
                                    &cred_id,
                                    classified.status_code.map(|s| s as i32),
                                    Some(classified.reason.as_str()),
                                    Some(&classified.message),
                                    reset_at,
                                    false,
                                );
                                warn!(
                                    provider = %provider,
                                    credential = %cred_name,
                                    status = ?classified.status_code,
                                    reason = %classified.reason,
                                    "Credential exhausted mid-stream — benched for the retry"
                                );
                            }
                        }
                        item
                    });
                    return Ok(Box::pin(wrapped));
                }
                Err(e) => {
                    let classified = FallbackModelClient::classify_error(&e);
                    if !classified.should_rotate_credential {
                        return Err(e);
                    }
                    warn!(
                        attempt = attempt + 1,
                        max = max_attempts,
                        "Credential exhausted — rotating to next key"
                    );
                    self.bench_credential(&cred, &e);
                    last_err = Some(e);
                }
            }
        }

        Err(self.pool_exhausted_error(last_err))
    }

    fn set_api_key(&self, api_key: &str) {
        // "Use this key NOW", and nothing else.
        //
        // The pool is deliberately NOT touched here. The agent-loop
        // rotation path (`try_rotate_credential`) selects a DIFFERENT
        // existing credential from the pool and calls this with its value,
        // and the pooled client's own request loop sets the key from the
        // pool before every request. If this method rewrote a pool
        // credential's value, the agent-loop rotation would replace the
        // benched credential's secret with the next key's secret and leave
        // the pool holding a duplicate — on every 401 and every 429.
        //
        // Replacing a credential's SECRET is a different operation, and it
        // has its own method: `rotate_credential_value`.
        self.inner.set_api_key(api_key);

        // Refresh the ANCHOR: the credential the client is now using.
        //
        // The anchor is what a later `.env` rotation replaces, and it is
        // only updated inside this client's own request loop. The
        // agent-loop rotation (`try_rotate_credential`) moves the client
        // onto a DIFFERENT credential by calling this with its value —
        // without this, the anchor still points at the credential that was
        // just benched, and the next `.env` rotation would replace the
        // BENCHED credential's secret. That credential's availability
        // window is still running, so the fresh key is never selected and
        // the rotation is silently a no-op.
        //
        // The anchor is the pool credential whose value is the key being
        // set. If the pool does not know that value (a key set from
        // outside the pool), the anchor is left alone: there is no
        // credential to point at.
        if let Some(id) = self
            .pool
            .list()
            .into_iter()
            .find(|c| c.value == api_key.trim())
            .map(|c| c.id)
            && let Ok(mut slot) = self.last_credential_id.lock()
        {
            *slot = Some(id);
        }
    }

    fn rotate_credential_value(&self, api_key: &str) {
        // "Use this key now" as well as "store it in the pool", so the
        // request that follows carries the new secret.
        self.inner.set_api_key(api_key);

        let rotated = api_key.trim();
        if rotated.is_empty() {
            return;
        }

        // The pool already knows this value: it is one of the pool's own
        // credentials, so there is nothing to replace. Replacing anyway
        // would give two credentials the same secret.
        if self.pool.list().iter().any(|c| c.value == rotated) {
            return;
        }

        // Replace the credential the client is ACTUALLY using. The pool's
        // seeds are deduped by value, and at startup
        // `config.client.api_key` already holds the environment's value —
        // so the `env:`-sourced seed is usually skipped and the credential
        // in use is the config-sourced one. Choosing by source label would
        // find nothing; adding a new credential would put it BEHIND the
        // stale primary under `FillFirst`. The credential actually
        // selected is the only reliable anchor.
        let last_id = self
            .last_credential_id
            .lock()
            .ok()
            .and_then(|slot| slot.clone());
        if let Some(id) = last_id
            && self.pool.get(&id).is_some_and(|c| c.value != rotated)
        {
            // Replace the value in place: the id, the source and the
            // pool's ordering stay, so the failure bookkeeping and the
            // rotation order are unaffected.
            self.pool.replace_value(&id, rotated);
            return;
        }

        // The pool has no credential in use yet (nothing was ever selected,
        // or the selected one already holds this value). Fall back to the
        // FIRST credential: under `FillFirst` that is the one the next
        // request will pick, so replacing it is what makes the rotation
        // take effect. A pool with NO credentials at all used to fail with
        // "no available keys" before the inner client was ever tried, so
        // this is the case where a rotation rescues a gateway that had no
        // working key.
        if let Some(first) = self.pool.list().into_iter().next() {
            if first.value != rotated {
                self.pool.replace_value(&first.id, rotated);
            }
            return;
        }
        self.pool.add(PooledCredential::new(
            "rotated",
            crate::credential_pool::AuthType::ApiKey,
            rotated,
            "env:rotated",
        ));
    }

    fn set_endpoint(&self, base_url: &str, api_key: Option<&str>) {
        // The pool's keys belong to the OLD endpoint: drop them and seed the
        // new endpoint's key, or the next request rotates a stale credential
        // back in. A keyless endpoint leaves the pool empty (passthrough).
        for cred in self.pool.list() {
            self.pool.remove(&cred.id);
        }
        if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
            self.pool.add(PooledCredential::new(
                "connect",
                crate::credential_pool::AuthType::ApiKey,
                key,
                "tui /connect",
            ));
        }
        self.keyless_ok.store(
            crate::client::is_local_endpoint(base_url),
            std::sync::atomic::Ordering::Relaxed,
        );
        self.inner.set_endpoint(base_url, api_key);
    }

    fn base_url(&self) -> Option<String> {
        self.inner.base_url()
    }

    fn current_api_key(&self) -> Option<String> {
        self.inner.current_api_key()
    }
}

impl std::fmt::Debug for PooledModelClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledModelClient")
            .field("provider", &self.inner.provider_name())
            .field("pool", &self.pool)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::client::{ChatResponse, Choice, Message, MessageDelta, Role, Usage};
    use crate::credential_pool::{AuthType, PooledCredential};

    fn chat_response(model: &str) -> ChatResponse {
        ChatResponse {
            id: "resp_1".into(),
            object: "chat.completion".into(),
            created: 0,
            model: model.into(),
            choices: vec![Choice {
                index: 0,
                message: MessageDelta {
                    role: Some(Role::Assistant),
                    content: Some("Hello!".into()),
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        }
    }

    /// Scripted outcomes for the mock (avoids cloning `Result`/`Error`).
    enum Outcome {
        Ok(ChatResponse),
        /// 429 with a 5s retry-after.
        RateLimited,
        /// 402 billing exhaustion.
        Billing,
        /// 400 bad request (non-rotate).
        BadRequest,
    }

    /// Mock that returns a scripted sequence of outcomes; records the api key
    /// observed on each request into a shared buffer the test reads back.
    struct ScriptedClient {
        results: Vec<Outcome>,
        call_count: AtomicUsize,
        seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
        /// The key in force when each REQUEST was made.
        ///
        /// `seen_keys` records every `set_api_key` CALL, which is not the
        /// same thing: the pooled client sets the key on the inner client
        /// and then again from the pool before the request. A rotation test
        /// has to assert on the key the request carried, so that is
        /// recorded separately.
        request_keys: Arc<std::sync::Mutex<Vec<String>>>,
        /// The key currently set on this client.
        current_key: Arc<std::sync::Mutex<Option<String>>>,
    }

    impl ScriptedClient {
        #[allow(clippy::new_ret_no_self)]
        fn new(
            results: Vec<Outcome>,
            seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
        ) -> Arc<dyn ModelClient> {
            Arc::new(Self {
                results,
                call_count: AtomicUsize::new(0),
                seen_keys,
                request_keys: Arc::new(std::sync::Mutex::new(Vec::new())),
                current_key: Arc::new(std::sync::Mutex::new(None)),
            })
        }
    }

    /// A scripted client plus the buffer of keys the REQUESTS carried.
    ///
    /// The keys a request carried are the thing a rotation test has to
    /// assert on: the pooled client sets the key on the inner client and
    /// then again from the pool, so the set-call log has one more entry
    /// than the request log and the two differ in order.
    fn scripted_with_request_keys(
        results: Vec<Outcome>,
        seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
    ) -> (Arc<dyn ModelClient>, Arc<std::sync::Mutex<Vec<String>>>) {
        let request_keys = Arc::new(std::sync::Mutex::new(Vec::new()));
        let client = Arc::new(ScriptedClient {
            results,
            call_count: AtomicUsize::new(0),
            seen_keys,
            request_keys: request_keys.clone(),
            current_key: Arc::new(std::sync::Mutex::new(None)),
        });
        (client, request_keys)
    }

    #[async_trait]
    impl ModelClient for ScriptedClient {
        fn provider_name(&self) -> &str {
            "mock"
        }

        fn set_api_key(&self, api_key: &str) {
            if let Ok(mut slot) = self.current_key.lock() {
                *slot = Some(api_key.to_string());
            }
            if let Ok(mut keys) = self.seen_keys.lock() {
                keys.push(api_key.to_string());
            }
        }

        fn current_api_key(&self) -> Option<String> {
            self.current_key.lock().ok().and_then(|k| k.clone())
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            // Record the key in force at request time, before the scripted
            // outcome runs.
            if let Some(key) = self.current_key.lock().ok().and_then(|k| k.clone())
                && let Ok(mut keys) = self.request_keys.lock()
            {
                keys.push(key);
            }
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            match self.results.get(idx) {
                Some(Outcome::Ok(r)) => Ok(r.clone()),
                Some(Outcome::RateLimited) => Err(Error::RateLimited {
                    retry_after: Duration::from_secs(5),
                }),
                Some(Outcome::Billing) => Err(Error::Provider {
                    status: 402,
                    body: "{\"error\":{\"message\":\"insufficient credits\"}}".into(),
                    retry_after: None,
                }),
                Some(Outcome::BadRequest) => Err(Error::Provider {
                    status: 400,
                    body: "Bad Request".into(),
                    retry_after: None,
                }),
                None => Err(Error::Agent("script exhausted".into())),
            }
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(Error::Agent("streaming not mocked".into()))
        }
    }

    fn pool_with(keys: &[&str]) -> Arc<CredentialPool> {
        let pool = Arc::new(CredentialPool::new("mock"));
        for (i, key) in keys.iter().enumerate() {
            pool.add(PooledCredential::new(
                &format!("key-{i}"),
                AuthType::ApiKey,
                key,
                "test",
            ));
        }
        pool
    }

    /// A pool shaped like the one the gateway builds: one env-sourced key
    /// and one config-sourced key.
    fn pool_with_env_and_config(env_key: &str, config_key: &str) -> Arc<CredentialPool> {
        let pool = Arc::new(CredentialPool::new("mock"));
        pool.add(PooledCredential::new(
            "OPENAI_API_KEY",
            AuthType::ApiKey,
            env_key,
            "env:OPENAI_API_KEY",
        ));
        pool.add(PooledCredential::new(
            "primary",
            AuthType::ApiKey,
            config_key,
            "config (client.api_key)",
        ));
        pool
    }

    /// The pool the gateway ACTUALLY builds when the key comes from the
    /// environment.
    ///
    /// `apply_string_option_override` overwrites `config.client.api_key`
    /// with `OPENAI_API_KEY` at startup, so the config-sourced seed holds
    /// the environment's value. `add_seed` then DEDUPES by value, so the
    /// `env:`-sourced seed is skipped and the pool holds ONE credential:
    /// the config-sourced one, carrying the environment's value.
    ///
    /// This is the shape on the common path, and it is why a rotation
    /// cannot look for "the env credential" — there is not one.
    fn pool_with_deduped_env_value(env_value: &str) -> Arc<CredentialPool> {
        let pool = Arc::new(CredentialPool::new("mock"));
        pool.add(PooledCredential::new(
            "primary",
            AuthType::ApiKey,
            env_value,
            "config (client.api_key)",
        ));
        pool
    }

    fn request() -> ChatRequest {
        ChatRequest::new("mock-model", vec![Message::user("hi")])
    }

    fn recording() -> Arc<std::sync::Mutex<Vec<String>>> {
        Arc::new(std::sync::Mutex::new(Vec::new()))
    }

    #[tokio::test]
    async fn empty_pool_is_a_passthrough_for_keyless_endpoints() {
        // ollama / lm-studio / an open gateway carry no key: the wrapper used
        // to fail with "no API key is configured" instead of calling through.
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Ok(chat_response("mock"))], seen.clone());
        let pooled = PooledModelClient::new(client, pool_with(&[])).with_keyless_ok(true);
        let resp = pooled
            .chat(request())
            .await
            .expect("keyless call goes through");
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("Hello!"));
        assert!(seen.lock().unwrap().is_empty(), "no key was swapped in");
    }

    #[tokio::test]
    async fn set_endpoint_replaces_the_old_endpoints_keys() {
        // After /connect the pool must hold ONLY the new endpoint's key, or
        // the next request rotates the previous provider's key back in.
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Ok(chat_response("mock"))], seen.clone());
        let pooled = PooledModelClient::new(client, pool_with(&["old-key"]));
        pooled.set_endpoint("http://new.example/v1", Some("new-key"));
        pooled.chat(request()).await.expect("chat after retarget");
        assert_eq!(seen.lock().unwrap().as_slice(), ["new-key"]);

        // A keyless LOCAL target empties the pool and passes through; a
        // keyless REMOTE one keeps the actionable error.
        pooled.set_endpoint("http://localhost:11434/v1", None);
        assert!(pooled.pool.is_empty() && pooled.passthrough());
        pooled.set_endpoint("https://api.openai.com/v1", None);
        assert!(!pooled.passthrough());
    }

    /// RT-6c: a key rotated at runtime must be the key the NEXT request
    /// sends.
    ///
    /// The pool selects a credential and calls `inner.set_api_key(&cred.value)`
    /// before every request, so a key set only on the inner client is
    /// overwritten by the pool's startup-seeded value on the very next turn.
    /// The rotation must therefore reach the POOL.
    #[tokio::test]
    async fn a_rotated_key_is_the_key_the_next_request_sends() {
        let seen = recording();
        let (client, request_keys) = scripted_with_request_keys(
            vec![
                Outcome::Ok(chat_response("mock")),
                Outcome::Ok(chat_response("mock")),
            ],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(
            client,
            pool_with_env_and_config("old-env-key", "config-key"),
        );

        // Before the rotation, the pool's first credential is used.
        pooled.chat(request()).await.expect("chat before rotation");
        assert_eq!(
            request_keys.lock().unwrap().as_slice(),
            ["old-env-key"],
            "before the rotation the pool's env key is used"
        );

        // Rotate, exactly as the gateway's per-turn `.env` reload does.
        pooled.rotate_credential_value("rotated-key");

        // The NEXT request must carry the rotated key, not the pool's old one.
        pooled.chat(request()).await.expect("chat after rotation");
        let keys = request_keys.lock().unwrap().clone();
        assert_eq!(
            keys,
            vec!["old-env-key", "rotated-key"],
            "the rotation must reach the pool, or the next request reverts to the old key"
        );
    }

    /// A rotation must not discard the keys the user put in `operant.toml`.
    /// Only the env-sourced credential is replaced.
    /// The production shape: ONE credential, named "primary", whose source
    /// is the config seed but whose VALUE came from the environment.
    ///
    /// This is what the gateway actually builds when the key comes from
    /// `.env`: `apply_env_overrides` copies `OPENAI_API_KEY` into
    /// `client.api_key`, the "primary" seed already holds that value, and
    /// `add_seed` dedupes by value, so the `env:`-seeded credential is
    /// DROPPED. A rotation that looks for "the env credential" finds
    /// nothing here — and one that ADDS a credential puts it behind the
    /// stale primary under `FillFirst`, so the rotation still does not
    /// take effect. Only replacing the credential actually in use works.
    #[tokio::test]
    async fn a_rotation_reaches_the_single_credential_the_gateway_builds() {
        let seen = recording();
        let (client, request_keys) = scripted_with_request_keys(
            vec![
                Outcome::Ok(chat_response("mock")),
                Outcome::Ok(chat_response("mock")),
            ],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with_deduped_env_value("old-env-key"));
        assert_eq!(pooled.pool.len(), 1, "the env seed is deduped away");

        // Before the rotation the pool's only credential is used.
        pooled.chat(request()).await.expect("chat before rotation");
        assert_eq!(
            request_keys.lock().unwrap().as_slice(),
            ["old-env-key"],
            "before the rotation the startup-seeded key is used"
        );

        pooled.rotate_credential_value("rotated-key");

        pooled.chat(request()).await.expect("chat after rotation");
        assert_eq!(
            request_keys.lock().unwrap().as_slice(),
            ["old-env-key", "rotated-key"],
            "the rotation must reach the credential in use"
        );
        // The pool must not have grown a duplicate.
        assert_eq!(pooled.pool.len(), 1, "the rotation must not add a key");
    }

    /// A rotation that happens BEFORE any request has run still has to
    /// reach the pool: the first credential is the one `FillFirst` will
    /// pick.
    #[tokio::test]
    async fn a_rotation_before_any_request_reaches_the_first_credential() {
        let seen = recording();
        let (client, request_keys) =
            scripted_with_request_keys(vec![Outcome::Ok(chat_response("mock"))], seen.clone());
        let pooled = PooledModelClient::new(client, pool_with_deduped_env_value("old-env-key"));

        pooled.rotate_credential_value("rotated-key");

        pooled.chat(request()).await.expect("chat after rotation");
        assert_eq!(
            request_keys.lock().unwrap().as_slice(),
            ["rotated-key"],
            "the first request must carry the rotated key"
        );
        assert_eq!(pooled.pool.len(), 1, "the rotation must not add a key");
    }

    /// The agent-loop rotation path calls `set_api_key` with a credential it
    /// just selected FROM the pool. That must not overwrite the pool: the
    /// benched credential would take the next key's value, and the pool
    /// would end up holding a duplicate.
    #[tokio::test]
    async fn a_key_the_pool_already_holds_does_not_change_the_pool() {
        let pool = pool_with(&["k1", "k2"]);
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Ok(chat_response("mock"))], seen.clone());
        let pooled = PooledModelClient::new(client, pool);

        // Exactly what `try_rotate_credential` does after `pool.select()`.
        let selected = pooled.pool.select().expect("a credential is selected");
        pooled.set_api_key(&selected.value);

        let values: Vec<String> = pooled.pool.list().into_iter().map(|c| c.value).collect();
        assert_eq!(
            values,
            vec!["k1".to_string(), "k2".to_string()],
            "the pool must be unchanged: a selected key is already in it"
        );
    }

    /// An internal pool rotation (a 429 or a 401 moving to the next
    /// credential) changes WHICH credential is selected, and nothing else.
    /// No credential's secret may change, and the pool must not grow.
    ///
    /// This is the reason `set_api_key` and `rotate_credential_value` are
    /// separate methods: overloading one of them would rewrite the benched
    /// credential's secret on every 401 and every 429.
    #[tokio::test]
    async fn an_internal_pool_rotation_changes_no_credential_value() {
        let seen = recording();
        let (client, request_keys) = scripted_with_request_keys(
            vec![Outcome::RateLimited, Outcome::Ok(chat_response("mock"))],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        pooled.chat(request()).await.expect("chat rotates on 429");

        let values: Vec<String> = pooled.pool.list().into_iter().map(|c| c.value).collect();
        assert_eq!(
            values,
            vec!["k1".to_string(), "k2".to_string()],
            "the rotation moved to k2; it must not rewrite k1's secret"
        );
        assert_eq!(
            request_keys.lock().unwrap().as_slice(),
            ["k1", "k2"],
            "the rotation switched credentials"
        );
    }

    /// A retarget that carries NO key must not leave the client with no
    /// credential.
    ///
    /// `set_endpoint` drops every pool credential and re-seeds only when a
    /// key is present. A `.env` that carries only a base URL (the key lives
    /// in `operant.toml`) reaches the retarget with no key, and the next
    /// request would fail with "no available keys". The caller therefore
    /// hands the client back the key it is already using — this pins that
    /// the pool survives such a retarget.
    #[tokio::test]
    async fn a_retarget_with_the_current_key_keeps_the_pool_usable() {
        let seen = recording();
        let (client, request_keys) = scripted_with_request_keys(
            vec![
                Outcome::Ok(chat_response("mock")),
                Outcome::Ok(chat_response("mock")),
            ],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1"]));

        // The client is using k1; the caller reads it back and hands it over.
        pooled.chat(request()).await.expect("first chat");
        let current = pooled
            .current_api_key()
            .expect("the current key is readable");
        assert_eq!(current, "k1");

        pooled.set_endpoint("http://new.example/v1", Some(&current));

        assert!(
            !pooled.pool.is_empty(),
            "the retarget must leave the client with a credential"
        );
        pooled
            .chat(request())
            .await
            .expect("chat after the retarget");
        assert_eq!(
            request_keys.lock().unwrap().as_slice(),
            ["k1", "k1"],
            "the request after the retarget must still carry the key"
        );
    }

    /// The FULL rotation sequence, end to end: the agent-loop rotation
    /// moves the client to k2, and a later `.env` rotation to k3 must
    /// REPLACE k2 — not the benched k1.
    ///
    /// The anchor (the pool credential the client last used) is what a
    /// `.env` rotation replaces, and it was only updated inside the
    /// client's own request loop. The agent-loop rotation moved the client
    /// to k2 without updating it, so the anchor still pointed at the
    /// benched k1: the `.env` rotation replaced k1's secret, k1 stayed
    /// unavailable for its 401 window, and the fresh key was never
    /// selected. The rotation was silently a no-op.
    #[tokio::test]
    async fn an_env_rotation_after_an_agent_loop_rotation_replaces_the_new_credential() {
        let seen = recording();
        let (client, request_keys) =
            scripted_with_request_keys(vec![Outcome::Ok(chat_response("mock"))], seen.clone());
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        // A request runs, so k1 is the credential in use.
        pooled.chat(request()).await.expect("first chat");
        assert_eq!(
            request_keys.lock().unwrap().as_slice(),
            ["k1"],
            "the first request carried k1"
        );

        // The agent-loop rotation: invalidate k1, select k2, switch to it.
        let active = pooled.pool.list().into_iter().next().expect("a credential");
        pooled
            .pool
            .invalidate(&active.id, None, Some("benched"), None, false);
        let next = pooled
            .pool
            .select()
            .expect("the next credential is selected");
        assert_eq!(next.value, "k2");
        pooled.set_api_key(&next.value);

        // Now the `.env` rotation supplies a brand-new key.
        pooled.rotate_credential_value("k3");

        // The credential in force must be k3, and the benched k1's secret
        // must be untouched.
        let values: Vec<String> = pooled.pool.list().into_iter().map(|c| c.value).collect();
        assert!(
            values.contains(&"k3".to_string()),
            "the rotated key must be IN the pool, got: {values:?}"
        );
        assert!(
            values.contains(&"k1".to_string()),
            "the benched credential's secret must survive, got: {values:?}"
        );
    }

    /// The agent-loop rotation path, exactly: use k1, invalidate it, select
    /// k2, and ask the client to switch to k2.
    ///
    /// The client must not rewrite k1's secret. If `set_api_key` also
    /// replaced the pool credential the client was using, k1 would take
    /// k2's value here and the pool would hold two copies of k2 with k1's
    /// real secret destroyed — on every 401 and every 429.
    #[tokio::test]
    async fn the_agent_loop_rotation_does_not_rewrite_the_benched_credential() {
        let seen = recording();
        let (client, _request_keys) = scripted_with_request_keys(
            vec![
                Outcome::Ok(chat_response("mock")),
                Outcome::Ok(chat_response("mock")),
            ],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        // A request runs, so k1 is the credential in use.
        pooled.chat(request()).await.expect("first chat");

        // `try_rotate_credential`: invalidate the active credential, select
        // the next one, and switch the client to it.
        let active = pooled.pool.list().into_iter().next().expect("a credential");
        pooled
            .pool
            .invalidate(&active.id, None, Some("rotated"), None, false);
        let next = pooled
            .pool
            .select()
            .expect("the next credential is selected");
        assert_eq!(next.value, "k2", "the rotation moves to the second key");
        pooled.set_api_key(&next.value);

        let values: Vec<String> = pooled.pool.list().into_iter().map(|c| c.value).collect();
        assert_eq!(
            values,
            vec!["k1".to_string(), "k2".to_string()],
            "k1's secret must survive the rotation to k2"
        );
    }

    #[tokio::test]
    async fn a_rotation_keeps_the_config_sourced_keys() {
        let pool = pool_with_env_and_config("old-env-key", "config-key");
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Ok(chat_response("mock"))], seen.clone());
        let pooled = PooledModelClient::new(client, pool);

        pooled.rotate_credential_value("rotated-key");

        let values: Vec<String> = pooled.pool.list().into_iter().map(|c| c.value).collect();
        assert!(
            values.contains(&"rotated-key".to_string()),
            "the rotated key must be in the pool: {values:?}"
        );
        assert!(
            values.contains(&"config-key".to_string()),
            "the config-sourced key must survive the rotation: {values:?}"
        );
        assert!(
            !values.contains(&"old-env-key".to_string()),
            "the old env key must be gone: {values:?}"
        );
    }

    /// A pool with NO credentials used to fail with "no available keys"
    /// before the inner client was ever tried, so a rotation could not
    /// rescue a gateway that had no working key. It must now.
    #[tokio::test]
    async fn a_rotation_rescues_a_pool_with_no_credentials() {
        let seen = recording();
        let (client, request_keys) =
            scripted_with_request_keys(vec![Outcome::Ok(chat_response("mock"))], seen.clone());
        let pooled = PooledModelClient::new(client, Arc::new(CredentialPool::new("mock")));
        assert!(pooled.pool.is_empty());

        pooled.rotate_credential_value("rotated-key");

        pooled
            .chat(request())
            .await
            .expect("the rotated key must make the request work");
        assert_eq!(request_keys.lock().unwrap().as_slice(), ["rotated-key"]);
    }

    #[tokio::test]
    async fn rotates_to_next_key_on_429_and_succeeds() {
        let seen = recording();
        let client = ScriptedClient::new(
            vec![Outcome::RateLimited, Outcome::Ok(chat_response("mock"))],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        let result = pooled.chat(request()).await.unwrap();
        assert_eq!(result.model, "mock");
        // First attempt used k1, rotated, second attempt used k2.
        let keys = seen.lock().unwrap().clone();
        assert_eq!(keys, vec!["k1", "k2"]);
        // k1 benched (reset ~5s from retry-after); k2 still available.
        let list = pooled.pool.list();
        assert_eq!(
            list.iter().filter(|c| c.is_available()).count(),
            1,
            "k1 should be benched, k2 available"
        );
        let reset_at = list[0].error_reset_at.unwrap();
        let bench_secs = (reset_at - chrono::Utc::now()).num_seconds();
        assert!(
            bench_secs <= 10,
            "retry-after (5s) should override the 1h TTL, got {bench_secs}s"
        );
    }

    #[tokio::test]
    async fn single_key_failure_propagates_immediately() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::RateLimited], seen);
        let pooled = PooledModelClient::new(client, pool_with(&["k1"]));

        let err = pooled.chat(request()).await.unwrap_err();
        assert!(matches!(err, Error::RateLimited { .. }));
        // Sole credential → retry-after (5s) honored, never the 1h bench.
        let cred = pooled.pool.list().into_iter().next().unwrap();
        let reset_at = cred.error_reset_at.unwrap();
        let bench_secs = (reset_at - chrono::Utc::now()).num_seconds();
        assert!(
            bench_secs <= 10,
            "sole credential should bench ~retry-after, got {bench_secs}s"
        );
    }

    #[tokio::test]
    async fn non_rotate_error_propagates_without_rotation() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::BadRequest], seen.clone());
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        let err = pooled.chat(request()).await.unwrap_err();
        assert!(matches!(err, Error::Provider { status: 400, .. }));
        // Only one key was touched (no rotation on non-rotate errors).
        let keys = seen.lock().unwrap().clone();
        assert_eq!(keys, vec!["k1"]);
        assert!(pooled.pool.has_available());
    }

    #[tokio::test]
    async fn all_keys_benched_returns_last_error() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::RateLimited, Outcome::RateLimited], seen);
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        let err = pooled.chat(request()).await.unwrap_err();
        assert!(matches!(err, Error::RateLimited { .. }));
        // Both keys benched → pool exhausted.
        assert!(!pooled.pool.has_available());
    }

    #[tokio::test]
    async fn empty_pool_passthrough_error() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Ok(chat_response("mock"))], seen);
        let pool = Arc::new(CredentialPool::new("mock"));
        let pooled = PooledModelClient::new(client, pool);
        let err = pooled.chat(request()).await.unwrap_err();
        assert!(err.to_string().contains("no available keys"));
    }

    #[tokio::test]
    async fn exhausted_pool_surfaces_rate_limited_with_remaining_bench() {
        // Regression: after a previous call benched every key, the NEXT call
        // found the pool empty and surfaced an opaque `Agent("no available
        // keys")` error — unclassifiable, so the outer fallback chain never
        // switched providers. Now it must surface `RateLimited` with the
        // earliest remaining bench (hermes pool-exhaustion parity), letting
        // the chain act on the rate-limit class.
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Ok(chat_response("mock"))], seen);
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        // Simulate a previous call that benched both keys for ~10s.
        let reset_at = chrono::Utc::now() + chrono::Duration::seconds(10);
        for cred in pooled.pool.list() {
            pooled.pool.invalidate_with_reset(
                &cred.id,
                Some(429),
                Some("rate_limit"),
                Some("rate limit"),
                Some(reset_at),
                false,
            );
        }
        assert!(!pooled.pool.has_available());

        let err = pooled.chat(request()).await.unwrap_err();
        match err {
            Error::RateLimited { retry_after } => {
                let secs = retry_after.as_secs();
                assert!(
                    (1..=10).contains(&secs),
                    "expected remaining bench ~10s, got {secs}s"
                );
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// Streaming mock: each `chat_streaming` call pops the next scripted
    /// sequence of stream items.
    struct StreamingScriptedClient {
        scripts: std::sync::Mutex<std::collections::VecDeque<Vec<Result<StreamChunk>>>>,
        seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl StreamingScriptedClient {
        #[allow(clippy::new_ret_no_self)]
        fn new(
            scripts: Vec<Vec<Result<StreamChunk>>>,
            seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
        ) -> Arc<dyn ModelClient> {
            Arc::new(Self {
                scripts: std::sync::Mutex::new(scripts.into()), // VecDeque::from
                seen_keys,
            })
        }
    }

    #[async_trait]
    impl ModelClient for StreamingScriptedClient {
        fn provider_name(&self) -> &str {
            "mock-stream"
        }

        fn set_api_key(&self, api_key: &str) {
            if let Ok(mut keys) = self.seen_keys.lock() {
                keys.push(api_key.to_string());
            }
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Err(Error::Agent("non-streaming not used".into()))
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let script = self
                .scripts
                .lock()
                .map(|mut s| s.pop_front().unwrap_or_default())
                .unwrap_or_default();
            Ok(Box::pin(futures::stream::iter(script)))
        }
    }

    #[tokio::test]
    async fn mid_stream_rotate_error_benches_key_and_next_request_rotates() {
        let seen = recording();
        let client = StreamingScriptedClient::new(
            vec![
                // Request 1: stream established with k1, then a mid-stream
                // 429 chunk (connection succeeded, body failed).
                vec![
                    Err(Error::RateLimited {
                        retry_after: Duration::from_secs(5),
                    }),
                    Ok(StreamChunk::new(Some("partial".to_string()), None, None)),
                ],
                // Request 2 (the retry): clean stream on the rotated key.
                vec![
                    Ok(StreamChunk::new(Some("rotated".to_string()), None, None)),
                    Ok(StreamChunk::new(Some(" answer".to_string()), None, None)),
                ],
            ],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        // Request 1: the mid-stream 429 surfaces AND benches k1.
        let mut s1 = pooled.chat_streaming(request()).await.unwrap();
        let first = s1.next().await.unwrap();
        assert!(
            matches!(first, Err(Error::RateLimited { .. })),
            "mid-stream 429 must surface from the stream"
        );
        let _ = s1.next().await.unwrap().unwrap(); // remaining chunk still flows
        assert_eq!(
            pooled
                .pool
                .list()
                .iter()
                .filter(|c| c.is_available())
                .count(),
            1,
            "k1 benched by the mid-stream 429; only k2 available"
        );

        // Request 2: rotation fires — the retry selects k2.
        let mut s2 = pooled.chat_streaming(request()).await.unwrap();
        let mut text = String::new();
        while let Some(item) = s2.next().await {
            if let Ok(chunk) = item {
                text.push_str(chunk.content.as_deref().unwrap_or(""));
            }
        }
        assert_eq!(text, "rotated answer");
        let keys = seen.lock().unwrap().clone();
        assert_eq!(keys, vec!["k1", "k2"], "rotation fired on the retry");
    }

    #[tokio::test]
    async fn mid_stream_network_drop_does_not_bench_key() {
        let seen = recording();
        // reqwest transport error (provider closing the SSE connection
        // mid-body) — the realistic mid-stream drop.
        let net_err = reqwest::Client::new()
            .get("http://127.0.0.1:9/")
            .send()
            .await
            .unwrap_err();
        let client = StreamingScriptedClient::new(
            vec![
                // Transport drop (network) — not the key's fault.
                vec![Err(Error::Network(net_err.into()))],
                // Retry succeeds with the SAME key (no rotation).
                vec![Ok(StreamChunk::new(
                    Some("recovered".to_string()),
                    None,
                    None,
                ))],
            ],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1"]));

        let mut s1 = pooled.chat_streaming(request()).await.unwrap();
        assert!(matches!(s1.next().await.unwrap(), Err(Error::Network(_))));
        // Sole key NOT benched — a transport blip isn't a key failure.
        assert!(pooled.pool.has_available());

        // Next request reuses k1 and succeeds.
        let mut s2 = pooled.chat_streaming(request()).await.unwrap();
        let text = s2.next().await.unwrap().unwrap();
        assert_eq!(text.content.as_deref(), Some("recovered"));
        let keys = seen.lock().unwrap().clone();
        assert_eq!(keys, vec!["k1", "k1"], "transport drop does not rotate");
    }

    #[tokio::test]
    async fn billing_error_benches_full_ttl_even_when_sole() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Billing], seen);
        let pooled = PooledModelClient::new(client, pool_with(&["k1"]));

        let _ = pooled.chat(request()).await.unwrap_err();
        // Billing keeps the full bench (1h) even for a sole credential.
        let cred = pooled.pool.list().into_iter().next().unwrap();
        let reset_at = cred.error_reset_at.unwrap();
        let bench_secs = (reset_at - chrono::Utc::now()).num_seconds();
        assert!(
            bench_secs >= 3000,
            "billing should bench ~1h, got {bench_secs}s"
        );
    }
}
