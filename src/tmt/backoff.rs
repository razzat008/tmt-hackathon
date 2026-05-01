use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct GlobalBackoffState {
    inner: Arc<Mutex<Inner>>,
    config: Config,
}

struct Inner {
    resume_at: Option<Instant>,
    streak: u32,
    last_429_at: Option<Instant>,
}

#[derive(Clone)]
pub struct Config {
    pub base_cooldown: Duration,
    pub max_cooldown: Duration,
    pub jitter_factor: f64,
    pub max_streak: u32,
    pub streak_reset_after: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_cooldown: Duration::from_secs_f64(1.0),
            max_cooldown: Duration::from_secs(60),
            jitter_factor: 0.5,
            max_streak: 10,
            streak_reset_after: Duration::from_secs(30),
        }
    }
}

impl GlobalBackoffState {
    pub fn new(config: Config) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                resume_at: None,
                streak: 0,
                last_429_at: None,
            })),
            config,
        }
    }

    pub fn wait_if_needed(&self) {
        loop {
            let maybe_wait = {
                let guard = self.inner.lock().unwrap();
                guard
                    .resume_at
                    .and_then(|t| t.checked_duration_since(Instant::now()))
            };

            match maybe_wait {
                None => return,
                Some(d) if d.is_zero() => return,
                Some(d) => std::thread::sleep(d),
            }
        }
    }

    pub fn signal_rate_limited(&self, retry_after: Option<Duration>) -> Duration {
        let mut guard = self.inner.lock().unwrap();
        let now = Instant::now();

        if let Some(last) = guard.last_429_at {
            if now.duration_since(last) > self.config.streak_reset_after {
                guard.streak = 0;
            }
        }

        guard.streak = (guard.streak + 1).min(self.config.max_streak);
        guard.last_429_at = Some(now);

        let cooldown = self.compute_cooldown(guard.streak, retry_after);

        let new_resume = now + cooldown;
        guard.resume_at = Some(match guard.resume_at {
            Some(existing) if existing > new_resume => existing,
            _ => new_resume,
        });

        cooldown
    }

    pub fn signal_success(&self) {
        let mut guard = self.inner.lock().unwrap();
        let now = Instant::now();

        if let Some(last) = guard.last_429_at {
            if now.duration_since(last) > self.config.streak_reset_after {
                guard.streak = 0;
                guard.last_429_at = None;
            }
        }
    }

    fn compute_cooldown(&self, streak: u32, retry_after: Option<Duration>) -> Duration {
        use rand::Rng;

        let base_secs = match retry_after {
            Some(d) if d > Duration::ZERO => {
                d.as_secs_f64().min(self.config.max_cooldown.as_secs_f64())
            }
            _ => {
                let exp = self.config.base_cooldown.as_secs_f64() * 2_f64.powi((streak as i32) - 1);
                exp.min(self.config.max_cooldown.as_secs_f64())
            }
        };

        let lo = base_secs * (1.0 - self.config.jitter_factor);
        let mut rng = rand::thread_rng();
        let jittered = lo + rng.r#gen::<f64>() * (base_secs - lo);

        Duration::from_secs_f64(jittered.max(0.0))
    }
}

impl Default for GlobalBackoffState {
    fn default() -> Self {
        Self::new(Config::default())
    }
}

#[derive(Clone)]
pub struct AsyncGlobalBackoffState {
    inner: Arc<tokio::sync::Mutex<Inner>>,
    config: Config,
}

impl AsyncGlobalBackoffState {
    pub fn new(config: Config) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(Inner {
                resume_at: None,
                streak: 0,
                last_429_at: None,
            })),
            config,
        }
    }

    pub async fn wait_if_needed(&self) {
        loop {
            let maybe_wait = {
                let guard = self.inner.lock().await;
                guard
                    .resume_at
                    .and_then(|t| t.checked_duration_since(Instant::now()))
            };

            match maybe_wait {
                None => return,
                Some(d) if d.is_zero() => return,
                Some(d) => tokio::time::sleep(d).await,
            }
        }
    }

    pub async fn signal_rate_limited(&self, retry_after: Option<Duration>) -> Duration {
        let mut guard = self.inner.lock().await;
        let now = Instant::now();

        if let Some(last) = guard.last_429_at {
            if now.duration_since(last) > self.config.streak_reset_after {
                guard.streak = 0;
            }
        }

        guard.streak = (guard.streak + 1).min(self.config.max_streak);
        guard.last_429_at = Some(now);

        let cooldown = self.compute_cooldown(guard.streak, retry_after);

        let new_resume = now + cooldown;
        guard.resume_at = Some(match guard.resume_at {
            Some(existing) if existing > new_resume => existing,
            _ => new_resume,
        });

        cooldown
    }

    pub async fn signal_success(&self) {
        let mut guard = self.inner.lock().await;
        let now = Instant::now();

        if let Some(last) = guard.last_429_at {
            if now.duration_since(last) > self.config.streak_reset_after {
                guard.streak = 0;
                guard.last_429_at = None;
            }
        }
    }

    fn compute_cooldown(&self, streak: u32, retry_after: Option<Duration>) -> Duration {
        use rand::Rng;

        let base_secs = match retry_after {
            Some(d) if d > Duration::ZERO => {
                d.as_secs_f64().min(self.config.max_cooldown.as_secs_f64())
            }
            _ => {
                let exp = self.config.base_cooldown.as_secs_f64() * 2_f64.powi((streak as i32) - 1);
                exp.min(self.config.max_cooldown.as_secs_f64())
            }
        };

        let lo = base_secs * (1.0 - self.config.jitter_factor);
        let mut rng = rand::thread_rng();
        let jittered = lo + rng.r#gen::<f64>() * (base_secs - lo);

        Duration::from_secs_f64(jittered.max(0.0))
    }
}

impl Default for AsyncGlobalBackoffState {
    fn default() -> Self {
        Self::new(Config::default())
    }
}

pub fn parse_retry_after(value: Option<&str>) -> Option<Duration> {
    let s = value?.trim();

    if let Ok(secs) = s.parse::<f64>() {
        return if secs > 0.0 {
            Some(Duration::from_secs_f64(secs))
        } else {
            None
        };
    }

    #[cfg(feature = "httpdate")]
    if let Ok(system_time) = httpdate::parse_http_date(s) {
        let now = std::time::SystemTime::now();
        return system_time
            .duration_since(now)
            .ok()
            .filter(|d| !d.is_zero());
    }

    None
}
