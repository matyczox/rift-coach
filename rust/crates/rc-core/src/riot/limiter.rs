//! Client-side rate limiter for the Riot dev key (20 req / 1 s and 100 req / 120 s),
//! `src/main/riot/limiter.ts`.
//!
//! Each limit is a token bucket whose spent tokens come back exactly `window_ms`
//! after they were spent (a "token return" bucket, equivalent to a sliding log).
//! This guarantees that no window of length `window_ms` — however Riot aligns
//! its own windows — ever sees more than `count` requests. A small `pad_ms`
//! covers clock skew / network latency between us and Riot.
//!
//! Time is `tokio::time::Instant`, so tests can run on a paused clock.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateLimit {
    pub count: usize,
    pub window_ms: f64,
}

pub const DEV_KEY_LIMITS: [RateLimit; 2] = [RateLimit { count: 20, window_ms: 1_000.0 }, RateLimit { count: 100, window_ms: 120_000.0 }];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadlineError;

impl std::fmt::Display for DeadlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Riot request would exceed the deadline")
    }
}
impl std::error::Error for DeadlineError {}

struct State {
    /// Grant times (ms since `origin`), ascending.
    log: VecDeque<f64>,
    paused_until: f64,
}

pub struct RateLimiter {
    limits: Vec<RateLimit>,
    pad_ms: f64,
    max_window: f64,
    origin: Instant,
    state: Mutex<State>,
    /// FIFO turn: tokio's mutex is fair, so waiters are granted in arrival order.
    turn: tokio::sync::Mutex<()>,
}

impl RateLimiter {
    pub fn new(limits: &[RateLimit], pad_ms: f64) -> RateLimiter {
        let max_window = limits.iter().map(|l| l.window_ms).fold(0.0, f64::max);
        RateLimiter { limits: limits.to_vec(), pad_ms, max_window, origin: Instant::now(), state: Mutex::new(State { log: VecDeque::new(), paused_until: 0.0 }), turn: tokio::sync::Mutex::new(()) }
    }

    /// Dev-key limits with the default 50 ms pad.
    pub fn dev_key() -> RateLimiter {
        RateLimiter::new(&DEV_KEY_LIMITS, 50.0)
    }

    fn ms_of(&self, t: Instant) -> f64 {
        if t >= self.origin {
            t.duration_since(self.origin).as_secs_f64() * 1000.0
        } else {
            -(self.origin.duration_since(t).as_secs_f64() * 1000.0)
        }
    }

    fn now(&self) -> f64 {
        self.ms_of(Instant::now())
    }

    fn count_since(log: &VecDeque<f64>, t: f64) -> usize {
        log.iter().rev().take_while(|&&x| x > t).count()
    }

    fn prune(&self, s: &mut State, t: f64) {
        let cut = t - self.max_window - self.pad_ms;
        while s.log.front().is_some_and(|&x| x <= cut) {
            s.log.pop_front();
        }
    }

    fn delay_locked(&self, s: &mut State, t: f64) -> f64 {
        self.prune(s, t);
        let mut wait = (s.paused_until - t).max(0.0);
        for l in &self.limits {
            if Self::count_since(&s.log, t - l.window_ms) >= l.count {
                // The token we need back is the `count`-th most recent grant.
                let blocking = s.log[s.log.len() - l.count];
                wait = wait.max(blocking + l.window_ms + self.pad_ms - t);
            }
        }
        wait
    }

    /// Milliseconds until the next token could be granted (0 = now).
    pub fn delay(&self) -> f64 {
        let t = self.now();
        let mut s = self.state.lock().unwrap();
        self.delay_locked(&mut s, t)
    }

    /// Tokens available right now in the tightest bucket.
    pub fn available(&self) -> usize {
        let t = self.now();
        let mut s = self.state.lock().unwrap();
        self.prune(&mut s, t);
        if s.paused_until > t {
            return 0;
        }
        self.limits.iter().map(|l| l.count.saturating_sub(Self::count_since(&s.log, t - l.window_ms))).min().unwrap_or(usize::MAX)
    }

    /// How many requests the longest-window limit can still grant by time `t`
    /// (counting tokens that return before then). Used to size request budgets.
    pub fn available_by(&self, t: Instant) -> usize {
        let Some(l) = self.limits.iter().copied().reduce(|a, b| if b.window_ms > a.window_ms { b } else { a }) else { return usize::MAX };
        let tm = self.ms_of(t);
        let s = self.state.lock().unwrap();
        l.count.saturating_sub(Self::count_since(&s.log, tm - l.window_ms - self.pad_ms))
    }

