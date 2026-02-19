use std::time::Duration;

/// Configurable retry policy with backoff.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff: Backoff,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub jitter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Backoff {
    None,
    Linear,
    Exponential,
    Fibonacci,
}

impl RetryPolicy {
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            backoff: Backoff::None,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            jitter: false,
        }
    }

    pub fn exponential(max_attempts: u32, base_delay: Duration) -> Self {
        Self {
            max_attempts,
            backoff: Backoff::Exponential,
            base_delay,
            max_delay: Duration::from_secs(60),
            jitter: false,
        }
    }

    pub fn with_jitter(mut self) -> Self {
        self.jitter = true;
        self
    }

    pub fn with_max_delay(mut self, max: Duration) -> Self {
        self.max_delay = max;
        self
    }

    /// Delay before the given 1-indexed attempt. Attempt 1 returns zero.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt <= 1 || self.backoff == Backoff::None {
            return Duration::ZERO;
        }
        let n = attempt - 1;
        let base_ms = self.base_delay.as_millis() as f64;
        let delay_ms = match self.backoff {
            Backoff::None => 0.0,
            Backoff::Linear => base_ms * n as f64,
            Backoff::Exponential => base_ms * 2f64.powi(n as i32 - 1),
            Backoff::Fibonacci => base_ms * fib(n) as f64,
        };
        let delay_ms = delay_ms.min(self.max_delay.as_millis() as f64);
        let delay_ms = if self.jitter {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            attempt.hash(&mut h);
            let r = (h.finish() % 1000) as f64 / 1000.0; // 0..1
            delay_ms * (0.5 + r)
        } else {
            delay_ms
        };
        Duration::from_millis(delay_ms as u64)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::none()
    }
}

fn fib(n: u32) -> u64 {
    let (mut a, mut b) = (0u64, 1u64);
    for _ in 0..n {
        let t = a + b;
        a = b;
        b = t;
    }
    a
}
