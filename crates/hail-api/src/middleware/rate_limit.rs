//! Tiny in-memory token bucket keyed by remote IP.
//!
//! Used to throttle `/api/auth/login` to 5 attempts / 60s per source IP.
//! This deliberately avoids `tower-governor`: that crate pulls in
//! `governor` + `dashmap` + `nonzero_ext`, all of which would be net-new
//! transitive surface for one rate-limited endpoint. For our scale (≤ 20
//! users, single host) a `Mutex<HashMap<IpAddr, Bucket>>` is fine.
//!
//! Buckets are evicted lazily on access; abandoned IPs sit in the map
//! until the next probe touches them. We also cap the map size at
//! `MAX_TRACKED_IPS` and evict the oldest-seen entry when full, so a
//! flooding attacker can't blow heap.
//!
//! Future work: swap this for `tower-governor` once we have a second
//! reason to add it.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Max successful + failed attempts per window.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 5;
/// Sliding window length.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);
/// Cap on tracked IPs to bound memory under an attacker spraying source IPs.
const MAX_TRACKED_IPS: usize = 4096;

/// Per-IP attempt counter.
#[derive(Debug)]
struct Bucket {
    count: u32,
    /// When the current window started. Counter resets when `now -
    /// window_start > window`.
    window_start: Instant,
}

/// Thread-safe rate limiter shared via `Arc` in `AppState`.
pub struct IpRateLimiter {
    inner: Mutex<HashMap<IpAddr, Bucket>>,
    max_attempts: u32,
    window: Duration,
}

impl IpRateLimiter {
    /// Construct a limiter with explicit knobs. Use [`Self::default`] for
    /// the production tuning (5 / 60s).
    pub fn new(max_attempts: u32, window: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            max_attempts,
            window,
        }
    }

    /// Try to register one attempt from `ip`. Returns `true` when the
    /// caller is allowed to proceed, `false` if it's currently rate-limited.
    ///
    /// This counts both successful and failed login attempts — successes
    /// are cheap to count, so we don't bother distinguishing.
    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut map = self.inner.lock().expect("rate-limit mutex poisoned");

        // Evict oldest if cap reached and `ip` not present. Cheapest
        // strategy that bounds memory; collisions on hot IPs are still
        // counted correctly.
        if map.len() >= MAX_TRACKED_IPS && !map.contains_key(&ip) {
            if let Some((&oldest_ip, _)) = map
                .iter()
                .min_by_key(|(_, b)| b.window_start)
            {
                map.remove(&oldest_ip);
            }
        }

        let bucket = map.entry(ip).or_insert_with(|| Bucket {
            count: 0,
            window_start: now,
        });

        if now.duration_since(bucket.window_start) > self.window {
            bucket.count = 0;
            bucket.window_start = now;
        }

        if bucket.count >= self.max_attempts {
            return false;
        }
        bucket.count += 1;
        true
    }
}

impl Default for IpRateLimiter {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_ATTEMPTS, DEFAULT_WINDOW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn allows_up_to_max_then_blocks() {
        let lim = IpRateLimiter::new(3, Duration::from_secs(60));
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(lim.check(ip));
        assert!(lim.check(ip));
        assert!(lim.check(ip));
        assert!(!lim.check(ip));
    }

    #[test]
    fn different_ips_are_independent() {
        let lim = IpRateLimiter::new(1, Duration::from_secs(60));
        assert!(lim.check(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(lim.check(IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2))));
        assert!(!lim.check(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[test]
    fn window_resets() {
        let lim = IpRateLimiter::new(1, Duration::from_millis(50));
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(lim.check(ip));
        assert!(!lim.check(ip));
        std::thread::sleep(Duration::from_millis(80));
        assert!(lim.check(ip));
    }
}
