use std::time::Duration;

use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};

use crate::tmt::backoff::parse_retry_after;

#[derive(Debug, Clone)]
pub struct TmtClient {
    base_url: String,
    token: String,
    http: Client,
}

impl TmtClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            token: token.to_string(),
            http: Client::new(),
        }
    }

    pub async fn translate_sentence(
        &self,
        text: &str,
        src_lang: &str,
        tgt_lang: &str,
    ) -> Result<String, TmtError> {
        let payload = TmtRequest {
            text,
            src_lang,
            tgt_lang,
        };

        let response = self
            .http
            .post(&self.base_url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token))
            .json(&payload)
            .send()
            .await
            .map_err(|err| TmtError::Transport(err.to_string()))?;

        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| parse_retry_after(Some(value)));
            return Err(TmtError::RateLimited { retry_after });
        }

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(TmtError::Http(status.as_u16(), body));
        }

        let api = response
            .json::<TmtResponse>()
            .await
            .map_err(|err| TmtError::InvalidResponse(err.to_string()))?;

        if api.message_type != "SUCCESS" {
            return Err(TmtError::ApiFailure(api.message));
        }

        Ok(api.output)
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
    #[error("rate limited")]
    RateLimited { retry_after: Option<Duration> },

    #[error("transport error: {0}")]
    Transport(String),

    #[error("http error {0}: {1}")]
    Http(u16, String),

    #[error("api failure: {0}")]
    ApiFailure(String),

    #[error("invalid response: {0}")]
    InvalidResponse(String),
}
