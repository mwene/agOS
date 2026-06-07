use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        RateLimitConfig {
            requests_per_minute: 100,
        }
    }
}

struct Window {
    count: u32,
    window_start: Instant,
}

pub struct RateLimiter {
    windows: std::sync::Mutex<HashMap<String, Window>>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        RateLimiter {
            windows: std::sync::Mutex::new(HashMap::new()),
            config,
        }
    }

    pub fn check(&self, key: &str) -> Result<(), String> {
        let now = Instant::now();
        let window_duration = Duration::from_secs(60);
        let limit = self.config.requests_per_minute;

        let mut windows = self.windows.lock().unwrap();

        if let Some(window) = windows.get_mut(key) {
            if now - window.window_start > window_duration {
                *window = Window {
                    count: 1,
                    window_start: now,
                };
                return Ok(());
            }

            if window.count >= limit {
                return Err("rate limit exceeded".to_string());
            }

            window.count += 1;
        } else {
            windows.insert(
                key.to_string(),
                Window {
                    count: 1,
                    window_start: now,
                },
            );
        }

        Ok(())
    }

    pub fn get_config(&self) -> RateLimitConfig {
        self.config.clone()
    }

    pub fn set_config(&mut self, config: RateLimitConfig) {
        self.config = config;
    }

    pub fn force_window_elapsed(&self, key: &str, secs: u64) {
        let mut windows = self.windows.lock().unwrap();
        if let Some(window) = windows.get_mut(key) {
            window.window_start -= Duration::from_secs(secs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allows_first_request() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_minute: 10,
        });
        assert!(limiter.check("test-key").is_ok());
    }

    #[test]
    fn test_allows_within_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_minute: 5,
        });
        for _ in 0..5 {
            assert!(limiter.check("test-key").is_ok());
        }
    }

    #[test]
    fn test_rejects_over_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_minute: 3,
        });
        assert!(limiter.check("test-key").is_ok());
        assert!(limiter.check("test-key").is_ok());
        assert!(limiter.check("test-key").is_ok());
        assert!(limiter.check("test-key").is_err());
    }

    #[test]
    fn test_different_keys_independent() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_minute: 2,
        });
        assert!(limiter.check("key-a").is_ok());
        assert!(limiter.check("key-a").is_ok());
        assert!(limiter.check("key-a").is_err());

        assert!(limiter.check("key-b").is_ok());
        assert!(limiter.check("key-b").is_ok());
    }

    #[test]
    fn test_window_resets_after_expiry() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_minute: 2,
        });
        assert!(limiter.check("test-key").is_ok());
        assert!(limiter.check("test-key").is_ok());
        assert!(limiter.check("test-key").is_err());

        limiter.force_window_elapsed("test-key", 61);

        assert!(limiter.check("test-key").is_ok());
    }
}
