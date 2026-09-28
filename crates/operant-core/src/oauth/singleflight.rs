//! Single-flight refresh coordination: one rotating-refresh-token spend per
//! account, no matter how many callers discover the expiry at once.
//!
//! # The failure this prevents
//!
//! Anthropic, Codex, xAI and Nous issue **single-use rotating refresh tokens**.
//! A refresh token is valid exactly once; redeeming it returns a *new* one and
//! the old one dies. So if N concurrent requests each notice that account A
//! is expiring and each spends A's token:
//!
//! * one refresh succeeds;
//! * N−1 are rejected (`invalid_grant`, `refresh_token_reused`, HTTP 400/401);
//! * the pool marks the credential exhausted, and a failed refresh is
//!   indistinguishable from a revoked credential — the account looks dead and
//!   the user is told to re-authenticate.
//!
//! The token is not the only casualty: N−1 wasted refresh attempts also look
//! like a credential-stuffing burst to the provider's rate limiter.
//!
//! # Design
//!
//! [`RefreshCoordinator<T>`] keeps a `HashMap<account, Arc<Flight<T>>>` under
//! a `std::sync::Mutex`. The mutex is held only for map bookkeeping — never
//! across the refresh `await` — so a slow refresh on one account cannot block
//! lookups for another. A `tokio::sync::Mutex` is deliberately avoided: its
//! guard is runtime-affine, which makes `Send` bounds across `await` awkward
//! for no benefit here, since the critical section is a couple of `HashMap`
//! operations.
//!
//! Each `Flight<T>` carries:
//!
//! * `consumed_token` — the refresh token the leader is spending. This is the
//!   **re-check key**: a caller arriving holding the *same* token knows the
//!   leader's result already supersedes what it has, so it must not spend it.
//! * `outcome` — `None` while running, `Some` once finished, behind its own
//!   mutex so a late arrival can read it without touching the map.
//! * a `watch` channel used purely as a "something finished" signal. Waiters
//!   `subscribe()` **before** re-reading `outcome`, so a result landing
//!   between the subscribe and the read is seen by the read, and one landing
//!   after it wakes `changed()`. There is no lost-wakeup window.
//!
//! ## Per-account, not global
//!
//! The map is keyed by account, so account A's slow refresh never delays
//! account B. There is deliberately no process-wide "refresh lock" in this
//! module — that would serialise the whole fleet on the slowest provider.
//!
//! ## Arrival shapes
//!
//! | Flight state | Caller's `observed_token` | Behaviour |
//! |---|---|---|
//! | running | anything (same account) | await the leader's outcome |
//! | finished | the token the leader spent | **re-check hit**: return that outcome, spend nothing |
//! | finished | a different token | replace the entry and run its own refresh |
//! | finished | empty (nothing to compare) | replace the entry and run its own refresh |
//!
//! The third row is why the key is `(account, token)` rather than `account`
//! alone: a caller that already holds a *newer* token must be able to use it,
//! and a failed leader must not strand a caller whose token is still good.
//!
//! ## Failure is attributable
//!
//! [`CoalescedFailure`] separates [`CoalescedFailure::Own`] (this caller spent
//! the token and it failed) from [`CoalescedFailure::Shared`] (another caller
//! spent it; this caller made no network request). N waiters therefore produce
//! **one** provider-side failure, not N — which is what stops a bad refresh
//! from tripping rate limiters or looking like a credential attack.
//!
//! ## Scope
//!
//! The map is in-process. Two `operant` processes refreshing the same account
//! cannot be coalesced here; that case is what
//! [`crate::oauth_refresh::OAuthRefresher::sync_from_auth_store`] mitigates
//! after the fact.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::watch;
use tracing::debug;

use crate::error::Error;

/// Why a coalesced refresh failed, and who spent the token.
#[derive(Debug)]
pub enum CoalescedFailure {
    /// This caller ran the refresh and it failed.
    Own(Error),
    /// Another caller for the same account ran it and it failed. `waiters` is
    /// how many callers were sharing that single attempt.
    Shared {
        /// Number of callers that observed this attempt.
        waiters: u32,
        /// The leader's failure.
        source: Arc<Error>,
    },
}

