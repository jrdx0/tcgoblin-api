use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::Value;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ATTEMPTS: u32 = 3;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct DiscordClient {
    http: Client,
}

#[derive(Deserialize)]
struct RateLimited {
    retry_after: f64,
}

#[derive(Deserialize)]
struct DiscordError {
    message: String,
}

impl DiscordClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("failed to build Discord HTTP client");
        Self { http }
    }

    pub async fn send(&self, webhook_url: &str, message: &Value) -> Result<(), String> {
        for attempt in 1..=MAX_ATTEMPTS {
            let response = self
                .http
                .post(webhook_url)
                .json(message)
                .send()
                .await
                .map_err(|e| format!("request to Discord failed: {}", e.without_url()))?;

            let status = response.status();
            if status.is_success() {
                return Ok(());
            }

            if status == StatusCode::TOO_MANY_REQUESTS && attempt < MAX_ATTEMPTS {
                let wait = response
                    .json::<RateLimited>()
                    .await
                    .map(|r| Duration::from_secs_f64(r.retry_after.max(0.0)))
                    .unwrap_or(Duration::from_secs(1))
                    .min(MAX_RETRY_WAIT);
                tokio::time::sleep(wait).await;
                continue;
            }

            let detail = response
                .json::<DiscordError>()
                .await
                .map(|e| format!(": {}", e.message))
                .unwrap_or_default();
            return Err(format!("Discord responded with HTTP {status}{detail}"));
        }

        Err("Discord kept rate limiting the webhook".to_string())
    }
}
