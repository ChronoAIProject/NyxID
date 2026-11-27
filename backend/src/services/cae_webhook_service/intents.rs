use std::time::Duration;

use bson::doc;
use mongodb::Database;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        oauth_broker_binding::{COLLECTION_NAME as BINDINGS, OauthBrokerBinding},
        oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient},
        security_effect_intent::{SecurityEffect, SecurityEffectIntent},
    },
    services::{
        security_effect_service::{self, Outcome},
        webhook_delivery_service,
    },
};

// Below the relay's 60-second claim. Once sending, expiry becomes uncertain,
// never a second dispatch; stopping this future does not make a send retryable.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub(in crate::services) async fn deliver_intent(
    db: &Database,
    keys: &EncryptionKeys,
    intent: &SecurityEffectIntent,
) -> AppResult<Outcome> {
    let SecurityEffect::Cae {
        binding_id,
        client_id,
        revoke_source,
    } = &intent.effect
    else {
        return Ok(Outcome::Failed("invalid_effect_kind"));
    };
    let Some(binding) = db
        .collection::<OauthBrokerBinding>(BINDINGS)
        .find_one(doc! {"_id": binding_id, "client_id": client_id, "revoked": true})
        .await?
    else {
        return Ok(Outcome::Failed("revoked_binding_missing"));
    };
    let Some(revoked_at) = binding.revoked_at else {
        return Ok(Outcome::Failed("revocation_time_missing"));
    };
    let Some(client) = db
        .collection::<OauthClient>(CLIENTS)
        .find_one(doc! {"_id": client_id})
        .await?
    else {
        return Ok(Outcome::Failed("client_missing"));
    };
    let Some(url) = client
        .revocation_webhook_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
    else {
        return Ok(Outcome::Discard);
    };
    let Some(encrypted) = client.revocation_webhook_secret_encrypted.as_ref() else {
        return Ok(Outcome::Failed("signing_key_missing"));
    };
    let secret = match crate::services::oauth_client_webhook_record::open(
        keys,
        &client,
        crate::services::oauth_client_webhook_record::REVOCATION,
        encrypted,
    )
    .await
    {
        Ok(secret) if !secret.is_empty() && std::str::from_utf8(&secret).is_ok() => secret,
        Ok(_) => return Ok(Outcome::Failed("signing_key_invalid")),
        Err(_) => return Ok(Outcome::Retry("signing_key_unavailable")),
    };
    let event = super::RevocationEvent::new_at(
        binding.id,
        client_id.clone(),
        revoke_source,
        binding.revoke_reason,
        revoked_at,
    );
    let body = match serde_json::to_vec(&event) {
        Ok(body) => body,
        Err(_) => return Ok(Outcome::Failed("event_encoding_failed")),
    };
    let signature = webhook_delivery_service::compute_body_signature(&secret, &body);
    let (url, http) = match super::super::proxy_destination::http::signed_webhook(url) {
        Ok(target) => target,
        Err(AppError::ValidationError(_)) => {
            return Ok(Outcome::Failed(
                webhook_delivery_service::DESTINATION_REFUSED,
            ));
        }
        Err(_) => return Ok(Outcome::Retry("destination_unavailable")),
    };
    if !security_effect_service::begin_send(db, intent).await? {
        return Ok(Outcome::LostClaim);
    }
    // The legacy body and HMAC are unchanged. The persisted occurrence UUID is
    // stable, but the receiver contract does not promise deduplication on it.
    let result = tokio::time::timeout(
        SEND_TIMEOUT,
        http.post(url)
            .timeout(SEND_TIMEOUT)
            .header("Content-Type", "application/json")
            .header("X-NyxID-Event", event.event_type)
            .header("X-NyxID-Delivery-Id", &intent.id)
            .header("X-NyxID-Signature", format!("sha256={signature}"))
            .body(body)
            .send(),
    )
    .await;
    Ok(match result {
        Ok(Ok(response)) if response.status().is_success() => Outcome::Delivered,
        Ok(Ok(response)) if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
            Outcome::Retry("receiver_rate_limited")
        }
        Ok(Ok(response)) if response.status().is_server_error() => {
            Outcome::Uncertain("receiver_server_error")
        }
        Ok(Ok(response)) if response.status().is_redirection() => {
            Outcome::Failed(webhook_delivery_service::DESTINATION_REFUSED)
        }
        Ok(Ok(_)) => Outcome::Failed("receiver_rejected"),
        Ok(Err(error)) if super::super::proxy_destination::http::denied_address(&error) => {
            Outcome::Failed(webhook_delivery_service::DESTINATION_REFUSED)
        }
        Ok(Err(error)) if error.is_connect() => Outcome::Retry("connection_failed"),
        Ok(Err(_)) | Err(_) => Outcome::Uncertain("dispatch_response_missing"),
    })
}
