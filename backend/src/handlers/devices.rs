use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::Deserialize;
use serde_json::json;
use std::fmt;

use crate::AppState;
use crate::crypto::device_code::decode_device_code;
use crate::errors::{AppError, AppResult};
use crate::handlers::auth::{extract_ip, extract_user_agent};
use crate::mw::auth::AuthUser;
use crate::redaction::RedactedLen;
#[cfg(not(test))]
use crate::services::audit_service;
use crate::services::device_code_service::{
    DeviceCodeApprove, DeviceCodeApproveInput, DeviceCodeInitiate, DeviceCodeInitiateInput,
    DeviceCodeLockoutNotification, DeviceCodePoll, DeviceCodePollInput, DeviceOnboard,
    DeviceOnboardInput, approve, claim_lockout_notification, initiate, onboard, poll,
};
use crate::services::notification_service;
use crate::services::notification_service::{
    DeviceNotificationContext, DeviceNotificationTemplate,
};

#[derive(Deserialize)]
pub struct RequestDeviceCodeRequest {
    pub device_pubkey: String,
    pub hw_id: String,
    #[serde(default)]
    pub suggested_label: Option<String>,
}

#[derive(Deserialize)]
pub struct PollDeviceCodeRequest {
    pub device_code: String,
    pub timestamp: i64,
    pub signature: String,
}

#[derive(Deserialize)]
pub struct ApproveDeviceCodeRequest {
    pub user_code: String,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub default_services: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct OnboardDeviceRequest {
    #[serde(default)]
    pub org_id: Option<String>,
    pub label: String,
    pub wifi_ssid: String,
    pub wifi_password: String,
    #[serde(default)]
    pub default_services: Option<Vec<String>>,
}

impl fmt::Debug for RequestDeviceCodeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestDeviceCodeRequest")
            .field("device_pubkey", &RedactedLen(self.device_pubkey.len()))
            .field("hw_id", &self.hw_id)
            .field("suggested_label", &self.suggested_label)
            .finish()
    }
}

impl fmt::Debug for PollDeviceCodeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollDeviceCodeRequest")
            .field("device_code", &RedactedLen(self.device_code.len()))
            .field("timestamp", &self.timestamp)
            .field("signature", &RedactedLen(self.signature.len()))
            .finish()
    }
}

impl fmt::Debug for ApproveDeviceCodeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApproveDeviceCodeRequest")
            .field("user_code", &RedactedLen(self.user_code.len()))
            .field("org_id", &self.org_id)
            .field("label", &self.label)
            .field("default_services", &self.default_services)
            .finish()
    }
}

impl fmt::Debug for OnboardDeviceRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnboardDeviceRequest")
            .field("org_id", &self.org_id)
            .field("label", &self.label)
            .field("wifi_ssid", &self.wifi_ssid)
            .field("wifi_password", &RedactedLen(self.wifi_password.len()))
            .field("default_services", &self.default_services)
            .finish()
    }
}

pub async fn request_device_code(
    State(state): State<AppState>,
    Json(req): Json<RequestDeviceCodeRequest>,
) -> AppResult<Json<DeviceCodeInitiate>> {
    let device_pubkey = decode_device_pubkey(&req.device_pubkey)?;
    let hw_id = normalize_hw_id(&req.hw_id)?;
    let suggested_label = normalize_suggested_label(req.suggested_label)?;

    let response = initiate(
        &state.db,
        DeviceCodeInitiateInput {
            device_pubkey,
            hw_id,
            suggested_label,
            frontend_url: state.config.frontend_url.clone(),
        },
    )
    .await?;

    Ok(Json(response))
}

pub async fn poll_device_code(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<PollDeviceCodeRequest>,
) -> AppResult<Json<DeviceCodePoll>> {
    let device_code = normalize_device_code(&req.device_code)?;
    let signature = decode_poll_signature(&req.signature)?;
    let ip_address = extract_ip(&headers, Some(peer));
    let user_agent = extract_user_agent(&headers);
    let response = match poll(
        &state.db,
        &state.encryption_keys,
        DeviceCodePollInput {
            device_code: device_code.clone(),
            timestamp: req.timestamp,
            signature,
        },
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            if matches!(error, AppError::DeviceCodeLocked)
                && let Err(notify_error) =
                    send_lockout_notifications(&state, &device_code, ip_address, user_agent).await
            {
                tracing::warn!(
                    error = %notify_error,
                    "Failed to send device-code lockout notification"
                );
            }
            return Err(error);
        }
    };

    Ok(Json(response))
}

