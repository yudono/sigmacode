use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, TokenBucket>>>,
    default_config: RateLimitConfig,
}

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_tokens: u32,
    pub refill_interval: Duration,
    pub tokens_per_refill: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_tokens: 60,
            refill_interval: Duration::from_secs(1),
            tokens_per_refill: 10,
        }
    }
}

#[derive(Debug)]
struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(config: &RateLimitConfig) -> Self {
        Self {
            tokens: config.max_tokens as f64,
            max_tokens: config.max_tokens as f64,
            refill_rate: config.tokens_per_refill as f64 / config.refill_interval.as_secs_f64(),
            last_refill: Instant::now(),
        }
    }

    fn try_consume(&mut self, tokens: u32) -> bool {
        self.refill();
        if self.tokens >= tokens as f64 {
            self.tokens -= tokens as f64;
            true
        } else {
            false
        }
    }

    fn wait_time(&self, tokens: u32) -> Duration {
        let deficit = tokens as f64 - self.tokens;
        if deficit <= 0.0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(deficit / self.refill_rate)
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        let refill_amount = elapsed * self.refill_rate;
        self.tokens = (self.tokens + refill_amount).min(self.max_tokens);
        self.last_refill = now;
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(RateLimitConfig::default())
    }
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            default_config: config,
        }
    }

    pub fn with_provider_limit(
        self,
        provider: &str,
        max_tokens: u32,
        refill_interval: Duration,
        tokens_per_refill: u32,
    ) -> Self {
        let config = RateLimitConfig {
            max_tokens,
            refill_interval,
            tokens_per_refill,
        };
        {
            let mut buckets = self.buckets.blocking_lock();
            buckets.insert(
                provider.to_string(),
                TokenBucket::new(&config),
            );
        }
        self
    }

    pub async fn acquire(&self, key: &str) -> RateLimitResult {
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(&self.default_config));

        if bucket.try_consume(1) {
            RateLimitResult::Allowed
        } else {
            let wait = bucket.wait_time(1);
            RateLimitResult::Limited { retry_after: wait }
        }
    }

    pub async fn acquire_many(&self, key: &str, tokens: u32) -> RateLimitResult {
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(&self.default_config));

        if bucket.try_consume(tokens) {
            RateLimitResult::Allowed
        } else {
            let wait = bucket.wait_time(tokens);
            RateLimitResult::Limited { retry_after: wait }
        }
    }

    pub async fn reset(&self, key: &str) {
        let mut buckets = self.buckets.lock().await;
        buckets.insert(key.to_string(), TokenBucket::new(&self.default_config));
    }

    pub async fn remaining(&self, key: &str) -> u32 {
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(&self.default_config));
        bucket.refill();
        bucket.tokens as u32
    }
}

#[derive(Debug, Clone)]
pub enum RateLimitResult {
    Allowed,
    Limited { retry_after: Duration },
}

impl RateLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, RateLimitResult::Allowed)
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            RateLimitResult::Allowed => None,
            RateLimitResult::Limited { retry_after } => Some(*retry_after),
        }
    }
}

#[derive(Clone)]
pub struct LlmRateLimiter {
    pub provider: RateLimiter,
    pub tool: RateLimiter,
}

impl Default for LlmRateLimiter {
    fn default() -> Self {
        Self {
            provider: RateLimiter::new(RateLimitConfig {
                max_tokens: 30,
                refill_interval: Duration::from_secs(1),
                tokens_per_refill: 5,
            }),
            tool: RateLimiter::new(RateLimitConfig {
                max_tokens: 20,
                refill_interval: Duration::from_secs(1),
                tokens_per_refill: 10,
            }),
        }
    }
}

impl LlmRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn check_llm_request(&self, provider: &str) -> RateLimitResult {
        self.provider.acquire(provider).await
    }

    pub async fn check_tool_execution(&self, tool_name: &str) -> RateLimitResult {
        self.tool.acquire(tool_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_tokens: 5,
            refill_interval: Duration::from_secs(1),
            tokens_per_refill: 5,
        });

        for _ in 0..5 {
            let result = limiter.acquire("test").await;
            assert!(result.is_allowed());
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_limits_excess() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_tokens: 2,
            refill_interval: Duration::from_secs(60),
            tokens_per_refill: 1,
        });

        assert!(limiter.acquire("test").await.is_allowed());
        assert!(limiter.acquire("test").await.is_allowed());

        let result = limiter.acquire("test").await;
        assert!(!result.is_allowed());
        assert!(result.retry_after().is_some());
    }

    #[tokio::test]
    async fn test_rate_limiter_refill() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_tokens: 1,
            refill_interval: Duration::from_millis(50),
            tokens_per_refill: 1,
        });

        assert!(limiter.acquire("test").await.is_allowed());
        assert!(!limiter.acquire("test").await.is_allowed());

        tokio::time::sleep(Duration::from_millis(60)).await;

        assert!(limiter.acquire("test").await.is_allowed());
    }

    #[tokio::test]
    async fn test_rate_limiter_separate_keys() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_tokens: 1,
            refill_interval: Duration::from_secs(60),
            tokens_per_refill: 1,
        });

        assert!(limiter.acquire("key_a").await.is_allowed());
        assert!(!limiter.acquire("key_a").await.is_allowed());
        assert!(limiter.acquire("key_b").await.is_allowed());
    }

    #[tokio::test]
    async fn test_llm_rate_limiter() {
        let limiter = LlmRateLimiter::new();
        assert!(limiter.check_llm_request("mimo").await.is_allowed());
        assert!(limiter.check_tool_execution("bash").await.is_allowed());
    }

    #[tokio::test]
    async fn test_remaining_tokens() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_tokens: 10,
            refill_interval: Duration::from_secs(1),
            tokens_per_refill: 10,
        });

        assert_eq!(limiter.remaining("test").await, 10);
        limiter.acquire("test").await;
        assert_eq!(limiter.remaining("test").await, 9);
    }
}
