use tokio::sync::RwLock;
use tokio::time::Duration;
use tracing::info;
// ============================================================================
// RATE LIMITER
// ============================================================================

/// Handles rate limiting across all concurrent tasks with non-contending reads
pub struct RateLimiter {
    until: RwLock<Option<tokio::time::Instant>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            until: RwLock::new(None),
        }
    }

    /// Sets a rate limit that all tasks must wait for (monotonically extending any active limit)
    pub async fn set_rate_limit(&self, duration_secs: u64) {
        let wake_time = tokio::time::Instant::now() + Duration::from_secs(duration_secs);
        let mut guard = self.until.write().await;
        if guard.is_none_or(|prev| wake_time > prev) {
            *guard = Some(wake_time);
            info!(
                "Global rate limit set: all tasks sleeping {} seconds",
                duration_secs
            );
        }
    }

    /// Waits if rate limit is currently active without blocking other readers
    pub async fn wait_if_limited(&self) {
        loop {
            let until = *self.until.read().await;
            if let Some(wake_time) = until
                && tokio::time::Instant::now() < wake_time
            {
                tokio::time::sleep_until(wake_time).await;
                continue;
            }
            break;
        }
    }
}
