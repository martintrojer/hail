//! Exponential backoff with full jitter for JMAP EventSource reconnects.
//!
//! Schedule from the task contract: 1s, 2s, 4s, 8s, 16s, then capped
//! at 60s. Each delivered delay is "full jitter" — a uniform random
//! draw in `[0, base)` per AWS's classic recommendation. Keeps thunder
//! herds from forming when Stalwart bounces.

use std::time::Duration;

const BASE_SECS: u64 = 1;
const MAX_SECS: u64 = 60;
/// Maximum power-of-two factor applied to BASE_SECS before we clamp.
/// `1 << 5 == 32`, `1 << 6 == 64`, so at attempt 6 we're already past
/// the 60s ceiling.
const MAX_SHIFT: u32 = 6;

/// Backoff state machine. Caller owns it; one per per-user supervisor
/// task. `next()` advances and returns the delay to sleep for; reset
/// after a successful connection to start the next retry burst at 1s.
#[derive(Debug, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    /// Construct a fresh schedule starting from attempt 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute the next delay and advance internal state. Calls use
    /// `rand::random_range` for full jitter; tests use [`Self::base`]
    /// for deterministic upper bounds.
    pub fn next_delay(&mut self) -> Duration {
        let base = self.base();
        self.attempt = self.attempt.saturating_add(1);
        // Full jitter: uniform in [0, base_ms).
        let base_ms = base.as_millis() as u64;
        let jittered = if base_ms == 0 {
            0
        } else {
            rand::random_range(0..base_ms)
        };
        Duration::from_millis(jittered)
    }

    /// Upper bound (pre-jitter) of the next delay this state will
    /// produce. Used by tests to assert the schedule grows.
    #[must_use]
    pub fn base(&self) -> Duration {
        let shift = self.attempt.min(MAX_SHIFT);
        let secs = (BASE_SECS << shift).min(MAX_SECS);
        Duration::from_secs(secs)
    }

    /// Reset to attempt 0 after a successful connection. Subsequent
    /// disconnects start the schedule fresh at 1s.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_schedule_matches_contract() {
        let mut b = Backoff::new();
        // 1, 2, 4, 8, 16, 32, 60, 60, 60, ...
        let expected_secs = [1, 2, 4, 8, 16, 32, 60, 60, 60];
        for exp in expected_secs {
            assert_eq!(b.base(), Duration::from_secs(exp), "attempt={}", b.attempt);
            let _ = b.next_delay();
        }
    }

    #[test]
    fn next_delay_is_within_base() {
        let mut b = Backoff::new();
        for _ in 0..20 {
            let base = b.base();
            let d = b.next_delay();
            assert!(d <= base, "delay {d:?} exceeded base {base:?}");
        }
    }

    #[test]
    fn reset_returns_to_first_attempt() {
        let mut b = Backoff::new();
        for _ in 0..4 {
            let _ = b.next_delay();
        }
        b.reset();
        assert_eq!(b.base(), Duration::from_secs(1));
    }
}
