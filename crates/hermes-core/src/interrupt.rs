//! Thread-safe interrupt signal for graceful cancellation.
//!
//! Provides a lightweight `InterruptFlag` backed by `Arc<AtomicBool>` that
//! can be shared across threads and tasks.  Useful for signalling tool
//! execution or agent loop cancellation without needing a full channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;

/// Error returned by [`InterruptFlag::check`] when the flag has been triggered.
#[derive(Debug, Clone)]
pub struct Interrupted(pub String);

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Interrupted: {}", self.0)
    }
}

impl std::error::Error for Interrupted {}

/// A thread-safe interrupt signal that can be shared across tasks.
///
/// Wraps an `Arc<AtomicBool>` so cloning produces a handle to the same
/// underlying flag — perfect for injecting into concurrent tool executions
/// or agent sub-loops.
///
/// # Examples
///
/// ```ignore
/// let flag = InterruptFlag::new();
/// let guard = flag.clone();
///
/// tokio::spawn(async move {
///     guard.check("background work").unwrap();
/// });
///
/// flag.trigger();
/// ```
#[derive(Debug, Clone)]
pub struct InterruptFlag {
    triggered: Arc<AtomicBool>,
}

impl InterruptFlag {
    /// Create a new `InterruptFlag` in the untriggered state.
    pub fn new() -> Self {
        Self {
            triggered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set the flag to the triggered state.
    ///
    /// All clones of this flag will now report `is_triggered() == true`.
    pub fn trigger(&self) {
        self.triggered.store(true, Ordering::SeqCst);
    }

    /// Reset the flag back to the untriggered state.
    pub fn reset(&self) {
        self.triggered.store(false, Ordering::SeqCst);
    }

    /// Return `true` if the flag has been triggered.
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    /// Check the flag and return `Err(Interrupted(msg))` if triggered.
    ///
    /// This is a convenience wrapper for use inside tool execution loops:
    ///
    /// ```ignore
    /// fn do_work(&self, flag: &InterruptFlag) -> Result<()> {
    ///     flag.check("processing step 1")?;
    ///     // ...
    ///     flag.check("processing step 2")?;
    ///     Ok(())
    /// }
    /// ```
    pub fn check(&self, msg: &str) -> std::result::Result<(), Interrupted> {
        if self.is_triggered() {
            Err(Interrupted(msg.to_string()))
        } else {
            Ok(())
        }
    }

    /// Block the current task until the flag is triggered or the timeout
    /// expires.
    ///
    /// Polls the flag every 50 ms.  Returns `true` if the flag was triggered
    /// within the timeout, `false` if the timeout elapsed first.
    pub async fn wait_for_interrupt(&self, timeout: Duration) -> bool {
        let start = tokio::time::Instant::now();
        loop {
            if self.is_triggered() {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Default for InterruptFlag {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: AtomicBool is Send + Sync, and Arc is Send + Sync.
// The struct contains no other state.
unsafe impl Send for InterruptFlag {}
unsafe impl Sync for InterruptFlag {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_is_not_triggered() {
        let flag = InterruptFlag::new();
        assert!(!flag.is_triggered());
    }

    #[test]
    fn test_trigger_sets_flag() {
        let flag = InterruptFlag::new();
        flag.trigger();
        assert!(flag.is_triggered());
    }

    #[test]
    fn test_reset_clears_flag() {
        let flag = InterruptFlag::new();
        flag.trigger();
        assert!(flag.is_triggered());
        flag.reset();
        assert!(!flag.is_triggered());
    }

    #[test]
    fn test_check_ok_when_not_triggered() {
        let flag = InterruptFlag::new();
        assert!(flag.check("test").is_ok());
    }

    #[test]
    fn test_check_err_when_triggered() {
        let flag = InterruptFlag::new();
        flag.trigger();
        let err = flag.check("test message").unwrap_err();
        assert!(err.to_string().contains("test message"));
    }

    #[test]
    fn test_clone_shares_state() {
        let flag = InterruptFlag::new();
        let flag2 = flag.clone();
        flag.trigger();
        assert!(flag2.is_triggered());
    }

    #[test]
    fn test_clone_reset() {
        let flag = InterruptFlag::new();
        let flag2 = flag.clone();
        flag.trigger();
        flag2.reset();
        assert!(!flag.is_triggered());
    }

    #[tokio::test]
    async fn test_wait_for_interrupt_timeout() {
        let flag = InterruptFlag::new();
        let result = flag.wait_for_interrupt(Duration::from_millis(10)).await;
        assert!(!result); // timeout, not triggered
    }

    #[tokio::test]
    async fn test_wait_for_interrupt_triggered() {
        let flag = InterruptFlag::new();
        let flag2 = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            flag2.trigger();
        });
        let result = flag.wait_for_interrupt(Duration::from_secs(5)).await;
        assert!(result);
    }
}
