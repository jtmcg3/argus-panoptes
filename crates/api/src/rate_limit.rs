//! Rate limiting middleware for API endpoints.
//!
//! SEC-006: Implements request rate limiting to prevent DoS attacks.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Configuration for rate limiting.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per window.
    pub max_requests: u32,
    /// Window duration.
    pub window: Duration,
    /// Maximum concurrent connections per IP.
    pub max_concurrent: u32,
    /// Maximum request body size in bytes.
    pub max_body_size: usize,
    /// Maximum WebSocket connections per IP.
    pub max_websocket_per_ip: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 100,                      // 100 requests
            window: Duration::from_secs(60),        // per minute
            max_concurrent: 50,                     // 50 concurrent per IP
            max_body_size: 1024 * 1024,             // 1 MB max body
            max_websocket_per_ip: 10,               // 10 WebSocket connections per IP
        }
    }
}

/// Entry tracking requests from an IP address.
#[derive(Debug)]
struct RateLimitEntry {
    /// Timestamps of requests within the window.
    requests: Vec<Instant>,
    /// Current concurrent connection count.
    concurrent: u32,
    /// Current WebSocket connection count.
    websocket_count: u32,
}

impl RateLimitEntry {
    fn new() -> Self {
        Self {
            requests: Vec::with_capacity(100),
            concurrent: 0,
            websocket_count: 0,
        }
    }

    /// Prune old requests outside the window.
    fn prune(&mut self, window: Duration) {
        let cutoff = Instant::now() - window;
        self.requests.retain(|t| *t > cutoff);
    }

    /// Check if rate limit is exceeded.
    fn is_rate_limited(&self, max_requests: u32) -> bool {
        self.requests.len() >= max_requests as usize
    }

    /// Record a new request.
    fn record_request(&mut self) {
        self.requests.push(Instant::now());
    }
}

/// Thread-safe rate limiter using sliding window algorithm.
#[derive(Debug)]
pub struct RateLimiter {
    config: RateLimitConfig,
    entries: RwLock<HashMap<IpAddr, RateLimitEntry>>,
    last_cleanup: RwLock<Instant>,
}

impl RateLimiter {
    /// Create a new rate limiter with the given configuration.
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            entries: RwLock::new(HashMap::new()),
            last_cleanup: RwLock::new(Instant::now()),
        }
    }

    /// Check if a request from the given IP should be allowed.
    ///
    /// Returns `true` if allowed, `false` if rate limited.
    pub fn check_request(&self, ip: IpAddr) -> bool {
        self.maybe_cleanup();

        let mut entries = self.entries.write();
        let entry = entries.entry(ip).or_insert_with(RateLimitEntry::new);

        // Prune old requests
        entry.prune(self.config.window);

        // Check rate limit
        if entry.is_rate_limited(self.config.max_requests) {
            return false;
        }

        // Record the request
        entry.record_request();
        true
    }

    /// Acquire a concurrent connection slot.
    ///
    /// Returns `true` if slot acquired, `false` if at limit.
    pub fn acquire_concurrent(&self, ip: IpAddr) -> bool {
        let mut entries = self.entries.write();
        let entry = entries.entry(ip).or_insert_with(RateLimitEntry::new);

        if entry.concurrent >= self.config.max_concurrent {
            return false;
        }

        entry.concurrent += 1;
        true
    }

    /// Release a concurrent connection slot.
    pub fn release_concurrent(&self, ip: IpAddr) {
        let mut entries = self.entries.write();
        if let Some(entry) = entries.get_mut(&ip) {
            entry.concurrent = entry.concurrent.saturating_sub(1);
        }
    }

    /// Acquire a WebSocket connection slot.
    ///
    /// Returns `true` if slot acquired, `false` if at limit.
    pub fn acquire_websocket(&self, ip: IpAddr) -> bool {
        let mut entries = self.entries.write();
        let entry = entries.entry(ip).or_insert_with(RateLimitEntry::new);

        if entry.websocket_count >= self.config.max_websocket_per_ip {
            return false;
        }

        entry.websocket_count += 1;
        true
    }

    /// Release a WebSocket connection slot.
    pub fn release_websocket(&self, ip: IpAddr) {
        let mut entries = self.entries.write();
        if let Some(entry) = entries.get_mut(&ip) {
            entry.websocket_count = entry.websocket_count.saturating_sub(1);
        }
    }

    /// Get max body size from config.
    pub fn max_body_size(&self) -> usize {
        self.config.max_body_size
    }

    /// Periodically cleanup stale entries (every 5 minutes).
    fn maybe_cleanup(&self) {
        const CLEANUP_INTERVAL: Duration = Duration::from_secs(300);

        let should_cleanup = {
            let last = self.last_cleanup.read();
            last.elapsed() > CLEANUP_INTERVAL
        };

        if should_cleanup {
            let mut entries = self.entries.write();
            let mut last = self.last_cleanup.write();

            // Double-check after acquiring write lock
            if last.elapsed() > CLEANUP_INTERVAL {
                // Remove entries with no recent requests and no active connections
                entries.retain(|_, entry| {
                    entry.prune(self.config.window);
                    !entry.requests.is_empty()
                        || entry.concurrent > 0
                        || entry.websocket_count > 0
                });

                *last = Instant::now();
            }
        }
    }

    /// Get current stats for monitoring.
    pub fn stats(&self) -> RateLimitStats {
        let entries = self.entries.read();
        let total_ips = entries.len();
        let total_concurrent: u32 = entries.values().map(|e| e.concurrent).sum();
        let total_websockets: u32 = entries.values().map(|e| e.websocket_count).sum();

        RateLimitStats {
            tracked_ips: total_ips,
            total_concurrent,
            total_websockets,
        }
    }
}

