use axum::{Json, extract::State};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::models::notification_channel::NotificationChannel;
use crate::mw::auth::AuthUser;
use crate::services::approval_policy::GlobalApprovalMode;
use crate::services::notification_service;
use crate::services::notification_service::settings::{self, Readiness, SettingsPatch};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct NotificationSettingsResponse {
    pub id: String,
    pub telegram_connected: bool,
    pub telegram_link_pending: bool,
    pub telegram_username: Option<String>,
    pub telegram_enabled: bool,
    /// Compatibility flag: some operations need approval. Use global_approval_mode for the scope.
    pub approval_required: bool,
    pub global_approval_mode: GlobalApprovalMode,
    /// Retained for older clients; required approval is never suspended by delivery.
    pub approval_suspended: bool,
    pub approval_delivery_configured: bool,
    pub approval_web_available: bool,
    pub approval_timeout_secs: u32,
    pub grant_expiry_days: u32,
    pub push_enabled: bool,
    pub push_device_count: usize,
    pub updated_at: String,
}

/// Assistant postcondition projection for notification settings.  The detail
/// response may contain an upstream Telegram username; the projection omits
/// it and retains only booleans, counters, the binding id, and a timestamp.
#[derive(Debug, Serialize)]
pub struct NotificationSettingsAuthorizationEvidenceResponse {
    pub id: String,
    pub telegram_connected: bool,
    pub telegram_link_pending: bool,
    pub telegram_enabled: bool,
    /// Compatibility flag: some operations need approval. Use global_approval_mode for the scope.
    pub approval_required: bool,
    pub global_approval_mode: GlobalApprovalMode,
    pub approval_timeout_secs: u32,
    pub grant_expiry_days: u32,
    pub push_enabled: bool,
    pub push_device_count: usize,
    pub updated_at: String,
}

