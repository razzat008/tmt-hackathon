use std::collections::HashMap;
use std::sync::Arc;

use futures::future::join_all;
use tokio::sync::{Mutex, Semaphore, oneshot};

use crate::{
    config::{MAX_REQUEST_TEXT_BYTES, RuntimeConfig},
    error::AppError,
    tmt::{TmtClient, TmtError},
    translate::split_sentences,
};

#[derive(Clone)]
pub struct TranslationService {
    client: TmtClient,
    semaphore: Arc<Semaphore>,
    cache: Arc<Mutex<HashMap<CacheKey, String>>>,
    inflight: Arc<Mutex<HashMap<CacheKey, Vec<oneshot::Sender<InFlightResult>>>>>,
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
            cache: Arc::new(Mutex::new(HashMap::new())),
            inflight: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Split text into sentences and translate them concurrently.
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

        let futures: Vec<_> = sentences
            .iter()
            .map(|s| self.translate_sentence_cached(s, src_lang, tgt_lang))
            .collect();

        let results = join_all(futures).await;

        let mut translated = Vec::with_capacity(results.len());
        for result in results {
            translated.push(result?);
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

        let inflight_result = result.as_ref().map(|v| v.clone()).map_err(|e| e.to_string());

        let waiters = {
            let mut inflight = self.inflight.lock().await;
            inflight.remove(&key).unwrap_or_default()
        };
        for waiter in waiters {
            let _ = waiter.send(inflight_result.clone());
        }

        if let Ok(value) = &result {
            self.cache.lock().await.insert(key, value.clone());
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
            return Err(AppError::RequestTooLarge {
                len,
                max: MAX_REQUEST_TEXT_BYTES,
            });
        }

        // Semaphore bounds how many API calls are in-flight simultaneously.
        // All retry/backoff/rate-gate logic lives inside TmtClient.
        let _permit =
            self.semaphore
                .acquire()
                .await
                .map_err(|_| AppError::TranslationFailed {
                    message: "semaphore closed".to_string(),
                })?;

        self.client
            .translate_sentence(sentence, src_lang, tgt_lang)
            .await
            .map_err(|err| match err {
                TmtError::Transport(msg) => AppError::Network { message: msg },
                err => AppError::TranslationFailed {
                    message: err.to_string(),
                },
            })
    }
}
