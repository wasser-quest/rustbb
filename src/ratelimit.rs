//! Fixed-window limits per key (an address, or an account), kept in memory. Good for one
//! process; several processes would each keep their own counts.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_KEYS: usize = 50_000;

pub struct Limiter {
    windows: Mutex<HashMap<String, (Instant, u32)>>,
    /// False when a load balancer in front does the limiting (FORUM_RATE_LIMIT=off).
    enabled: bool,
}

impl Default for Limiter {
    fn default() -> Self {
        Self { windows: Mutex::new(HashMap::new()), enabled: true }
    }
}

impl Limiter {
    /// The limiter as the environment sets it: `FORUM_RATE_LIMIT=off` leaves the limiting to the
    /// load balancer in front of the forum (see deploy/nginx-rate-limit.conf.example).
    pub fn from_env() -> Self {
        let enabled = std::env::var("FORUM_RATE_LIMIT").map_or(true, |value| value != "off");
        Self { windows: Mutex::new(HashMap::new()), enabled }
    }

    /// Counts one action for `key`. Returns false once `limit` actions happened in `window`.
    pub fn allow(&self, key: &str, limit: u32, window: Duration) -> bool {
        if !self.enabled {
            return true;
        }
        self.allow_at(key, limit, window, Instant::now())
    }

    fn allow_at(&self, key: &str, limit: u32, window: Duration, now: Instant) -> bool {
        let Ok(mut windows) = self.windows.lock() else {
            return false;
        };
        if windows.len() > MAX_KEYS {
            windows.retain(|_, (start, _)| now.duration_since(*start) < window);
        }
        let entry = windows.entry(key.to_owned()).or_insert((now, 0));
        if now.duration_since(entry.0) >= window {
            *entry = (now, 0);
        }
        if entry.1 >= limit {
            return false;
        }
        entry.1 += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_resets_after_the_window() {
        let limiter = Limiter::default();
        let window = Duration::from_secs(60);
        let start = Instant::now();
        assert!(limiter.allow_at("ip", 2, window, start));
        assert!(limiter.allow_at("ip", 2, window, start));
        assert!(!limiter.allow_at("ip", 2, window, start));
        assert!(limiter.allow_at("other", 2, window, start));
        assert!(limiter.allow_at("ip", 2, window, start + window));
    }
}
