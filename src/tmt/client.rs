use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::tmt::backoff::{AsyncGlobalBackoffState, Config as BackoffConfig, parse_retry_after};

const MAX_RATE_LIMIT_ATTEMPTS: u32 = 100;

/// Global monotonically increasing request ID, mirroring Python's `_request_counter`.
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct TmtClient {
    base_url: String,
    token: String,
    http: Client,
    max_retries: u32,
    max_rate_limit_wait_s: f64,
    base_rate_limit_secs: f64,
    // shared across all clones — prevents concurrent burst past the rate gate
    request_gate: Arc<Mutex<Instant>>,
    backoff: AsyncGlobalBackoffState,
}

impl TmtClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self::with_config(base_url, token, None, 4, 300)
    }

    pub fn with_config(
        base_url: &str,
        token: &str,
        rate_limit_ms: Option<u64>,
        max_retries: u32,
        max_rate_limit_wait_s: u64,
    ) -> Self {
        let base_rate_limit_secs = rate_limit_ms.unwrap_or(0) as f64 / 1000.0;
        let backoff = AsyncGlobalBackoffState::new(BackoffConfig {
            // mirrors Python: base_cooldown = max(1.0, base_rate_limit_seconds)
            base_cooldown: Duration::from_secs_f64(base_rate_limit_secs.max(1.0)),
            max_cooldown: Duration::from_secs(60),
            jitter_factor: 0.5,
            max_streak: 10,
            streak_reset_after: Duration::from_secs(30),
        });
        Self {
            base_url: base_url.to_string(),
            token: token.to_string(),
            http: Client::new(),
            max_retries,
            max_rate_limit_wait_s: max_rate_limit_wait_s as f64,
            base_rate_limit_secs,
            request_gate: Arc::new(Mutex::new(Instant::now())),
            backoff,
        }
    }

    /// Enforces minimum inter-request spacing within this client.
    /// Mirrors Python's `_wait_for_base_rate`.
    async fn wait_for_base_rate(&self) {
        if self.base_rate_limit_secs <= 0.0 {
            return;
        }
        loop {
            let wait = {
                let mut gate = self.request_gate.lock().await;
                let now = Instant::now();
                if now >= *gate {
                    *gate = now + Duration::from_secs_f64(self.base_rate_limit_secs);
                    return;
                }
                *gate - now
            };
            // lock released before sleeping
            tokio::time::sleep(wait).await;
        }
    }

    /// Translate one sentence, retrying on 429s and transient failures.
    ///
    /// 429s are bounded by `max_rate_limit_wait_s` budget and `MAX_RATE_LIMIT_ATTEMPTS`.
    /// Non-429 failures are bounded by `max_retries` with linear sleep between attempts.
    pub async fn translate_sentence(
        &self,
        text: &str,
        src_lang: &str,
        tgt_lang: &str,
    ) -> Result<String, TmtError> {
        let req_id = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let in_bytes = text.as_bytes().len();
        let payload = TmtRequest { text, src_lang, tgt_lang };

        let mut total_rate_limit_wait = 0.0_f64;
        let mut rate_limit_attempts: u32 = 0;
        let mut non_rate_limit_failures: u32 = 0;
        let mut call_attempt: u32 = 0;
        let mut last_error: Option<TmtError> = None;

        loop {
            call_attempt += 1;

            self.backoff.wait_if_needed().await;
            self.wait_for_base_rate().await;

            debug!(
                req_id,
                attempt = call_attempt,
                src = src_lang,
                tgt = tgt_lang,
                in_bytes,
                input = text,
                "sending request"
            );

            let result = self
                .http
                .post(&self.base_url)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", self.token))
                .json(&payload)
                .send()
                .await;

            match result {
                Err(err) => {
                    non_rate_limit_failures += 1;
                    warn!(
                        req_id,
                        attempt = call_attempt,
                        failures = non_rate_limit_failures,
                        error = %err,
                        input = text,
                        "transport error"
                    );
                    last_error = Some(TmtError::Transport(err.to_string()));
                }

                Ok(resp) => {
                    let status = resp.status();

                    if status == StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = resp
                            .headers()
                            .get(header::RETRY_AFTER)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| parse_retry_after(Some(v)));

                        rate_limit_attempts += 1;
                        let cooldown = self.backoff.signal_rate_limited(retry_after).await;
                        let cooldown_s = cooldown.as_secs_f64();
                        total_rate_limit_wait += cooldown_s;

                        warn!(
                            req_id,
                            attempt = call_attempt,
                            rate_limit_attempt = rate_limit_attempts,
                            cooldown_s = format!("{cooldown_s:.1}"),
                            total_wait_s = format!("{total_rate_limit_wait:.1}"),
                            "rate limited (429) — backing off"
                        );

                        if self.max_rate_limit_wait_s > 0.0
                            && total_rate_limit_wait > self.max_rate_limit_wait_s
                        {
                            return Err(TmtError::RateLimitBudgetExceeded {
                                total_wait_s: total_rate_limit_wait,
                                max_s: self.max_rate_limit_wait_s,
                            });
                        }
                        if rate_limit_attempts >= MAX_RATE_LIMIT_ATTEMPTS {
                            return Err(TmtError::RateLimitAttemptsExceeded {
                                attempts: rate_limit_attempts,
                            });
                        }
                        continue;
                    }

                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        non_rate_limit_failures += 1;
                        warn!(
                            req_id,
                            attempt = call_attempt,
                            failures = non_rate_limit_failures,
                            status = status.as_u16(),
                            input = text,
                            "http error"
                        );
                        last_error = Some(TmtError::Http(status.as_u16(), body));
                    } else {
                        match resp.json::<TmtResponse>().await {
                            Err(err) => {
                                non_rate_limit_failures += 1;
                                warn!(
                                    req_id,
                                    attempt = call_attempt,
                                    failures = non_rate_limit_failures,
                                    error = %err,
                                    input = text,
                                    "failed to parse response"
                                );
                                last_error = Some(TmtError::InvalidResponse(err.to_string()));
                            }

                            Ok(api) if api.message_type == "SUCCESS" => {
                                self.backoff.signal_success().await;
                                let out_chars = api.output.chars().count();
                                info!(
                                    req_id,
                                    attempt = call_attempt,
                                    src = src_lang,
                                    tgt = tgt_lang,
                                    in_bytes,
                                    out_chars,
                                    "translated"
                                );
                                debug!(
                                    req_id,
                                    input = text,
                                    output = %api.output,
                                    "translation detail"
                                );
                                return Ok(api.output);
                            }

                            // API returns 200 but body signals rate limiting
                            Ok(api) if api.message.to_lowercase().contains("rate limit") => {
                                rate_limit_attempts += 1;
                                let cooldown = self.backoff.signal_rate_limited(None).await;
                                let cooldown_s = cooldown.as_secs_f64();
                                total_rate_limit_wait += cooldown_s;

                                warn!(
                                    req_id,
                                    attempt = call_attempt,
                                    rate_limit_attempt = rate_limit_attempts,
                                    cooldown_s = format!("{cooldown_s:.1}"),
                                    total_wait_s = format!("{total_rate_limit_wait:.1}"),
                                    api_message = %api.message,
                                    "rate limit in response body — backing off"
                                );

                                if self.max_rate_limit_wait_s > 0.0
                                    && total_rate_limit_wait > self.max_rate_limit_wait_s
                                {
                                    return Err(TmtError::RateLimitBudgetExceeded {
                                        total_wait_s: total_rate_limit_wait,
                                        max_s: self.max_rate_limit_wait_s,
                                    });
                                }
                                if rate_limit_attempts >= MAX_RATE_LIMIT_ATTEMPTS {
                                    return Err(TmtError::RateLimitAttemptsExceeded {
                                        attempts: rate_limit_attempts,
                                    });
                                }
                                continue;
                            }

                            Ok(api) => {
                                non_rate_limit_failures += 1;
                                warn!(
                                    req_id,
                                    attempt = call_attempt,
                                    failures = non_rate_limit_failures,
                                    api_message = %api.message,
                                    input = text,
                                    "api returned failure"
                                );
                                last_error = Some(TmtError::ApiFailure(api.message));
                            }
                        }
                    }
                }
            }

            // non-429 failure: break if retry budget exhausted, else sleep and retry
            if non_rate_limit_failures > self.max_retries {
                break;
            }
            let sleep_s = (0.75 * non_rate_limit_failures as f64).min(3.0);
            debug!(
                req_id,
                attempt = call_attempt,
                sleep_s = format!("{sleep_s:.2}"),
                "retrying after error"
            );
            tokio::time::sleep(Duration::from_secs_f64(sleep_s)).await;
        }

        Err(last_error.unwrap_or(TmtError::ApiFailure("no response".to_string())))
    }
}

#[derive(Debug, Serialize)]
struct TmtRequest<'a> {
    text: &'a str,
    src_lang: &'a str,
    tgt_lang: &'a str,
}

#[derive(Debug, Deserialize)]
struct TmtResponse {
    message_type: String,
    message: String,
    output: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TmtError {
    #[error("transport error: {0}")]
    Transport(String),

    #[error("http error {0}: {1}")]
    Http(u16, String),

    #[error("api failure: {0}")]
    ApiFailure(String),

    #[error("invalid response: {0}")]
    InvalidResponse(String),

    #[error("rate limit wait budget exceeded: {total_wait_s:.1}s > {max_s:.1}s")]
    RateLimitBudgetExceeded { total_wait_s: f64, max_s: f64 },

    #[error("rate limit attempts exhausted after {attempts} retries")]
    RateLimitAttemptsExceeded { attempts: u32 },
}
