use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore, oneshot};
use tracing::{debug, warn};

use crate::{
    config::{MAX_REQUEST_TEXT_BYTES, RuntimeConfig},
    error::AppError,
    tmt::{AsyncGlobalBackoffState, TmtClient, TmtError},
    translate::split_sentences,
};

#[derive(Clone)]
pub struct TranslationService {
    client: TmtClient,
    semaphore: Arc<Semaphore>,
    rate_limit_ms: Option<u64>,
    max_retries: u32,
    cache: Arc<Mutex<HashMap<CacheKey, String>>>,
    inflight: Arc<Mutex<HashMap<CacheKey, Vec<oneshot::Sender<InFlightResult>>>>>,
    backoff: AsyncGlobalBackoffState,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    sentence: String,
    src_lang: String,
    tgt_lang: String,
}

type InFlightResult = Result<String, String>;

impl TranslationService {
    pub fn new(client: TmtClient, config: &RuntimeConfig) -> Result<Self, AppError> {
        if config.concurrency == 0 {
            return Err(AppError::InvalidArgument {
                message: "concurrency must be >= 1".to_string(),
            });
        }

        Ok(Self {
            client,
            semaphore: Arc::new(Semaphore::new(config.concurrency)),
            rate_limit_ms: config.rate_limit_ms,
            max_retries: config.max_retries,
            cache: Arc::new(Mutex::new(HashMap::new())),
            inflight: Arc::new(Mutex::new(HashMap::new())),
            backoff: AsyncGlobalBackoffState::default(),
        })
    }

    // In translate_text, instead of spawning all sentences concurrently:
    pub async fn translate_text(
        &self,
        text: &str,
        src_lang: &str,
        tgt_lang: &str,
    ) -> Result<String, AppError> {
        let sentences = split_sentences(text);
        if sentences.is_empty() {
            return Ok(text.to_string());
        }

        let mut translated = Vec::with_capacity(sentences.len());
        for sentence in &sentences {
            // Sequential — each sentence waits for the previous to complete
            let output = self
                .translate_sentence_cached(sentence, src_lang, tgt_lang)
                .await?;
            translated.push(output);
        }
        Ok(translated.join(" "))
    }

    async fn translate_sentence_cached(
        &self,
        sentence: &str,
        src_lang: &str,
        tgt_lang: &str,
    ) -> Result<String, AppError> {
        let key = CacheKey {
            sentence: sentence.to_string(),
            src_lang: src_lang.to_string(),
            tgt_lang: tgt_lang.to_string(),
        };

        if let Some(hit) = self.cache.lock().await.get(&key).cloned() {
            return Ok(hit);
        }

        let receiver = {
            let mut inflight = self.inflight.lock().await;
            if let Some(waiters) = inflight.get_mut(&key) {
                let (tx, rx) = oneshot::channel();
                waiters.push(tx);
                Some(rx)
            } else {
                inflight.insert(key.clone(), Vec::new());
                None
            }
        };

        if let Some(rx) = receiver {
            return match rx.await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(message)) => Err(AppError::TranslationFailed { message }),
                Err(_) => Err(AppError::TranslationFailed {
                    message: "inflight channel closed".to_string(),
                }),
            };
        }

        let result = self
            .translate_sentence_uncached(sentence, src_lang, tgt_lang)
            .await;

        let inflight_result = result
            .as_ref()
            .map(|value| value.clone())
            .map_err(|err| err.to_string());

        let waiters = {
            let mut inflight = self.inflight.lock().await;
            inflight.remove(&key).unwrap_or_default()
        };

        for waiter in waiters {
            let _ = waiter.send(inflight_result.clone());
        }

        if let Ok(value) = &result {
            let mut cache = self.cache.lock().await;
            cache.insert(key, value.clone());
        }

        result
    }

    async fn translate_sentence_uncached(
        &self,
        sentence: &str,
        src_lang: &str,
        tgt_lang: &str,
    ) -> Result<String, AppError> {
        let len = sentence.as_bytes().len();
        if len > MAX_REQUEST_TEXT_BYTES {
            warn!(
                len,
                max = MAX_REQUEST_TEXT_BYTES,
                "sentence exceeds request size limit"
            );
            return Err(AppError::RequestTooLarge {
                len,
                max: MAX_REQUEST_TEXT_BYTES,
            });
        }

        for attempt in 1..=self.max_retries {
            // Acquire permit first — gates how many tasks even reach the API
            let _permit =
                self.semaphore
                    .acquire()
                    .await
                    .map_err(|_| AppError::TranslationFailed {
                        message: "translation semaphore closed".to_string(),
                    })?;

            self.backoff.wait_if_needed().await;

            // Mandatory inter-request spacing regardless of config
            tokio::time::sleep(Duration::from_millis(200)).await;

            let response = self
                .client
                .translate_sentence(sentence, src_lang, tgt_lang)
                .await;

            match response {
                Ok(output) => {
                    self.backoff.signal_success().await;
                    self.apply_rate_limit().await;
                    return Ok(output);
                }
                Err(TmtError::RateLimited { retry_after }) => {
                    warn!(attempt, retry_after = ?retry_after, "rate limited by API");
                    self.backoff.signal_rate_limited(retry_after).await;
                    if attempt == self.max_retries {
                        return Err(AppError::RateLimitExceeded {
                            attempts: self.max_retries,
                        });
                    }
                }
                Err(TmtError::Transport(message)) => {
                    return Err(AppError::Network { message });
                }
                Err(err) => {
                    return Err(AppError::TranslationFailed {
                        message: err.to_string(),
                    });
                }
            }
        }

        Err(AppError::RateLimitExceeded {
            attempts: self.max_retries,
        })
    }

    async fn apply_rate_limit(&self) {
        if let Some(ms) = self.rate_limit_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }
}
