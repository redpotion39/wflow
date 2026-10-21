//! Shared HTTP client and retry policy for the cloud STT / update endpoints.
//!
//! A single pooled `reqwest::Client` is reused for the whole process so TLS
//! sessions and connections survive between dictations.

use anyhow::{Context, Result};
use reqwest::{RequestBuilder, Response, StatusCode};
use std::sync::LazyLock;
use std::time::Duration;

pub const USER_AGENT: &str = concat!("Wflow/", env!("CARGO_PKG_VERSION"));

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .unwrap_or_default()
});

/// Returns a cheap clone of the shared client.
pub fn client() -> reqwest::Client {
    CLIENT.clone()
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Sends a request, retrying transient failures with exponential backoff.
///
/// `build` is invoked once per attempt because request bodies (multipart / JSON)
/// are consumed when the request is sent.
pub async fn send_with_retry<F>(label: &str, attempts: usize, mut build: F) -> Result<Response>
where
    F: FnMut() -> RequestBuilder,
{
    let attempts = attempts.max(1);
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..attempts {
        if attempt > 0 {
            let backoff = 250u64 * (1u64 << (attempt - 1).min(3));
            tokio::time::sleep(Duration::from_millis(backoff)).await;
        }

        match build().send().await {
            Ok(response) => {
                if is_retryable_status(response.status()) && attempt + 1 < attempts {
                    tracing::warn!(
                        "{} returned HTTP {} (attempt {}/{}), retrying",
                        label,
                        response.status(),
                        attempt + 1,
                        attempts
                    );
                    last_error = Some(anyhow::anyhow!("HTTP {}", response.status()));
                    continue;
                }
                return Ok(response);
            }
            Err(err) => {
                let retryable = err.is_timeout() || err.is_connect();
                if retryable && attempt + 1 < attempts {
                    tracing::warn!(
                        "{} request failed ({}), retrying ({}/{})",
                        label,
                        err,
                        attempt + 1,
                        attempts
                    );
                    last_error = Some(err.into());
                    continue;
                }
                return Err(err).with_context(|| format!("{} request failed", label));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("{} request failed", label)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_statuses() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::OK));
    }
}
