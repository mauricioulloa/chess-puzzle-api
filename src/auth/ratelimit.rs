//! Fixed-window rate limiting.
//!
//! Hand-rolled rather than pulled from a crate because the limit varies per
//! subject: an anonymous caller and a key holder share the same code path but
//! not the same quota, and the response has to report the remaining budget in
//! headers. A fixed window is coarse at the boundary — a caller can spend two
//! windows' worth across one — but it is predictable, cheap, and honest about
//! when it resets, which matters more than smoothing for a courtesy limit.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subject {
    Key(i64),
    Ip(IpAddr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    /// Seconds until the current window rolls over.
    pub reset_after: u64,
}

struct Window {
    started: Instant,
    count: u32,
}

#[derive(Default)]
pub struct RateLimiter {
    windows: Mutex<HashMap<Subject, Window>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check(&self, subject: Subject, limit: u32) -> Decision {
        self.check_at(subject, limit, Instant::now())
    }

    /// Time is injected so the window rollover can be tested without sleeping.
    pub fn check_at(&self, subject: Subject, limit: u32, now: Instant) -> Decision {
        let mut windows = self.windows.lock().expect("rate limiter lock");
        let window = windows.entry(subject).or_insert(Window {
            started: now,
            count: 0,
        });

        if now.duration_since(window.started) >= WINDOW {
            window.started = now;
            window.count = 0;
        }

        let elapsed = now.duration_since(window.started);
        let reset_after = WINDOW.saturating_sub(elapsed).as_secs().max(1);

        if window.count >= limit {
            return Decision {
                allowed: false,
                limit,
                remaining: 0,
                reset_after,
            };
        }

        window.count += 1;
        Decision {
            allowed: true,
            limit,
            remaining: limit.saturating_sub(window.count),
            reset_after,
        }
    }

    /// Drops windows nobody has touched for two full periods. Without this the
    /// map grows once per distinct client address, forever.
    pub fn prune(&self) {
        self.prune_at(Instant::now());
    }

    pub fn prune_at(&self, now: Instant) {
        let cutoff = WINDOW * 2;
        self.windows
            .lock()
            .expect("rate limiter lock")
            .retain(|_, window| now.duration_since(window.started) < cutoff);
    }

    pub fn tracked(&self) -> usize {
        self.windows.lock().expect("rate limiter lock").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> Subject {
        Subject::Ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, last)))
    }

    #[test]
    fn allows_up_to_the_limit_then_refuses() {
        let limiter = RateLimiter::new();
        for expected_remaining in (0..3).rev() {
            let decision = limiter.check(ip(1), 3);
            assert!(decision.allowed);
            assert_eq!(decision.remaining, expected_remaining);
        }

        let decision = limiter.check(ip(1), 3);
        assert!(!decision.allowed);
        assert_eq!(decision.remaining, 0);
        assert!(decision.reset_after <= 60 && decision.reset_after >= 1);
    }

    #[test]
    fn subjects_do_not_share_a_budget() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            assert!(limiter.check(ip(1), 3).allowed);
        }
        assert!(!limiter.check(ip(1), 3).allowed);
        assert!(
            limiter.check(ip(2), 3).allowed,
            "one caller exhausting its quota must not block another"
        );
        assert!(limiter.check(Subject::Key(1), 3).allowed);
    }

    #[test]
    fn the_window_rolls_over() {
        let limiter = RateLimiter::new();
        let start = Instant::now();

        for _ in 0..3 {
            assert!(limiter.check_at(ip(1), 3, start).allowed);
        }
        assert!(!limiter.check_at(ip(1), 3, start).allowed);

        let later = start + WINDOW + Duration::from_secs(1);
        let decision = limiter.check_at(ip(1), 3, later);
        assert!(decision.allowed, "a new window starts with a full budget");
        assert_eq!(decision.remaining, 2);
    }

    #[test]
    fn a_zero_limit_refuses_everything() {
        let limiter = RateLimiter::new();
        assert!(!limiter.check(ip(1), 0).allowed);
    }

    #[test]
    fn pruning_drops_only_stale_windows() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        limiter.check_at(ip(1), 10, start);
        limiter.check_at(ip(2), 10, start + WINDOW * 2);
        assert_eq!(limiter.tracked(), 2);

        limiter.prune_at(start + WINDOW * 2 + Duration::from_secs(1));
        assert_eq!(limiter.tracked(), 1, "the fresh window must survive");
    }
}
