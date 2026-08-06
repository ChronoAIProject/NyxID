use std::collections::HashMap;

use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::Request,
};
use serde::Serialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::services::{audit_service, trigger_service};

#[derive(Serialize)]
pub struct TriggerIngressResponse {
    pub status: &'static str,
    pub event_id: String,
}

pub async fn receive_trigger(
    State(state): State<AppState>,
    Path(trigger_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    request: Request<Body>,
) -> AppResult<Json<TriggerIngressResponse>> {
    let trigger = trigger_service::load_active_for_ingress(&state.db, &trigger_id).await?;
    let headers = request.headers().clone();
    let body = to_bytes(request.into_body(), state.config.trigger_payload_max_bytes)
        .await
        .map_err(|_| AppError::TriggerPayloadTooLarge)?;
    trigger_service::verify_ingress(
        &state.encryption_keys,
        &trigger,
        &headers,
        query.get("token").map(String::as_str),
        &body,
    )
    .await?;
    if !state.per_trigger_limiter.check(&trigger.id) {
        return Err(AppError::TriggerRateLimited);
    }
    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::ValidationError("trigger payload must be valid JSON".to_string()))?;
    let event_id = trigger_service::event_id(&headers, &payload, &body)?;
    if state.trigger_dedup_cache.contains(&trigger.id, &event_id) {
        return Ok(Json(TriggerIngressResponse {
            status: "duplicate",
            event_id,
        }));
    }
    trigger_service::deliver_event(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &state.config,
        &state.jwt_keys,
        &state.per_channel_event_limiter,
        &state.event_dedup_cache,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        &trigger,
        &event_id,
        payload,
    )
    .await?;
    state
        .trigger_dedup_cache
        .insert_if_absent(&trigger.id, &event_id);
    audit_service::log_async(
        state.db.clone(),
        Some(trigger.user_id.clone()),
        "trigger_event_forwarded".to_string(),
        Some(serde_json::json!({
            "trigger_id": trigger.id,
            "event_id": event_id,
            "delivery_type": match trigger.delivery {
                crate::models::trigger::TriggerDelivery::Webhook { .. } => "webhook",
                crate::models::trigger::TriggerDelivery::Agent { .. } => "agent",
                crate::models::trigger::TriggerDelivery::Notification => "notification",
            },
        })),
        None,
        None,
        None,
        None,
    );
    Ok(Json(TriggerIngressResponse {
        status: "accepted",
        event_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        http::{HeaderValue, StatusCode},
        routing::post,
    };
    use chrono::Utc;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use crate::crypto::token::hash_token;
    use crate::models::trigger::{
        COLLECTION_NAME as TRIGGERS, Trigger, TriggerDelivery, TriggerStatus, TriggerTokenLocation,
        TriggerVerification,
    };
    use crate::test_utils::{connect_test_database, test_app_config, test_app_state_with_config};

    async fn receiver() -> (String, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let handler_count = count.clone();
        let app = Router::new().route(
            "/hook",
            post(move || {
                let handler_count = handler_count.clone();
                async move {
                    handler_count.fetch_add(1, Ordering::SeqCst);
                    StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind receiver");
        let address = listener.local_addr().expect("receiver address");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve receiver") });
        (format!("http://{address}/hook"), count)
    }

    async fn insert_trigger(
        state: &AppState,
        url: String,
        verification: TriggerVerification,
        raw_secret: &str,
        status: TriggerStatus,
    ) -> Trigger {
        let now = Utc::now();
        let verification_secret_encrypted =
            if matches!(verification, TriggerVerification::HmacSha256 { .. }) {
                Some(
                    state
                        .encryption_keys
                        .encrypt(raw_secret.as_bytes())
                        .await
                        .expect("encrypt verification secret"),
                )
            } else {
                None
            };
        let trigger = Trigger {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            label: "Inbound build event".to_string(),
            user_service_id: None,
            status,
            secret_hash: hash_token(raw_secret),
            verification,
            verification_secret_encrypted,
            delivery: TriggerDelivery::Webhook { url },
            delivery_secret_encrypted: Some(
                state
                    .encryption_keys
                    .encrypt(b"delivery-secret")
                    .await
                    .expect("encrypt delivery secret"),
            ),
            created_at: now,
            updated_at: now,
        };
        state
            .db
            .collection::<Trigger>(TRIGGERS)
            .insert_one(&trigger)
            .await
            .expect("insert trigger");
        trigger
    }

    fn request(body: &'static str, bearer: Option<&str>) -> Request<Body> {
        let mut request = Request::builder().method("POST").uri("/");
        if let Some(secret) = bearer {
            request = request.header("Authorization", format!("Bearer {secret}"));
        }
        request.body(Body::from(body)).expect("request")
    }

    #[tokio::test]
    async fn valid_token_delivers_once_and_replay_is_deduplicated() {
        let Some(db) = connect_test_database("trigger_ingress_dedup").await else {
            return;
        };
        let state = test_app_state_with_config(db, test_app_config());
        let (url, count) = receiver().await;
        let trigger = insert_trigger(
            &state,
            url,
            TriggerVerification::Token {
                location: TriggerTokenLocation::Bearer,
            },
            "nyx_trg_valid",
            TriggerStatus::Active,
        )
        .await;
        let invoke = || {
            receive_trigger(
                State(state.clone()),
                Path(trigger.id.clone()),
                Query(HashMap::new()),
                request(
                    r#"{"event_id":"delivery-1","action":"completed"}"#,
                    Some("nyx_trg_valid"),
                ),
            )
        };
        let Json(first) = invoke().await.expect("first delivery");
        let Json(second) = invoke().await.expect("deduplicated delivery");
        assert_eq!(first.status, "accepted");
        assert_eq!(second.status, "duplicate");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wrong_token_is_rejected_without_delivery() {
        let Some(db) = connect_test_database("trigger_ingress_wrong_token").await else {
            return;
        };
        let state = test_app_state_with_config(db, test_app_config());
        let (url, count) = receiver().await;
        let trigger = insert_trigger(
            &state,
            url,
            TriggerVerification::Token {
                location: TriggerTokenLocation::Bearer,
            },
            "nyx_trg_valid",
            TriggerStatus::Active,
        )
        .await;
        let result = receive_trigger(
            State(state),
            Path(trigger.id),
            Query(HashMap::new()),
            request(r#"{"action":"completed"}"#, Some("wrong")),
        )
        .await;
        assert!(matches!(result, Err(AppError::TriggerSecretInvalid)));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn hmac_mode_verifies_raw_body() {
        let Some(db) = connect_test_database("trigger_ingress_hmac").await else {
            return;
        };
        let state = test_app_state_with_config(db, test_app_config());
        let (url, count) = receiver().await;
        let raw_secret = "nyx_trg_hmac";
        let trigger = insert_trigger(
            &state,
            url,
            TriggerVerification::HmacSha256 {
                header_name: "X-Hub-Signature-256".to_string(),
            },
            raw_secret,
            TriggerStatus::Active,
        )
        .await;
        let body = r#"{"action":"opened"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(raw_secret.as_bytes()).expect("HMAC key");
        mac.update(body.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        let mut signed = request(body, None);
        signed.headers_mut().insert(
            "X-Hub-Signature-256",
            HeaderValue::from_str(&format!("sha256={signature}")).expect("signature"),
        );
        let Json(result) = receive_trigger(
            State(state),
            Path(trigger.id),
            Query(HashMap::new()),
            signed,
        )
        .await
        .expect("HMAC delivery");
        assert_eq!(result.status, "accepted");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn query_token_mode_accepts_valid_secret() {
        let Some(db) = connect_test_database("trigger_ingress_query_token").await else {
            return;
        };
        let state = test_app_state_with_config(db, test_app_config());
        let (url, count) = receiver().await;
        let trigger = insert_trigger(
            &state,
            url,
            TriggerVerification::Token {
                location: TriggerTokenLocation::Query,
            },
            "nyx_trg_query",
            TriggerStatus::Active,
        )
        .await;
        let Json(result) = receive_trigger(
            State(state),
            Path(trigger.id),
            Query(HashMap::from([(
                "token".to_string(),
                "nyx_trg_query".to_string(),
            )])),
            request(r#"{"action":"opened"}"#, None),
        )
        .await
        .expect("query-token delivery");
        assert_eq!(result.status, "accepted");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn disabled_trigger_is_not_found_shaped() {
        let Some(db) = connect_test_database("trigger_ingress_disabled").await else {
            return;
        };
        let state = test_app_state_with_config(db, test_app_config());
        let (url, _) = receiver().await;
        let trigger = insert_trigger(
            &state,
            url,
            TriggerVerification::Token {
                location: TriggerTokenLocation::Bearer,
            },
            "nyx_trg_valid",
            TriggerStatus::Disabled,
        )
        .await;
        let result = receive_trigger(
            State(state),
            Path(trigger.id),
            Query(HashMap::new()),
            request("{}", Some("nyx_trg_valid")),
        )
        .await;
        assert!(matches!(result, Err(AppError::TriggerNotFound)));
    }

    #[tokio::test]
    async fn payload_size_cap_is_enforced() {
        let Some(db) = connect_test_database("trigger_ingress_size").await else {
            return;
        };
        let mut config = test_app_config();
        config.trigger_payload_max_bytes = 4;
        let state = test_app_state_with_config(db, config);
        let (url, _) = receiver().await;
        let trigger = insert_trigger(
            &state,
            url,
            TriggerVerification::Token {
                location: TriggerTokenLocation::Bearer,
            },
            "nyx_trg_valid",
            TriggerStatus::Active,
        )
        .await;
        let result = receive_trigger(
            State(state),
            Path(trigger.id),
            Query(HashMap::new()),
            request(r#"{"too":"large"}"#, Some("nyx_trg_valid")),
        )
        .await;
        assert!(matches!(result, Err(AppError::TriggerPayloadTooLarge)));
    }
}