pub async fn approve_device_code(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<ApproveDeviceCodeRequest>,
) -> AppResult<Json<DeviceCodeApprove>> {
    approve_device_code_with_notification_dispatcher(
        state,
        auth_user,
        req,
        spawn_bind_success_notification,
    )
    .await
}

pub async fn onboard_device(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<OnboardDeviceRequest>,
) -> AppResult<Json<DeviceOnboard>> {
    let actor_user_id = auth_user.user_id.to_string();
    let org_id = normalize_org_id(req.org_id)?;
    let label = normalize_onboard_label(&req.label)?;
    let wifi_ssid = normalize_wifi_ssid(&req.wifi_ssid)?;
    let wifi_password = normalize_wifi_password(&req.wifi_password)?;

    let response = onboard(
        &state.db,
        &state.encryption_keys,
        &actor_user_id,
        DeviceOnboardInput {
            org_id: org_id.clone(),
            label,
            wifi_ssid,
            wifi_password,
            default_services: req.default_services,
            base_url: state.config.base_url.clone(),
        },
    )
    .await?;

    log_device_audit_for_user(
        state.db.clone(),
        &auth_user,
        "device_onboard_created",
        Some(json!({
            "node_id": response.node_id.clone(),
            "api_key_id": response.api_key_id.clone(),
            "label": response.label.clone(),
            "org_id": org_id,
        })),
    );

    Ok(Json(response))
}

async fn approve_device_code_with_notification_dispatcher<F>(
    state: AppState,
    auth_user: AuthUser,
    req: ApproveDeviceCodeRequest,
    dispatch_notification: F,
) -> AppResult<Json<DeviceCodeApprove>>
where
    F: FnOnce(&AppState, String, DeviceNotificationContext),
{
    let user_code_prefix = audit_user_code_prefix(&req.user_code);
    let actor_user_id = auth_user.user_id.to_string();
    let user_code = match normalize_user_code(&req.user_code) {
        Ok(user_code) => user_code,
        Err(error) => {
            audit_failed_approve_attempt(&state, &auth_user, &user_code_prefix, &error);
            return Err(error);
        }
    };
    let label = match normalize_label(req.label) {
        Ok(label) => label,
        Err(error) => {
            audit_failed_approve_attempt(&state, &auth_user, &user_code_prefix, &error);
            return Err(error);
        }
    };
    let org_id = match normalize_org_id(req.org_id) {
        Ok(org_id) => org_id,
        Err(error) => {
            audit_failed_approve_attempt(&state, &auth_user, &user_code_prefix, &error);
            return Err(error);
        }
    };

    let response = match approve(
        &state.db,
        &state.encryption_keys,
        &actor_user_id,
        DeviceCodeApproveInput {
            user_code,
            org_id,
            label,
            default_services: req.default_services,
        },
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            audit_failed_approve_attempt(&state, &auth_user, &user_code_prefix, &error);
            return Err(error);
        }
    };

    log_device_audit_for_user(
        state.db.clone(),
        &auth_user,
        "device_code_approved",
        Some(json!({
            "api_key_id": response.api_key_id,
            "node_id": response.node_id,
            "owner_user_id": response.owner_user_id,
            "org_id": response.org_id,
            "hw_id": response.hw_id,
            "device_label": response.device_label,
        })),
    );

    let context = DeviceNotificationContext {
        device_label: response.device_label.clone(),
        hw_id: response.hw_id.clone(),
        node_id: Some(response.node_id.clone()),
        failed_poll_count: None,
        locked_until: None,
    };
    dispatch_notification(&state, actor_user_id, context);

    Ok(Json(response))
}

