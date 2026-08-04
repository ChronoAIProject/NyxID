use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::services::{telegram_poller, telegram_service};

/// POST /api/v1/webhooks/telegram
///
/// Telegram webhook endpoint. Handles:
/// 1. Callback queries (approval decisions from inline keyboard)
/// 2. Messages (link commands like /start NYXID-A1B2C3)
///
/// Verified via X-Telegram-Bot-Api-Secret-Token header.
/// Always returns 200 OK to prevent Telegram retries.
pub async fn telegram_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<telegram_service::TelegramUpdate>,
) -> StatusCode {
    // Verify webhook secret
    let expected_secret = match state.config.telegram_webhook_secret.as_deref() {
        Some(s) => s,
        None => {
            tracing::warn!("Telegram webhook received but no secret configured");
            return StatusCode::OK;
        }
    };

    let received_secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_webhook_secret(received_secret, expected_secret) {
        tracing::warn!("Telegram webhook secret verification failed");
        return StatusCode::OK;
    }

    // Delegate update processing to the shared handler
    telegram_poller::process_update(&state, update).await;

    StatusCode::OK
}

fn verify_webhook_secret(received: &str, expected: &str) -> bool {
    use sha2::{Digest, Sha256};
    // Pre-hash both values so the constant-time comparison always operates
    // on 32-byte digests, preventing length leakage via timing side-channel.
    let h1 = Sha256::digest(received.as_bytes());
    let h2 = Sha256::digest(expected.as_bytes());
    h1.ct_eq(&h2).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_secret_valid() {
        assert!(verify_webhook_secret("mysecret", "mysecret"));
    }

    #[test]
    fn verify_secret_invalid() {
        assert!(!verify_webhook_secret("wrong", "mysecret"));
    }

    #[test]
    fn verify_secret_empty() {
        assert!(!verify_webhook_secret("", "mysecret"));
        assert!(verify_webhook_secret("", ""));
    }
}