impl std::fmt::Display for CoalescedFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoalescedFailure::Own(e) => write!(f, "{e}"),
            CoalescedFailure::Shared { waiters, source } => write!(
                f,
                "coalesced refresh failed on another caller \
                 ({waiters} shared this attempt, no additional request was made): {source}"
            ),
        }
    }
}

impl std::error::Error for CoalescedFailure {}

impl From<CoalescedFailure> for Error {
    /// Preserves the leader's error verbatim; a shared failure is re-labelled
    /// so a log reader can tell that this caller made no network request.
    fn from(failure: CoalescedFailure) -> Self {
        match failure {
            CoalescedFailure::Own(e) => e,
            CoalescedFailure::Shared { waiters, source } => Error::Authentication(format!(
                "coalesced refresh failed on another caller \
                 ({waiters} shared this attempt, no additional request was made): {source}"
            )),
        }
    }
}

impl CoalescedFailure {
    /// `true` when this caller performed the refresh itself.
    pub fn is_own(&self) -> bool {
        matches!(self, CoalescedFailure::Own(_))
    }

    /// How many callers shared the failing attempt (1 for an own failure).
    pub fn shared_attempts(&self) -> u32 {
        match self {
            CoalescedFailure::Own(_) => 1,
            CoalescedFailure::Shared { waiters, .. } => *waiters,
        }
    }
}

/// The result of a coalesced refresh, plus provenance for observability.
#[derive(Debug, Clone)]
pub struct Coalesced<T> {
    /// The value the leader's refresh produced.
    pub value: T,
    /// `true` when this caller performed the refresh; `false` when it observed
    /// a result another caller had already produced.
    pub was_leader: bool,
}

/// A published refresh outcome. The `Err` arm holds an `Arc` so N waiters can
/// share one leader's error without cloning it.
type Outcome<T> = Arc<Result<T, Arc<Error>>>;

/// One in-flight (or just-completed) refresh for an account.
struct Flight<T> {
    /// The refresh token the leader is spending — the re-check key.
    consumed_token: String,
    /// `None` while running; `Some` once the leader has finished.
    outcome: Mutex<Option<Outcome<T>>>,
    /// Completion signal. The value is irrelevant; only the version is used.
    done: watch::Sender<()>,
    /// How many callers have observed this attempt.
    waiters: Mutex<u32>,
}

impl<T> Flight<T> {
    fn new(consumed_token: String) -> Arc<Self> {
        let (done, _receiver) = watch::channel(());
        Arc::new(Self {
            consumed_token,
            outcome: Mutex::new(None),
            done,
            waiters: Mutex::new(1),
        })
    }

    fn matches(&self, observed_token: &str) -> bool {
        self.consumed_token == observed_token
    }

    fn outcome(&self) -> Option<Outcome<T>> {
        self.outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn bump_waiter(&self) {
        let mut waiters = self.waiters.lock().unwrap_or_else(|e| e.into_inner());
        *waiters = waiters.saturating_add(1);
    }

    fn waiters(&self) -> u32 {
        *self.waiters.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Publish the result, then signal. The order matters: a waiter that has
    /// already read `outcome == None` must be guaranteed that the send wakes
    /// it, so the send cannot come first.
    fn complete(&self, result: Result<T, Arc<Error>>) {
        *self.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(result));
        let _ = self.done.send(());
    }
}

/// Collapses concurrent refreshes of the same account into one.
///
/// Generic over the refreshed value so the coordinator is not tied to one
/// token type; the crate's own instance is over
/// [`OAuthTokenResponse`](crate::oauth_refresh::OAuthTokenResponse).
pub struct RefreshCoordinator<T> {
    flights: Mutex<HashMap<String, Arc<Flight<T>>>>,
    /// Ceiling on distinct tracked accounts. Past it, a new account is
    /// refreshed *untracked* — correct, merely uncoalesced — rather than
    /// evicting a live flight, since evicting a running flight would let a
    /// second caller spend the same single-use token.
    capacity: usize,
}

impl<T> Default for RefreshCoordinator<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> RefreshCoordinator<T> {
    /// A coordinator with the default account capacity.
    pub fn new() -> Self {
        Self::with_capacity(256)
    }