fn audit_failed_approve_attempt(
    state: &AppState,
    auth_user: &AuthUser,
    user_code_prefix: &str,
    error: &AppError,
) {
    log_device_audit_for_user(
        state.db.clone(),
        auth_user,
        "device_code_approve_failed",
        Some(json!({
            "user_code_prefix": user_code_prefix,
            "error_code": error.error_code(),
            "ip": auth_user.ip_address.clone(),
        })),
    );
}

#[cfg(not(test))]
fn log_device_audit_for_user(
    db: mongodb::Database,
    auth_user: &AuthUser,
    event_type: &'static str,
    event_data: Option<serde_json::Value>,
) {
    audit_service::log_for_user(db, auth_user, event_type, event_data);
}

#[cfg(test)]
fn log_device_audit_for_user(
    db: mongodb::Database,
    auth_user: &AuthUser,
    event_type: &'static str,
    event_data: Option<serde_json::Value>,
) {
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    let entry = AuditLog {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: Some(auth_user.user_id.to_string()),
        event_type: event_type.to_string(),
        event_data,
        ip_address: auth_user.ip_address.clone(),
        user_agent: auth_user.user_agent.clone(),
        api_key_id: auth_user.api_key_id.clone(),
        api_key_name: auth_user.api_key_name.clone(),
        created_at: chrono::Utc::now(),
    };

    tokio::spawn(async move {
        let event_type = entry.event_type.clone();
        let user_id = entry.user_id.clone();
        if let Err(error) = db
            .collection::<AuditLog>(AUDIT_LOG)
            .insert_one(&entry)
            .await
        {
            tracing::error!(event_type = %event_type, error = %error, "Failed to write audit log");
            return;
        }
        notify_device_audit_inserted(DeviceAuditInserted {
            event_type,
            user_id,
        });
    });
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct DeviceAuditInserted {
    event_type: String,
    user_id: Option<String>,
}

#[cfg(test)]
static DEVICE_AUDIT_INSERTED_TX: std::sync::OnceLock<
    tokio::sync::broadcast::Sender<DeviceAuditInserted>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn subscribe_device_audit_inserted() -> tokio::sync::broadcast::Receiver<DeviceAuditInserted> {
    DEVICE_AUDIT_INSERTED_TX
        .get_or_init(|| {
            let (tx, _rx) = tokio::sync::broadcast::channel(64);
            tx
        })
        .subscribe()
}

#[cfg(test)]
fn notify_device_audit_inserted(event: DeviceAuditInserted) {
    if let Some(tx) = DEVICE_AUDIT_INSERTED_TX.get() {
        let _ = tx.send(event);
    }
}

fn audit_user_code_prefix(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .take(4)
        .collect::<String>()
        .to_ascii_uppercase()
}

fn spawn_bind_success_notification(
    state: &AppState,
    user_id: String,
    context: DeviceNotificationContext,
) {
    let db = state.db.clone();
    let config = state.config.clone();
    let http_client = state.http_client.clone();
    let fcm_auth = state.fcm_auth.clone();
    let apns_auth = state.apns_auth.clone();

    tokio::spawn(async move {
        if let Err(error) = notification_service::send_device_notification(
            &db,
            &config,
            &http_client,
            fcm_auth.as_deref(),
            apns_auth.as_deref(),
            &user_id,
            DeviceNotificationTemplate::BindSuccess,
            &context,
        )
        .await
        {
            tracing::warn!(
                user_id = %user_id,
                error = %error,
                "Failed to send device bind success notification"
            );
        }
    });
}

/// Sends lockout notifications for bound device-code rows.
///
/// Pre-approval lockouts have no notification recipient because no org/user is
/// bound to the device_code yet; we record a system-level audit log instead so
/// operators can investigate via audit-log query.
async fn send_lockout_notifications(
    state: &AppState,
    device_code: &str,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> AppResult<()> {
    let Some(lockout) = claim_lockout_notification(&state.db, device_code).await? else {
        return Ok(());
    };

    if lockout.recipients.is_empty() {
        crate::services::audit_service::log_async(
            state.db.clone(),
            None,
            "device_code_locked_no_owner".to_string(),
            Some(lockout_no_owner_event_data(&lockout, ip_address.as_deref())),
            ip_address,
            user_agent,
            None,
            None,
        );
        tracing::warn!(
            hw_id = %lockout.hw_id,
            device_pubkey_fingerprint = %lockout.device_pubkey_fingerprint,
            failed_poll_count = lockout.failed_poll_count,
            "Device-code lockout has no approved owner or org admins to notify"
        );
        return Ok(());
    }

    let context = DeviceNotificationContext {
        device_label: lockout.device_label,
        hw_id: lockout.hw_id,
        node_id: lockout.node_id,
        failed_poll_count: Some(lockout.failed_poll_count),
        locked_until: Some(lockout.locked_until),
    };

    for recipient in lockout.recipients {
        for template in [
            DeviceNotificationTemplate::RepeatedFail,
            DeviceNotificationTemplate::LockAlert,
        ] {
            if let Err(error) = notification_service::send_device_notification(
                &state.db,
                &state.config,
                &state.http_client,
                state.fcm_auth.as_deref(),
                state.apns_auth.as_deref(),
                &recipient,
                template,
                &context,
            )
            .await
            {
                tracing::warn!(
                    user_id = %recipient,
                    template = %template.as_str(),
                    error = %error,
                    "Failed to send device-code failure notification"
                );
            }
        }
    }

    Ok(())
}

fn lockout_no_owner_event_data(
    lockout: &DeviceCodeLockoutNotification,
    ip_address: Option<&str>,
) -> serde_json::Value {
    json!({
        "device_pubkey_fingerprint": lockout.device_pubkey_fingerprint.clone(),
        "ip": ip_address,
        "hw_id": lockout.hw_id.clone(),
        "failed_poll_count": lockout.failed_poll_count,
        "locked_until": lockout.locked_until.to_rfc3339(),
    })
}

fn normalize_device_code(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    decode_device_code(trimmed)?;
    Ok(trimmed.to_string())
}

fn normalize_user_code(value: &str) -> AppResult<String> {
    let compact = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();

    if compact.len() != 12 || !compact.bytes().all(is_user_code_byte) {
        return Err(AppError::DeviceUserCodeInvalid);
    }

    Ok(format!(
        "{}-{}-{}",
        &compact[0..4],
        &compact[4..8],
        &compact[8..12]
    ))
}

fn is_user_code_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'A' | b'B'
            | b'C'
            | b'D'
            | b'E'
            | b'F'
            | b'G'
            | b'H'
            | b'J'
            | b'K'
            | b'L'
            | b'M'
            | b'N'
            | b'P'
            | b'Q'
            | b'R'
            | b'S'
            | b'T'
            | b'U'
            | b'V'
            | b'W'
            | b'X'
            | b'Y'
            | b'Z'
            | b'2'
            | b'3'
            | b'4'
            | b'5'
            | b'6'
            | b'7'
            | b'8'
            | b'9'
    )
}