impl NotificationSettingsAuthorizationEvidenceResponse {
    pub fn from_settings_response(response: &NotificationSettingsResponse) -> Self {
        Self {
            id: response.id.clone(),
            telegram_connected: response.telegram_connected,
            telegram_link_pending: response.telegram_link_pending,
            telegram_enabled: response.telegram_enabled,
            approval_required: response.approval_required,
            global_approval_mode: response.global_approval_mode,
            approval_timeout_secs: response.approval_timeout_secs,
            grant_expiry_days: response.grant_expiry_days,
            push_enabled: response.push_enabled,
            push_device_count: response.push_device_count,
            updated_at: response.updated_at.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateNotificationSettingsRequest {
    pub telegram_enabled: Option<bool>,
    pub approval_required: Option<bool>,
    pub global_approval_mode: Option<GlobalApprovalMode>,
    pub approval_timeout_secs: Option<u32>,
    pub grant_expiry_days: Option<u32>,
    pub push_enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TelegramLinkResponse {
    pub link_code: String,
    pub bot_username: String,
    pub expires_in_secs: u32,
    pub instructions: String,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/notifications/settings
pub async fn get_settings(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<NotificationSettingsResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    Ok(Json(to_settings_response(
        &channel,
        delivery_configured(&state, &channel),
    )))
}

/// GET /api/v1/notifications/settings/authorization
pub async fn get_settings_authorization(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<NotificationSettingsAuthorizationEvidenceResponse>> {
    let Json(detail) = get_settings(State(state), auth_user).await?;
    Ok(Json(
        NotificationSettingsAuthorizationEvidenceResponse::from_settings_response(&detail),
    ))
}

/// PUT /api/v1/notifications/settings
pub async fn update_settings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateNotificationSettingsRequest>,
) -> AppResult<Json<NotificationSettingsResponse>> {
    let user_id = auth_user.user_id.to_string();
    let principal = notification_principal(&auth_user);
    let patch = SettingsPatch {
        telegram_enabled: body.telegram_enabled,
        approval_required: body.approval_required,
        global_approval_mode: body.global_approval_mode,
        approval_timeout_secs: body.approval_timeout_secs,
        grant_expiry_days: body.grant_expiry_days,
        push_enabled: body.push_enabled,
    };
    let updated = settings::update(
        &state.db,
        &state.audit_runtime,
        &principal,
        &user_id,
        &patch,
        readiness(&state),
    )
    .await?;

    Ok(Json(to_settings_response(
        &updated,
        delivery_configured(&state, &updated),
    )))
}

// TODO(telemetry): emit `notification.channel_linked { channel: "telegram" }`
// from the Telegram webhook handler (`handlers/telegram_webhook.rs`), where
// the link is actually completed when the user sends the /start code to the
// bot. This handler only generates the link code; emitting here would misfire
// whenever a user starts but never finishes the flow. Webhook file is out of
// scope for this chunk.
//
/// POST /api/v1/notifications/telegram/link
///
/// Generate a one-time link code for connecting Telegram account.
pub async fn telegram_link(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<TelegramLinkResponse>> {
    let user_id = auth_user.user_id.to_string();
    let principal = notification_principal(&auth_user);
    let link_code = settings::issue_telegram_link(
        &state.db,
        &state.audit_runtime,
        &principal,
        &user_id,
        readiness(&state),
    )
    .await?;

    let bot_username = state
        .config
        .telegram_bot_username
        .clone()
        .unwrap_or_else(|| "NyxIDBot".to_string());

    Ok(Json(TelegramLinkResponse {
        link_code: link_code.clone(),
        bot_username: bot_username.clone(),
        expires_in_secs: 300,
        instructions: format!("Send /start {link_code} to @{bot_username} on Telegram"),
    }))
}

/// DELETE /api/v1/notifications/telegram
///
/// Disconnect Telegram from the user's notification settings.
pub async fn telegram_disconnect(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
) -> AppResult<Json<MessageResponse>> {
    let user_id = auth_user.user_id.to_string();
    let principal = notification_principal(&auth_user);
    let result = settings::disconnect_telegram(
        &state.db,
        &state.audit_runtime,
        &principal,
        &user_id,
        readiness(&state),
    )
    .await?;
    let web_only = result.channel.approval_required != Some(false)
        && !delivery_configured(&state, &result.channel);
    // Derived telemetry follows the definitely committed mutation, and only a
    // real unlink from the transaction snapshot contributes activity.
    let telegram_was_linked = result.telegram_was_linked;
    if telegram_was_linked {
        emit_event(
            state.telemetry.as_deref(),
            &user_id,
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::NotificationChannelUnlinked {
                channel: "telegram".to_string(),
            },
        );
    }

    let message = if web_only {
        "Telegram disconnected. Approval remains required. Review pending requests in Approval History on the web.".to_string()
    } else {
        "Telegram disconnected".to_string()
    };

    Ok(Json(MessageResponse { message }))
}

fn notification_principal(auth: &AuthUser) -> crate::models::audit_outbox::AuditPrincipal {
    use crate::models::audit_outbox::AuditPrincipal;
    match &auth.api_key_id {
        Some(id) => AuditPrincipal::ApiKey {
            id: id.clone(),
            owner_id: auth.user_id.to_string(),
        },
        None => AuditPrincipal::Person {
            id: auth.user_id.to_string(),
        },
    }
}

fn readiness(state: &AppState) -> Readiness<'_> {
    Readiness {
        config: &state.config,
        fcm_initialized: state.fcm_auth.is_some(),
        apns_initialized: state.apns_auth.is_some(),
    }
}

fn delivery_configured(state: &AppState, channel: &NotificationChannel) -> bool {
    notification_service::approval_delivery_configured(
        channel,
        &state.config,
        state.fcm_auth.is_some(),
        state.apns_auth.is_some(),
    )
}

fn to_settings_response(
    channel: &NotificationChannel,
    delivery_configured: bool,
) -> NotificationSettingsResponse {
    NotificationSettingsResponse {
        id: channel.id.clone(),
        telegram_connected: channel.telegram_chat_id.is_some(),
        telegram_link_pending: channel.telegram_link_code.is_some()
            && channel
                .telegram_link_code_expires_at
                .is_some_and(|expires_at| expires_at > Utc::now()),
        telegram_username: channel.telegram_username.clone(),
        telegram_enabled: channel.telegram_enabled,
        approval_required: channel.approval_required != Some(false),
        global_approval_mode: GlobalApprovalMode::from_stored(channel.approval_required),
        approval_suspended: false,
        approval_delivery_configured: delivery_configured,
        approval_web_available: true,
        approval_timeout_secs: channel.approval_timeout_secs,
        grant_expiry_days: channel.grant_expiry_days,
        push_enabled: channel.push_enabled,
        push_device_count: channel.push_devices.len(),
        updated_at: channel.updated_at.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::AppError;
    use crate::models::notification_channel::COLLECTION_NAME;
    use mongodb::bson::{self, doc};

    #[tokio::test]
    async fn unrelated_settings_preserve_unset_mode_and_conflicting_choices_do_not_create_rows() {
        let db = crate::test_utils::connect_test_database("notification_unset_mode_updates")
            .await
            .expect("real Mongo required");
        let state = crate::test_utils::test_app_state(db.clone());
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let request =
            |value| serde_json::from_value::<UpdateNotificationSettingsRequest>(value).unwrap();
        for (mode, legacy) in [("writes_only", true), ("all", false), ("off", true)] {
            let result = update_settings(
                State(state.clone()),
                crate::test_utils::test_auth_user(&owner),
                Json(request(
                    serde_json::json!({"global_approval_mode":mode,"approval_required":legacy}),
                )),
            )
            .await;
            assert!(matches!(result, Err(AppError::ValidationError(_))));
        }
        assert_eq!(
            db.collection::<NotificationChannel>(COLLECTION_NAME)
                .count_documents(doc! {"user_id":&owner})
                .await
                .unwrap(),
            0
        );
        let Json(updated) = update_settings(
            State(state),
            crate::test_utils::test_auth_user(&owner),
            Json(request(
                serde_json::json!({"approval_timeout_secs":60,"telegram_enabled":false}),
            )),
        )
        .await
        .unwrap();
        assert_eq!(updated.global_approval_mode, GlobalApprovalMode::WritesOnly);
        assert_eq!(updated.approval_timeout_secs, 60);
        let raw = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .unwrap();
        assert!(!raw.contains_key("approval_required"));
        crate::test_utils::teardown_test_database(&db).await;
    }

    #[tokio::test]
    async fn web_only_settings_allow_explicit_enable_opt_out_and_disabled_delivery() {
        let db = crate::test_utils::connect_test_database("notification_web_settings")
            .await
            .expect("configured Mongo required");
        let state = crate::test_utils::test_app_state(db.clone());
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let Json(first) = get_settings(
            State(state.clone()),
            crate::test_utils::test_auth_user(&owner),
        )
        .await
        .unwrap();
        assert!(first.approval_required && first.approval_web_available);
        assert!(!first.approval_suspended && !first.approval_delivery_configured);
        assert_eq!(first.approval_timeout_secs, 300);
        for required in [false, true] {
            let Json(result) = update_settings(
                State(state.clone()),
                crate::test_utils::test_auth_user(&owner),
                Json(UpdateNotificationSettingsRequest {
                    telegram_enabled: Some(false),
                    push_enabled: Some(false),
                    approval_required: Some(required),
                    global_approval_mode: None,
                    approval_timeout_secs: None,
                    grant_expiry_days: None,
                }),
            )
            .await
            .unwrap();
            assert_eq!(result.approval_required, required);
            assert!(!result.approval_suspended);
            assert_eq!(
                crate::services::approval_service::user_requires_approval(
                    &db,
                    &owner,
                    &crate::models::service_approval_config::ApprovalVerb::Write
                )
                .await
                .unwrap(),
                required
            );
        }
        let mut channel = notification_service::get_or_create_channel(&db, &owner)
            .await
            .unwrap();
        channel.push_enabled = true;
        channel
            .push_devices
            .push(crate::models::notification_channel::DeviceToken {
                device_id: "dummy".into(),
                platform: "fcm".into(),
                token: "dummy".into(),
                device_name: None,
                app_id: None,
                registered_at: Utc::now(),
                last_used_at: None,
            });
        let mut config = state.config.clone();
        config.fcm_project_id = Some("fixture-project".into());
        assert!(!notification_service::approval_delivery_configured(
            &channel, &config, false, false
        ));
        assert!(notification_service::approval_delivery_configured(
            &channel, &config, true, false
        ));
        channel.push_devices[0].token.clear();
        assert!(!notification_service::approval_delivery_configured(
            &channel, &config, true, true
        ));
        channel.push_devices[0].token = "dummy".into();
        channel.push_devices[0].platform = "apns".into();
        assert!(!notification_service::approval_delivery_configured(
            &channel, &config, true, true
        ));
        config.apns_topic = Some("dev.fixture.app".into());
        assert!(notification_service::approval_delivery_configured(
            &channel, &config, false, true
        ));
        channel.push_devices[0].platform = "unsupported".into();
        assert!(!notification_service::approval_delivery_configured(
            &channel, &config, true, true
        ));
        crate::test_utils::teardown_test_database(&db).await;
    }

    fn make_channel() -> NotificationChannel {
        NotificationChannel {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            telegram_chat_id: None,
            telegram_username: None,
            telegram_enabled: false,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            approval_required: Some(false),
            push_enabled: false,
            push_devices: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // --- Pure function tests: to_settings_response ---

    #[test]
    fn to_settings_response_defaults_with_no_telegram_and_no_push() {
        let channel = make_channel();
        let resp = to_settings_response(&channel, false);

        assert!(!resp.telegram_connected);
        assert!(resp.telegram_username.is_none());
        assert!(!resp.telegram_enabled);
        assert!(!resp.approval_required);
        assert!(!resp.approval_suspended);
        assert_eq!(resp.approval_timeout_secs, 30);
        assert_eq!(resp.grant_expiry_days, 30);
        assert!(!resp.push_enabled);
        assert_eq!(resp.push_device_count, 0);
    }

    #[test]
    fn to_settings_response_with_telegram_connected() {
        let mut channel = make_channel();
        channel.telegram_chat_id = Some(12345);
        channel.telegram_username = Some("testuser".to_string());
        channel.telegram_enabled = true;

        let resp = to_settings_response(&channel, false);

        assert!(resp.telegram_connected);
        assert_eq!(resp.telegram_username.as_deref(), Some("testuser"));
        assert!(resp.telegram_enabled);
    }

    #[test]
    fn to_settings_response_counts_push_devices() {
        let mut channel = make_channel();
        channel.push_enabled = true;
        for i in 0..3 {
            channel
                .push_devices
                .push(crate::models::notification_channel::DeviceToken {
                    device_id: format!("device-{i}"),
                    platform: "fcm".to_string(),
                    token: format!("token-{i}"),
                    device_name: None,
                    app_id: None,
                    registered_at: Utc::now(),
                    last_used_at: None,
                });
        }

        let resp = to_settings_response(&channel, false);

        assert!(resp.push_enabled);
        assert_eq!(resp.push_device_count, 3);
    }

    #[test]
    fn to_settings_response_with_custom_approval_settings() {
        let mut channel = make_channel();
        channel.approval_required = Some(true);
        channel.approval_timeout_secs = 120;
        channel.grant_expiry_days = 7;

        let resp = to_settings_response(&channel, false);

        assert!(resp.approval_required);
        assert!(!resp.approval_suspended);
        assert!(resp.approval_web_available);
        assert!(!resp.approval_delivery_configured);
        assert_eq!(resp.approval_timeout_secs, 120);
        assert_eq!(resp.grant_expiry_days, 7);
    }

    #[test]
    fn notification_authorization_projection_omits_telegram_username() {
        let mut channel = make_channel();
        channel.telegram_chat_id = Some(42);
        channel.telegram_username = Some("Bearer nyxid_ag_abcdefghijklmnop".to_string());
        channel.updated_at = Utc::now();
        let detail = to_settings_response(&channel, false);
        let value = serde_json::to_value(
            NotificationSettingsAuthorizationEvidenceResponse::from_settings_response(&detail),
        )
        .unwrap();
        assert!(value.get("telegram_username").is_none());
        assert!(value.to_string().find("nyxid_").is_none());
        assert_eq!(value["id"], channel.id);
    }

    // --- Serialization tests: NotificationSettingsResponse ---

    #[test]
    fn notification_settings_response_serialization() {
        let resp = NotificationSettingsResponse {
            id: "binding-1".to_string(),
            telegram_connected: true,
            telegram_link_pending: false,
            telegram_username: Some("alice".to_string()),
            telegram_enabled: true,
            approval_required: false,
            global_approval_mode: GlobalApprovalMode::Off,
            approval_suspended: false,
            approval_delivery_configured: false,
            approval_web_available: true,
            approval_timeout_secs: 60,
            grant_expiry_days: 14,
            push_enabled: true,
            push_device_count: 2,
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["telegram_connected"], true);
        assert_eq!(json["telegram_username"], "alice");
        assert_eq!(json["telegram_enabled"], true);
        assert_eq!(json["approval_required"], false);
        assert_eq!(json["approval_suspended"], false);
        assert_eq!(json["approval_timeout_secs"], 60);
        assert_eq!(json["grant_expiry_days"], 14);
        assert_eq!(json["push_enabled"], true);
        assert_eq!(json["push_device_count"], 2);
    }

    #[test]
    fn notification_settings_response_null_username() {
        let resp = NotificationSettingsResponse {
            id: "binding-1".to_string(),
            telegram_connected: false,
            telegram_link_pending: false,
            telegram_username: None,
            telegram_enabled: false,
            approval_required: false,
            global_approval_mode: GlobalApprovalMode::Off,
            approval_suspended: false,
            approval_delivery_configured: false,
            approval_web_available: true,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            push_enabled: false,
            push_device_count: 0,
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["telegram_connected"], false);
        assert!(json["telegram_username"].is_null());
    }

    // --- Serialization tests: TelegramLinkResponse ---

    #[test]
    fn telegram_link_response_serialization() {
        let resp = TelegramLinkResponse {
            link_code: "NYXID-ABC12345".to_string(),
            bot_username: "NyxIDBot".to_string(),
            expires_in_secs: 300,
            instructions: "Send /start NYXID-ABC12345 to @NyxIDBot on Telegram".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["link_code"], "NYXID-ABC12345");
        assert_eq!(json["bot_username"], "NyxIDBot");
        assert_eq!(json["expires_in_secs"], 300);
        assert!(json["instructions"].as_str().unwrap().contains("/start"));
    }

    // --- Serialization tests: MessageResponse ---

    #[test]
    fn message_response_serialization() {
        let resp = MessageResponse {
            message: "Telegram disconnected".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["message"], "Telegram disconnected");
    }

    // --- Deserialization tests: UpdateNotificationSettingsRequest ---

    #[test]
    fn update_notification_settings_request_deserialization_all_fields() {
        let json = serde_json::json!({
            "telegram_enabled": true,
            "approval_required": false,
            "approval_timeout_secs": 120,
            "grant_expiry_days": 7,
            "push_enabled": true
        });
        let req: UpdateNotificationSettingsRequest = serde_json::from_value(json).unwrap();

        assert_eq!(req.telegram_enabled, Some(true));
        assert_eq!(req.approval_required, Some(false));
        assert_eq!(req.approval_timeout_secs, Some(120));
        assert_eq!(req.grant_expiry_days, Some(7));
        assert_eq!(req.push_enabled, Some(true));
    }

    #[test]
    fn update_notification_settings_request_deserialization_empty_body() {
        let json = serde_json::json!({});
        let req: UpdateNotificationSettingsRequest = serde_json::from_value(json).unwrap();

        assert!(req.telegram_enabled.is_none());
        assert!(req.approval_required.is_none());
        assert!(req.approval_timeout_secs.is_none());
        assert!(req.grant_expiry_days.is_none());
        assert!(req.push_enabled.is_none());
    }
}