    /// A coordinator tracking at most `capacity` distinct accounts.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            flights: Mutex::new(HashMap::new()),
            capacity: capacity.max(1),
        }
    }

    /// Number of accounts currently tracked (running or recently completed).
    pub fn tracked(&self) -> usize {
        self.flights.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Forget any cached outcome for `account`, so the next caller refreshes.
    pub fn forget(&self, account: &str) {
        self.flights
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(account);
    }

    /// Run `refresh` for `account`, or join the attempt already in flight.
    ///
    /// `observed_token` is the refresh token the caller currently holds. It is
    /// only ever compared in memory, never logged. Pass the empty string when
    /// the grant has no token to compare: callers then never share a
    /// *completed* result, only a running one, which is the safe default for
    /// non-rotating grants.
    pub async fn coalesce<F, Fut>(
        &self,
        account: &str,
        observed_token: &str,
        refresh: F,
    ) -> Result<Coalesced<T>, CoalescedFailure>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, Error>>,
        T: Clone + Send + Sync + 'static,
    {
        // ── Bookkeeping. The map lock is released before anything awaits. ──
        let (tracked_flight, is_leader) = {
            let mut flights = self.flights.lock().unwrap_or_else(|e| e.into_inner());
            // Join the existing flight when it is still running (the account is
            // the same, so its result is a valid token for us whatever we hold)
            // or when it already consumed the very token we hold (the re-check).
            // Otherwise our token is newer than — or unrelated to — the recorded
            // attempt, so we replace it and run our own refresh.
            let can_join = flights.get(account).is_some_and(|flight| {
                flight.outcome().is_none()
                    || (!observed_token.is_empty() && flight.matches(observed_token))
            });

            if can_join {
                (flights.get(account).cloned(), false)
            } else if flights.len() >= self.capacity && !flights.contains_key(account) {
                // At capacity: refresh untracked rather than risk evicting a
                // live flight (see `capacity`).
                debug!(
                    account = %account,
                    tracked = flights.len(),
                    capacity = self.capacity,
                    "refresh coordinator at capacity; refreshing untracked"
                );
                (None, true)
            } else {
                let fresh = Flight::<T>::new(observed_token.to_string());
                flights.insert(account.to_string(), fresh.clone());
                (Some(fresh), true)
            }
        };

        // ── Leader: spend the token, exactly once. ──
        if is_leader {
            let shared = match refresh().await {
                Ok(value) => Ok(value),
                Err(e) => Err(Arc::new(e)),
            };
            if let Some(flight) = &tracked_flight {
                // Keep the entry: it is the re-check cache that stops the next
                // holder of this same token from spending it again.
                flight.complete(shared.clone());
            }
            debug!(
                account = %account,
                tracked = tracked_flight.is_some(),
                ok = shared.is_ok(),
                "coalesced refresh leader finished"
            );
            return match shared {
                Ok(value) => Ok(Coalesced {
                    value,
                    was_leader: true,
                }),
                // `Arc::try_unwrap` fails only while a waiter still holds the
                // error; the message copy keeps a failing refresh under load
                // from panicking the process.
                Err(e) => Err(CoalescedFailure::Own(
                    Arc::try_unwrap(e).unwrap_or_else(|shared| Error::Agent(shared.to_string())),
                )),
            };
        }

        // ── Follower. ──
        let flight = match tracked_flight {
            Some(flight) => flight,
            // Unreachable: only the leader path yields `None`.
            None => {
                return Err(CoalescedFailure::Own(Error::Agent(
                    "refresh coordinator bookkeeping error".to_string(),
                )));
            }
        };

        // Already finished on our token: this is the re-check. The leader has
        // already replaced the token we hold, so spending it would burn a
        // single-use token for nothing.
        if let Some(done) = flight.outcome() {
            return Self::share(done, &flight);
        }

        // Subscribe *before* the second read of `outcome`: if the leader
        // completes in this window the read sees it; if it completes after,
        // `changed()` wakes us. No lost wakeup either way.
        let mut done_rx = flight.done.subscribe();
        if let Some(done) = flight.outcome() {
            return Self::share(done, &flight);
        }
        if done_rx.changed().await.is_err() {
            // Sender dropped without a result — reachable only if the leader's
            // future panicked. Re-read once, then report a shared failure.
            return match flight.outcome() {
                Some(done) => Self::share(done, &flight),
                None => Err(Self::vanished(account, &flight)),
            };
        }
        match flight.outcome() {
            Some(done) => Self::share(done, &flight),
            None => Err(Self::vanished(account, &flight)),
        }
    }

    /// A leader that finished without publishing an outcome (panicked future).
    fn vanished(account: &str, flight: &Flight<T>) -> CoalescedFailure {
        CoalescedFailure::Shared {
            waiters: flight.waiters(),
            source: Arc::new(Error::Agent(format!(
                "coalesced refresh for account '{account}' ended without a result"
            ))),
        }
    }

    /// Turn a shared outcome into a caller's result, distinguishing who spent
    /// the token.
    fn share(outcome: Outcome<T>, flight: &Flight<T>) -> Result<Coalesced<T>, CoalescedFailure>
    where
        T: Clone,
    {
        flight.bump_waiter();
        match outcome.as_ref() {
            Ok(value) => Ok(Coalesced {
                value: (*value).clone(),
                was_leader: false,
            }),
            Err(source) => Err(CoalescedFailure::Shared {
                waiters: flight.waiters(),
                source: source.clone(),
            }),
        }
    }
}