fn decode_poll_signature(value: &str) -> AppResult<[u8; 64]> {
    let decoded = BASE64_STANDARD
        .decode(value)
        .map_err(|_| AppError::BadRequest("signature must be valid base64".to_string()))?;
    decoded
        .try_into()
        .map_err(|_| AppError::BadRequest("signature must decode to 64 bytes".to_string()))
}

fn decode_device_pubkey(value: &str) -> AppResult<[u8; 32]> {
    let decoded = BASE64_STANDARD
        .decode(value)
        .map_err(|_| AppError::BadRequest("device_pubkey must be valid base64".to_string()))?;
    decoded
        .try_into()
        .map_err(|_| AppError::BadRequest("device_pubkey must decode to 32 bytes".to_string()))
}

fn normalize_hw_id(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return Err(AppError::BadRequest(
            "hw_id must be between 1 and 256 characters".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_suggested_label(value: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 256 {
        return Err(AppError::BadRequest(
            "suggested_label must be at most 256 characters".to_string(),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn normalize_label(value: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 200 {
        return Err(AppError::BadRequest(
            "label must be at most 200 characters".to_string(),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn normalize_onboard_label(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return Err(AppError::BadRequest(
            "label must be between 1 and 128 characters".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_wifi_ssid(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 32 {
        return Err(AppError::BadRequest(
            "wifi_ssid must be between 1 and 32 characters".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_wifi_password(value: &str) -> AppResult<String> {
    if value.len() < 8 || value.len() > 63 {
        return Err(AppError::BadRequest(
            "wifi_password must be between 8 and 63 characters".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn normalize_org_id(value: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    uuid::Uuid::parse_str(trimmed)
        .map_err(|_| AppError::BadRequest("org_id must be a UUID".to_string()))?;
    Ok(Some(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::services::device_code_service::tests_support::setup_pending_row;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user};
    use mongodb::bson::doc;
    use std::time::Duration;
    use tokio::sync::oneshot;

    #[test]
    fn decode_device_pubkey_accepts_exactly_32_base64_bytes() {
        let encoded = BASE64_STANDARD.encode([5u8; 32]);

        assert_eq!(decode_device_pubkey(&encoded).unwrap(), [5u8; 32]);
    }

    #[tokio::test]
    async fn approve_handler_returns_before_slow_notification_dispatch_finishes() {
        let Some((db, response, _key)) =
            setup_pending_row("device_code_handler_approve_notification_async").await
        else {
            return;
        };
        let state = test_app_state(db);
        let actor_user_id = uuid::Uuid::new_v4().to_string();
        let auth_user = test_auth_user(&actor_user_id);
        let (release_notification_tx, release_notification_rx) = oneshot::channel::<()>();
        let (notification_done_tx, mut notification_done_rx) = oneshot::channel::<()>();

        let result = tokio::time::timeout(
            Duration::from_millis(200),
            approve_device_code_with_notification_dispatcher(
                state,
                auth_user,
                ApproveDeviceCodeRequest {
                    user_code: response.user_code,
                    org_id: None,
                    label: Some("Kitchen cam".to_string()),
                    default_services: None,
                },
                move |_state, _user_id, _context| {
                    tokio::spawn(async move {
                        let _ = release_notification_rx.await;
                        let _ = notification_done_tx.send(());
                    });
                },
            ),
        )
        .await
        .expect("approve should return without waiting for notification task");

        let _ = result.expect("approve succeeds");
        assert!(
            notification_done_rx.try_recv().is_err(),
            "notification task should still be pending after handler returns"
        );
        release_notification_tx
            .send(())
            .expect("release notification task");
        tokio::time::timeout(Duration::from_millis(200), notification_done_rx)
            .await
            .expect("notification task should finish after release")
            .expect("notification completion signal");
    }

    #[tokio::test]
    async fn approve_handler_audits_failed_attempts_with_redacted_user_code() {
        let Some(db) = connect_test_database("device_code_handler_approve_failed_audit").await
        else {
            return;
        };
        let state = test_app_state(db.clone());
        let actor_user_id = uuid::Uuid::new_v4().to_string();
        let mut auth_user = test_auth_user(&actor_user_id);
        auth_user.ip_address = Some("203.0.113.77".to_string());
        let mut audit_rx = subscribe_device_audit_inserted();

        let error = approve_device_code_with_notification_dispatcher(
            state,
            auth_user,
            ApproveDeviceCodeRequest {
                user_code: "abcd-efgh-jklm".to_string(),
                org_id: None,
                label: None,
                default_services: None,
            },
            |_state, _user_id, _context| panic!("notification should not dispatch on failure"),
        )
        .await
        .expect_err("approve should fail without matching code");
        assert!(matches!(error, AppError::DeviceUserCodeInvalid));

        let audit = wait_for_approve_failed_audit(&db, &actor_user_id, &mut audit_rx).await;
        let event = audit
            .event_data
            .expect("device_code_approve_failed event data");
        assert_eq!(event["user_code_prefix"], "ABCD");
        assert_eq!(event["error_code"], 9503);
        assert_eq!(event["ip"], "203.0.113.77");
    }

    #[test]
    fn decode_device_pubkey_rejects_invalid_base64() {
        let error = decode_device_pubkey("not base64").expect_err("invalid");

        assert!(matches!(error, AppError::BadRequest(_)));
    }

    #[test]
    fn decode_device_pubkey_rejects_wrong_length() {
        let encoded = BASE64_STANDARD.encode([5u8; 31]);
        let error = decode_device_pubkey(&encoded).expect_err("wrong length");

        assert!(matches!(error, AppError::BadRequest(_)));
    }

    #[test]
    fn normalize_device_code_accepts_base64url_unpadded() {
        let raw = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

        assert_eq!(normalize_device_code(raw).unwrap(), raw);
    }

    #[test]
    fn normalize_device_code_rejects_wrong_shape() {
        assert!(normalize_device_code("abc").is_err());
        assert!(normalize_device_code(&"=".repeat(43)).is_err());
        assert!(normalize_device_code(&"A".repeat(44)).is_err());
    }

    #[test]
    fn normalize_user_code_accepts_spaces_dashes_and_lowercase() {
        assert_eq!(
            normalize_user_code("abcd efgh jklm").unwrap(),
            "ABCD-EFGH-JKLM"
        );
        assert_eq!(
            normalize_user_code("abcd-efgh-jklm").unwrap(),
            "ABCD-EFGH-JKLM"
        );
    }

    #[test]
    fn normalize_user_code_rejects_ambiguous_or_wrong_length_input() {
        assert!(matches!(
            normalize_user_code("ABCD-EFGH-JKL").expect_err("short"),
            AppError::DeviceUserCodeInvalid
        ));
        assert!(matches!(
            normalize_user_code("ABCD-EFGH-IJKL").expect_err("ambiguous"),
            AppError::DeviceUserCodeInvalid
        ));
        assert!(matches!(
            normalize_user_code("ABCD-EFGH-OJKL").expect_err("ambiguous"),
            AppError::DeviceUserCodeInvalid
        ));
    }

    #[test]
    fn decode_poll_signature_accepts_exactly_64_base64_bytes() {
        let encoded = BASE64_STANDARD.encode([8u8; 64]);

        assert_eq!(decode_poll_signature(&encoded).unwrap(), [8u8; 64]);
    }

    #[test]
    fn decode_poll_signature_rejects_wrong_length() {
        let encoded = BASE64_STANDARD.encode([8u8; 63]);

        assert!(decode_poll_signature(&encoded).is_err());
    }

    #[test]
    fn normalize_hw_id_trims_and_bounds_length() {
        assert_eq!(normalize_hw_id(" esp32 ").unwrap(), "esp32");
        assert!(normalize_hw_id("").is_err());
        assert!(normalize_hw_id(&"x".repeat(257)).is_err());
    }

    #[test]
    fn normalize_suggested_label_trims_empty_to_none_and_caps_length() {
        assert_eq!(
            normalize_suggested_label(Some(" Lab ".to_string())).unwrap(),
            Some("Lab".to_string())
        );
        assert_eq!(
            normalize_suggested_label(Some("   ".to_string())).unwrap(),
            None
        );
        assert!(normalize_suggested_label(Some("x".repeat(257))).is_err());
    }

    #[test]
    fn normalize_label_trims_empty_to_none_and_caps_length() {
        assert_eq!(
            normalize_label(Some(" Hallway ".to_string())).unwrap(),
            Some("Hallway".to_string())
        );
        assert_eq!(normalize_label(Some("   ".to_string())).unwrap(), None);
        assert!(normalize_label(Some("x".repeat(201))).is_err());
    }

    #[test]
    fn normalize_onboard_fields_trim_and_enforce_wifi_bounds() {
        assert_eq!(normalize_onboard_label(" Kitchen ").unwrap(), "Kitchen");
        assert_eq!(normalize_wifi_ssid(" Home ").unwrap(), "Home");
        assert_eq!(normalize_wifi_password("hunter22").unwrap(), "hunter22");
        assert!(normalize_onboard_label("").is_err());
        assert!(normalize_onboard_label(&"x".repeat(129)).is_err());
        assert!(normalize_wifi_ssid("").is_err());
        assert!(normalize_wifi_ssid(&"x".repeat(33)).is_err());
        assert!(normalize_wifi_password("short").is_err());
        assert!(normalize_wifi_password(&"x".repeat(64)).is_err());
    }

    #[tokio::test]
    async fn onboard_handler_creates_payload_and_audits_without_secrets() {
        let Some(db) = connect_test_database("device_onboard_handler_happy").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");
        let state = test_app_state(db.clone());
        let actor_user_id = uuid::Uuid::new_v4().to_string();
        let auth_user = test_auth_user(&actor_user_id);
        let mut audit_rx = subscribe_device_audit_inserted();

        let Json(response) = onboard_device(
            State(state),
            auth_user,
            Json(OnboardDeviceRequest {
                org_id: None,
                label: "Kitchen Camera".to_string(),
                wifi_ssid: "MyHomeNetwork".to_string(),
                wifi_password: "hunter22".to_string(),
                default_services: None,
            }),
        )
        .await
        .expect("onboard");

        assert_eq!(response.label, "Kitchen Camera");
        assert!(response.qr_payload.contains("nyxid_ag_"));
        assert!(response.qr_payload.contains("psw=hunter22"));

        let audit = wait_for_onboard_audit(&db, &actor_user_id, &mut audit_rx).await;
        let event = audit.event_data.expect("device_onboard_created event data");
        assert_eq!(event["node_id"], response.node_id);
        assert_eq!(event["api_key_id"], response.api_key_id);
        assert_eq!(event["label"], "Kitchen Camera");
        let serialized = serde_json::to_string(&event).expect("event json");
        assert!(!serialized.contains("hunter22"));
        assert!(!serialized.contains("nyxid_ag_"));
        assert!(!serialized.contains("MyHomeNetwork"));
    }

    #[test]
    fn normalize_org_id_accepts_uuid_and_rejects_names() {
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(normalize_org_id(Some(id.clone())).unwrap(), Some(id));
        assert!(normalize_org_id(Some("my-org".to_string())).is_err());
    }

    #[test]
    fn lockout_no_owner_event_data_includes_forensic_fields() {
        let locked_until = chrono::Utc::now();
        let event = lockout_no_owner_event_data(
            &DeviceCodeLockoutNotification {
                recipients: vec![],
                device_label: "Kitchen cam".to_string(),
                hw_id: "esp32-p4-cam-1".to_string(),
                node_id: None,
                device_pubkey_fingerprint: "0123456789abcdef".to_string(),
                failed_poll_count: 3,
                locked_until,
            },
            Some("203.0.113.10"),
        );

        assert_eq!(event["device_pubkey_fingerprint"], "0123456789abcdef");
        assert_eq!(event["ip"], "203.0.113.10");
        assert_eq!(event["hw_id"], "esp32-p4-cam-1");
        assert_eq!(event["failed_poll_count"], 3);
        assert_eq!(event["locked_until"], locked_until.to_rfc3339());
    }

    async fn wait_for_approve_failed_audit(
        db: &mongodb::Database,
        user_id: &str,
        audit_rx: &mut tokio::sync::broadcast::Receiver<DeviceAuditInserted>,
    ) -> AuditLog {
        wait_for_device_audit(db, user_id, "device_code_approve_failed", audit_rx).await
    }

    async fn wait_for_onboard_audit(
        db: &mongodb::Database,
        user_id: &str,
        audit_rx: &mut tokio::sync::broadcast::Receiver<DeviceAuditInserted>,
    ) -> AuditLog {
        wait_for_device_audit(db, user_id, "device_onboard_created", audit_rx).await
    }

    async fn wait_for_device_audit(
        db: &mongodb::Database,
        user_id: &str,
        event_type: &str,
        audit_rx: &mut tokio::sync::broadcast::Receiver<DeviceAuditInserted>,
    ) -> AuditLog {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let event = audit_rx.recv().await.expect("device audit insert event");
                if event.event_type == event_type && event.user_id.as_deref() == Some(user_id) {
                    break;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{event_type} audit entry was not written"));

        db.collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! {
                "user_id": user_id,
                "event_type": event_type,
            })
            .await
            .expect("query audit")
            .unwrap_or_else(|| panic!("{event_type} audit entry was not found"))
    }
}
