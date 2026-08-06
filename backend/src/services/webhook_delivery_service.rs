//! Shared outbound webhook signing and bounded delivery.

use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;
use std::time::Duration;
use tokio::time::sleep;

use crate::errors::{AppError, AppResult};

const MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF_MS: u64 = 1_000;
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const LEGACY_REVOCATION_HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureContract {
    /// Sign `timestamp + "." + body` and send the timestamp in its own header.
    Timestamped,
    /// Compatibility contract used by the existing revocation webhook.
    BodyOnly,
}

#[derive(Clone, Copy)]
struct DeliveryPolicy {
    max_attempts: u32,
    base_backoff: Duration,
    timeout: Duration,
}

impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: MAX_ATTEMPTS,
            base_backoff: Duration::from_millis(BASE_BACKOFF_MS),
            timeout: HTTP_TIMEOUT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryFailure {
    pub attempts: u32,
    pub reason: &'static str,
    pub last_status: Option<u16>,
}

#[allow(clippy::too_many_arguments)]
pub async fn deliver_signed_body(
    http_client: &Client,
    url: &str,
    secret: &[u8],
    event_type: &str,
    event_id: &str,
    body: &[u8],
    signature_contract: SignatureContract,
) -> Result<(), DeliveryFailure> {
    let mut policy = DeliveryPolicy::default();
    if signature_contract == SignatureContract::BodyOnly {
        policy.timeout = LEGACY_REVOCATION_HTTP_TIMEOUT;
    }
    deliver_with_policy(
        http_client,
        url,
        secret,
        event_type,
        event_id,
        body,
        signature_contract,
        policy,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn deliver_with_policy(
    http_client: &Client,
    url: &str,
    secret: &[u8],
    event_type: &str,
    event_id: &str,
    body: &[u8],
    signature_contract: SignatureContract,
    policy: DeliveryPolicy,
) -> Result<(), DeliveryFailure> {
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let signature = match signature_contract {
        SignatureContract::Timestamped => compute_timestamped_signature(secret, &timestamp, body),
        SignatureContract::BodyOnly => compute_body_signature(secret, body),
    };
    let mut last_status = None;
    let mut reason = "send_failed";

    for attempt in 1..=policy.max_attempts {
        let mut request = http_client
            .post(url)
            .timeout(policy.timeout)
            .header("Content-Type", "application/json")
            .header("X-NyxID-Event", event_type)
            .header("X-NyxID-Delivery-Id", event_id)
            .header("X-NyxID-Signature", format!("sha256={signature}"))
            .body(body.to_vec());
        if signature_contract == SignatureContract::Timestamped {
            request = request.header("X-NyxID-Timestamp", &timestamp);
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                last_status = Some(response.status().as_u16());
                reason = "non_success_status";
            }
            Err(_) => {
                last_status = None;
                reason = "send_failed";
            }
        }

        if attempt < policy.max_attempts {
            let multiplier = 4_u32.pow(attempt - 1);
            sleep(policy.base_backoff.saturating_mul(multiplier)).await;
        }
    }

    Err(DeliveryFailure {
        attempts: policy.max_attempts,
        reason,
        last_status,
    })
}

pub fn compute_timestamped_signature(secret: &[u8], timestamp: &str, body: &[u8]) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

pub fn compute_body_signature(secret: &[u8], body: &[u8]) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Webhook targets are fetched by the server, so they must be HTTPS and must
/// resolve entirely to public addresses.
pub async fn validate_webhook_url(url: &str, field_name: &str) -> AppResult<String> {
    let normalized = url.trim();
    let parsed = url::Url::parse(normalized)
        .map_err(|_| AppError::ValidationError(format!("{field_name} must be a valid URL")))?;
    if parsed.scheme() != "https" {
        return Err(AppError::ValidationError(format!(
            "{field_name} must use https"
        )));
    }
    if parsed.fragment().is_some() {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not contain a fragment"
        )));
    }
    crate::services::url_validation::validate_public_http_url(normalized, field_name).await?;
    Ok(normalized.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::StatusCode, routing::post};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn timestamped_signature_matches_fixture() {
        let signature = compute_timestamped_signature(
            b"fixture-secret",
            "1700000000",
            br#"{"event_type":"connect_link.completed"}"#,
        );
        assert_eq!(
            signature,
            "b426d8e45504ab2702700a6ac32e73f9355bcbcfae7ffd6d452b65a50755b617"
        );
    }

    #[tokio::test]
    async fn delivery_retries_then_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let handler_attempts = attempts.clone();
        let app = Router::new().route(
            "/hook",
            post(move || {
                let handler_attempts = handler_attempts.clone();
                async move {
                    if handler_attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::NO_CONTENT
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("test address");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve test app") });

        let result = deliver_with_policy(
            &Client::new(),
            &format!("http://{address}/hook"),
            b"secret",
            "test.event",
            "event-id",
            b"{}",
            SignatureContract::Timestamped,
            DeliveryPolicy {
                max_attempts: 3,
                base_backoff: Duration::from_millis(1),
                timeout: Duration::from_secs(1),
            },
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn webhook_url_requires_https_before_dns_resolution() {
        let error = validate_webhook_url("http://127.0.0.1/hook", "webhook_url")
            .await
            .expect_err("http must be rejected");
        assert!(error.to_string().contains("https"));
    }

    #[tokio::test]
    async fn webhook_url_rejects_private_targets() {
        let error = validate_webhook_url("https://127.0.0.1/hook", "webhook_url")
            .await
            .expect_err("private target must be rejected");
        assert!(error.to_string().contains("private or internal"));
    }
}