/// Statistics about rate limiter state.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RateLimitStats {
    pub tracked_ips: usize,
    pub total_concurrent: u32,
    pub total_websockets: u32,
}

/// RAII guard for concurrent connection tracking.
pub struct ConcurrentGuard {
    limiter: Arc<RateLimiter>,
    ip: IpAddr,
}

impl ConcurrentGuard {
    pub fn new(limiter: Arc<RateLimiter>, ip: IpAddr) -> Option<Self> {
        if limiter.acquire_concurrent(ip) {
            Some(Self { limiter, ip })
        } else {
            None
        }
    }
}

impl Drop for ConcurrentGuard {
    fn drop(&mut self) {
        self.limiter.release_concurrent(self.ip);
    }
}

/// RAII guard for WebSocket connection tracking.
pub struct WebSocketGuard {
    limiter: Arc<RateLimiter>,
    ip: IpAddr,
}

impl WebSocketGuard {
    pub fn new(limiter: Arc<RateLimiter>, ip: IpAddr) -> Option<Self> {
        if limiter.acquire_websocket(ip) {
            Some(Self { limiter, ip })
        } else {
            None
        }
    }
}

impl Drop for WebSocketGuard {
    fn drop(&mut self) {
        self.limiter.release_websocket(self.ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_ip() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
    }

    #[test]
    fn test_rate_limit_allows_under_limit() {
        let config = RateLimitConfig {
            max_requests: 10,
            window: Duration::from_secs(60),
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);

        for _ in 0..10 {
            assert!(limiter.check_request(test_ip()));
        }
    }

    #[test]
    fn test_rate_limit_blocks_over_limit() {
        let config = RateLimitConfig {
            max_requests: 5,
            window: Duration::from_secs(60),
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);

        for _ in 0..5 {
            assert!(limiter.check_request(test_ip()));
        }

        // 6th request should be blocked
        assert!(!limiter.check_request(test_ip()));
    }

    #[test]
    fn test_concurrent_connection_limit() {
        let config = RateLimitConfig {
            max_concurrent: 3,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);
        let ip = test_ip();

        // Acquire 3 slots
        assert!(limiter.acquire_concurrent(ip));
        assert!(limiter.acquire_concurrent(ip));
        assert!(limiter.acquire_concurrent(ip));

        // 4th should fail
        assert!(!limiter.acquire_concurrent(ip));

        // Release one
        limiter.release_concurrent(ip);

        // Now should succeed
        assert!(limiter.acquire_concurrent(ip));
    }

    #[test]
    fn test_websocket_limit() {
        let config = RateLimitConfig {
            max_websocket_per_ip: 2,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);
        let ip = test_ip();

        assert!(limiter.acquire_websocket(ip));
        assert!(limiter.acquire_websocket(ip));
        assert!(!limiter.acquire_websocket(ip));

        limiter.release_websocket(ip);
        assert!(limiter.acquire_websocket(ip));
    }

    #[test]
    fn test_different_ips_have_separate_limits() {
        let config = RateLimitConfig {
            max_requests: 2,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);

        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

        // Both IPs should have their own limit
        assert!(limiter.check_request(ip1));
        assert!(limiter.check_request(ip1));
        assert!(!limiter.check_request(ip1)); // ip1 is limited

        // ip2 should still be allowed
        assert!(limiter.check_request(ip2));
        assert!(limiter.check_request(ip2));
        assert!(!limiter.check_request(ip2)); // now ip2 is limited too
    }

    #[test]
    fn test_concurrent_guard_releases_on_drop() {
        let config = RateLimitConfig {
            max_concurrent: 1,
            ..Default::default()
        };
        let limiter = Arc::new(RateLimiter::new(config));
        let ip = test_ip();

        {
            let _guard = ConcurrentGuard::new(limiter.clone(), ip).unwrap();
            // Should be at limit now
            assert!(!limiter.acquire_concurrent(ip));
        }

        // Guard dropped, should be able to acquire again
        assert!(limiter.acquire_concurrent(ip));
    }

    #[test]
    fn test_stats() {
        let config = RateLimitConfig::default();
        let limiter = RateLimiter::new(config);

        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

        limiter.check_request(ip1);
        limiter.acquire_concurrent(ip1);
        limiter.acquire_websocket(ip2);

        let stats = limiter.stats();
        assert_eq!(stats.tracked_ips, 2);
        assert_eq!(stats.total_concurrent, 1);
        assert_eq!(stats.total_websockets, 1);
    }
}