    /// Block all requests for `ms` (Retry-After from a 429).
    pub fn pause(&self, ms: f64) {
        let t = self.now();
        let mut s = self.state.lock().unwrap();
        s.paused_until = s.paused_until.max(t + ms.max(0.0));
    }

    /// Wait for a token. Requests are granted FIFO. Fails with [`DeadlineError`]
    /// (without consuming a token) if the wait would end after `deadline`.
    pub async fn acquire(&self, deadline: Option<Instant>) -> Result<(), DeadlineError> {
        let _turn = self.turn.lock().await;
        loop {
            let wait = {
                let t = self.now();
                let mut s = self.state.lock().unwrap();
                let w = self.delay_locked(&mut s, t);
                if w <= 0.0 {
                    s.log.push_back(t);
                    return Ok(());
                }
                w
            };
            let d = Duration::from_secs_f64(wait / 1000.0);
            if let Some(dl) = deadline {
                if Instant::now() + d > dl {
                    return Err(DeadlineError);
                }
            }
            tokio::time::sleep(d).await;
        }
    }

    /// Test/diagnostic hook: pretend `n` requests were granted just now.
    #[doc(hidden)]
    pub fn record_grants(&self, n: usize) {
        let t = self.now();
        let mut s = self.state.lock().unwrap();
        for _ in 0..n {
            s.log.push_back(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn ms_since(t0: Instant) -> u128 {
        Instant::now().duration_since(t0).as_millis()
    }

    #[tokio::test(start_paused = true)]
    async fn grants_20_then_waits_for_the_1s_window() {
        let l = Arc::new(RateLimiter::new(&DEV_KEY_LIMITS, 0.0));
        let t0 = Instant::now();
        let granted = Arc::new(Mutex::new(Vec::new()));
        let mut hs = Vec::new();
        for _ in 0..21 {
            let (l, g) = (l.clone(), granted.clone());
            hs.push(tokio::spawn(async move {
                l.acquire(None).await.unwrap();
                g.lock().unwrap().push(ms_since(t0));
            }));
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(granted.lock().unwrap().len(), 20);
        tokio::time::sleep(Duration::from_millis(997)).await;
        assert_eq!(granted.lock().unwrap().len(), 20);
        for h in hs {
            h.await.unwrap();
        }
        let g = granted.lock().unwrap();
        assert_eq!(g.len(), 21);
        assert!((1000..=1002).contains(&g[20]), "21st at {}", g[20]);
    }

    #[tokio::test(start_paused = true)]
    async fn honours_the_100_per_120s_bucket() {
        let l = Arc::new(RateLimiter::new(&DEV_KEY_LIMITS, 0.0));
        let n = Arc::new(AtomicUsize::new(0));
        let mut hs = Vec::new();
        for _ in 0..101 {
            let (l, n) = (l.clone(), n.clone());
            hs.push(tokio::spawn(async move {
                l.acquire(None).await.unwrap();
                n.fetch_add(1, Ordering::SeqCst);
            }));
        }
        tokio::time::sleep(Duration::from_millis(10_000)).await;
        assert_eq!(n.load(Ordering::SeqCst), 100); // 5 s of 20/s bursts
        tokio::time::sleep(Duration::from_millis(100_000)).await;
        assert_eq!(n.load(Ordering::SeqCst), 100); // still inside 120 s of the first grant
        tokio::time::sleep(Duration::from_millis(10_000)).await;
        for h in hs {
            h.await.unwrap();
        }
        assert_eq!(n.load(Ordering::SeqCst), 101);
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_without_token_and_pause() {
        let l = Arc::new(RateLimiter::new(&[RateLimit { count: 1, window_ms: 1000.0 }], 0.0));
        l.acquire(None).await.unwrap();
        let dl = Instant::now() + Duration::from_millis(500);
        assert_eq!(l.acquire(Some(dl)).await, Err(DeadlineError));
        assert_eq!(l.delay(), 1000.0);
        tokio::time::sleep(Duration::from_millis(1000)).await;
        l.pause(3000.0);
        let ok = Arc::new(AtomicUsize::new(0));
        let (l2, ok2) = (l.clone(), ok.clone());
        let h = tokio::spawn(async move {
            l2.acquire(None).await.unwrap();
            ok2.store(1, Ordering::SeqCst);
        });
        tokio::time::sleep(Duration::from_millis(2999)).await;
        assert_eq!(ok.load(Ordering::SeqCst), 0);
        h.await.unwrap();
        assert_eq!(ok.load(Ordering::SeqCst), 1);
        assert_eq!(l.available(), 0);
    }
}