/// The token type the crate's own refresh path produces.
pub type TokenCoordinator = RefreshCoordinator<crate::oauth_refresh::OAuthTokenResponse>;

static GLOBAL: OnceLock<TokenCoordinator> = OnceLock::new();

/// The process-wide coordinator used by [`crate::oauth_refresh`].
///
/// A `OnceLock` singleton because the callers that need coalescing construct a
/// fresh `OAuthRefresher` per call (see `CredentialPool::refresh_async`), so
/// per-instance state would not be shared and every concurrent caller would
/// still spend the token.
pub fn global() -> &'static TokenCoordinator {
    GLOBAL.get_or_init(TokenCoordinator::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tokio::time::sleep;

    #[tokio::test]
    async fn refresh_should_be_coalesced_across_concurrent_callers() {
        let coordinator = Arc::new(RefreshCoordinator::<String>::new());
        let refreshes = Arc::new(AtomicUsize::new(0));

        // 12 callers, same account, same (single-use) refresh token. One spends
        // it; the other 11 must observe that outcome.
        let mut handles = Vec::new();
        for _ in 0..12 {
            let coordinator = coordinator.clone();
            let refreshes = refreshes.clone();
            handles.push(tokio::spawn(async move {
                coordinator
                    .coalesce("anthropic/cred-1", "rt-0", || async move {
                        refreshes.fetch_add(1, Ordering::SeqCst);
                        // Long enough that every other caller has arrived and
                        // joined before the result lands.
                        sleep(Duration::from_millis(60)).await;
                        Ok("at-1".to_string())
                    })
                    .await
                    .expect("coalesced refresh succeeded")
            }));
        }

        let mut leaders = 0;
        for handle in handles {
            let got = handle.await.expect("join");
            assert_eq!(
                got.value, "at-1",
                "every caller must observe the same access token"
            );
            if got.was_leader {
                leaders += 1;
            }
        }

        assert_eq!(
            refreshes.load(Ordering::SeqCst),
            1,
            "exactly one refresh may be performed for N concurrent callers, or \
             the single-use rotating token is burned N times"
        );
        assert_eq!(leaders, 1, "exactly one caller is the leader");
    }

    #[tokio::test]
    async fn late_arrival_rechecks_instead_of_respending_the_token() {
        let coordinator = RefreshCoordinator::<String>::new();
        let refreshes = Arc::new(AtomicUsize::new(0));

        // Sequential callers all still holding the *stale* token. A naive
        // lock-without-recheck design refreshes 3 times here; each caller must
        // instead re-check and take the leader's result.
        for expected_leader in [true, false, false] {
            let refreshes = refreshes.clone();
            let got = coordinator
                .coalesce("codex/cred-1", "rt-0", || async move {
                    refreshes.fetch_add(1, Ordering::SeqCst);
                    Ok("at-1".to_string())
                })
                .await
                .expect("coalesced");
            assert_eq!(got.value, "at-1");
            assert_eq!(got.was_leader, expected_leader);
        }

        assert_eq!(
            refreshes.load(Ordering::SeqCst),
            1,
            "a caller still holding the spent token must re-check the completed \
             flight, not spend the token a second time"
        );

        // A caller that has since picked up the *new* refresh token may refresh
        // again: the cached result is not newer than what it holds.
        let later = refreshes.clone();
        let got = coordinator
            .coalesce("codex/cred-1", "rt-1", || async move {
                later.fetch_add(1, Ordering::SeqCst);
                Ok("at-2".to_string())
            })
            .await
            .expect("coalesced");
        assert_eq!(got.value, "at-2");
        assert!(got.was_leader);
        assert_eq!(refreshes.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn refresh_should_not_block_a_different_account() {
        let coordinator = Arc::new(RefreshCoordinator::<String>::new());
        let fast_finished_at = Arc::new(Mutex::new(None::<Instant>));
        let slow_finished_at = Arc::new(Mutex::new(None::<Instant>));

        let slow = {
            let coordinator = coordinator.clone();
            let slow_finished_at = slow_finished_at.clone();
            tokio::spawn(async move {
                coordinator
                    .coalesce("anthropic/cred-slow", "rt-slow", || async move {
                        sleep(Duration::from_millis(300)).await;
                        *slow_finished_at.lock().unwrap() = Some(Instant::now());
                        Ok("at-slow".to_string())
                    })
                    .await
                    .expect("slow account")
                    .value
            })
        };

        // Let the slow refresh actually claim its slot, so a global lock would
        // definitely be held when the fast one arrives.
        sleep(Duration::from_millis(50)).await;

        let fast = {
            let coordinator = coordinator.clone();
            let fast_finished_at = fast_finished_at.clone();
            tokio::spawn(async move {
                coordinator
                    .coalesce("anthropic/cred-fast", "rt-fast", || async move {
                        *fast_finished_at.lock().unwrap() = Some(Instant::now());
                        Ok("at-fast".to_string())
                    })
                    .await
                    .expect("fast account")
                    .value
            })
        };

        assert_eq!(fast.await.expect("join"), "at-fast");
        assert_eq!(slow.await.expect("join"), "at-slow");

        let fast_at = *fast_finished_at.lock().unwrap();
        let slow_at = *slow_finished_at.lock().unwrap();
        assert!(fast_at.is_some() && slow_at.is_some());
        assert!(
            fast_at < slow_at,
            "a slow refresh on one account must not delay a different account \
             (fast finished at {fast_at:?}, slow at {slow_at:?})"
        );
    }

    #[tokio::test]
    async fn shared_failure_is_distinguishable_from_an_own_failure() {
        let coordinator = Arc::new(RefreshCoordinator::<String>::new());
        let attempts = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..5 {
            let coordinator = coordinator.clone();
            let attempts = attempts.clone();
            handles.push(tokio::spawn(async move {
                coordinator
                    .coalesce("nous/cred-1", "rt-0", || async move {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        sleep(Duration::from_millis(40)).await;
                        Err(Error::Authentication("refresh_token_reused".to_string()))
                    })
                    .await
            }));
        }

        let mut own = 0;
        let mut shared = 0;
        for handle in handles {
            match handle.await.expect("join") {
                Ok(_) => panic!("refresh must fail"),
                Err(failure) => {
                    if failure.is_own() {
                        own += 1;
                        assert_eq!(failure.shared_attempts(), 1);
                    } else {
                        shared += 1;
                        // Every waiter can tell this was somebody else's
                        // attempt, so one bad refresh cannot look like five.
                        assert!(matches!(failure, CoalescedFailure::Shared { .. }));
                        assert!(
                            failure.to_string().contains("coalesced refresh failed"),
                            "shared failure must be labelled: {failure}"
                        );
                    }
                }
            }
        }

        assert_eq!(
            own, 1,
            "only the caller that spent the token owns the failure"
        );
        assert_eq!(shared, 4, "the other four must see a shared failure");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "N waiters must produce ONE provider-side failure, not N"
        );
    }

    #[tokio::test]
    async fn capacity_pressure_never_evicts_a_live_flight() {
        // Capacity 1: a second account must not be able to displace the first
        // account's running flight, or it would spend the same token.
        let coordinator = Arc::new(RefreshCoordinator::<String>::with_capacity(1));
        let refreshes = Arc::new(AtomicUsize::new(0));

        let slow = {
            let coordinator = coordinator.clone();
            let refreshes = refreshes.clone();
            tokio::spawn(async move {
                coordinator
                    .coalesce("acct-1", "rt-1", || async move {
                        refreshes.fetch_add(1, Ordering::SeqCst);
                        sleep(Duration::from_millis(150)).await;
                        Ok("at-1".to_string())
                    })
                    .await
                    .expect("account 1")
                    .value
            })
        };
        sleep(Duration::from_millis(30)).await;

        // A second account arrives while the map is full. It refreshes
        // untracked — uncoalesced, but never at the cost of a double spend.
        let other = coordinator
            .coalesce("acct-2", "rt-2", || async { Ok("at-2".to_string()) })
            .await
            .expect("account 2");
        assert_eq!(other.value, "at-2");
        assert!(other.was_leader);

        assert_eq!(slow.await.expect("join"), "at-1");
        assert_eq!(refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(coordinator.tracked(), 1, "the live flight was not evicted");
    }

    #[tokio::test]
    async fn forget_drops_the_cached_outcome() {
        let coordinator = RefreshCoordinator::<String>::new();
        let refreshes = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let refreshes = refreshes.clone();
            let _ = coordinator
                .coalesce("xai/cred-1", "rt-0", || async move {
                    refreshes.fetch_add(1, Ordering::SeqCst);
                    Ok("at".to_string())
                })
                .await;
        }
        assert_eq!(refreshes.load(Ordering::SeqCst), 1, "second call re-checks");
        assert_eq!(coordinator.tracked(), 1);

        coordinator.forget("xai/cred-1");
        assert_eq!(coordinator.tracked(), 0);

        let after_forget = refreshes.clone();
        let _ = coordinator
            .coalesce("xai/cred-1", "rt-0", || async move {
                after_forget.fetch_add(1, Ordering::SeqCst);
                Ok("at".to_string())
            })
            .await;
        assert_eq!(
            refreshes.load(Ordering::SeqCst),
            2,
            "forget() forces a fresh refresh"
        );
    }

    #[tokio::test]
    async fn empty_observed_token_never_shares_a_completed_result() {
        // A caller with nothing to compare must not be handed a cached result
        // that may belong to a different grant.
        let coordinator = RefreshCoordinator::<String>::new();
        let first = coordinator
            .coalesce("acct", "", || async { Ok("at-1".to_string()) })
            .await
            .expect("first");
        assert!(first.was_leader);
        let second = coordinator
            .coalesce("acct", "", || async { Ok("at-2".to_string()) })
            .await
            .expect("second");
        assert_eq!(second.value, "at-2");
        assert!(second.was_leader);
    }

    #[test]
    fn global_coordinator_is_shared_and_bounded() {
        // Callers must share one coordinator or the coalescing is theatre.
        assert!(std::ptr::eq(global(), global()));
        assert!(global().capacity >= 1);
    }
}
