//! Token-bucket speed limiter. A rate of zero means unlimited.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Limiter {
    state: Mutex<State>,
}

struct State {
    rate: u64,
    tokens: f64,
    last: Instant,
}

impl Limiter {
    pub fn new(rate: u64) -> Self {
        Self { state: Mutex::new(State { rate, tokens: 0.0, last: Instant::now() }) }
    }

    pub fn set_rate(&self, rate: u64) {
        let mut s = self.state.lock().unwrap();
        if s.rate != rate {
            *s = State { rate, tokens: 0.0, last: Instant::now() };
        }
    }

    fn charge(&self, bytes: usize) -> Duration {
        let mut s = self.state.lock().unwrap();
        if s.rate == 0 {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let rate = s.rate as f64;
        s.tokens = (s.tokens + now.duration_since(s.last).as_secs_f64() * rate).min(rate / 4.0);
        s.last = now;
        s.tokens -= bytes as f64;
        if s.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-s.tokens / rate)
        }
    }

    /// Account for `bytes` and wait until the budget allows them.
    pub async fn acquire(&self, bytes: usize) {
        let wait = self.charge(bytes);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}
