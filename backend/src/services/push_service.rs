use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::errors::{AppError, AppResult};

// ---------------------------------------------------------------------------
// Dedicated HTTP/2 client for APNs
// ---------------------------------------------------------------------------
// Apple requires HTTP/2 for all APNs connections. The shared reqwest client
// negotiates the protocol via ALPN, which can cause hyper Parse(Version)
// errors when the connection pool mixes HTTP/1.1 and HTTP/2 origins.
// A dedicated client with `http2_prior_knowledge()` forces HTTP/2 framing
// unconditionally, eliminating the mismatch.

static APNS_HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .http2_prior_knowledge()
        .connect_timeout(std::time::Duration::from_secs(10))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .build()
        .expect("Failed to create APNs HTTP/2 client")
});

// ---------------------------------------------------------------------------
// Shared token cache
// ---------------------------------------------------------------------------

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

// ---------------------------------------------------------------------------
// FCM (Firebase Cloud Messaging) via HTTP v1 API
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FcmServiceAccount {
    client_email: String,
    private_key: String,
}

/// Holds the FCM service account credentials and a cached OAuth2 access token.
pub struct FcmAuth {
    service_account: FcmServiceAccount,
    cached_token: RwLock<Option<CachedToken>>,
}

impl FcmAuth {
    /// Load and parse a Google service account JSON file.
    pub fn from_service_account_file(path: &str) -> AppResult<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            AppError::Internal(format!("Failed to read FCM service account at {path}: {e}"))
        })?;

        let sa: FcmServiceAccount = serde_json::from_str(&content)
            .map_err(|e| AppError::Internal(format!("Invalid FCM service account JSON: {e}")))?;

        Ok(Self {
            service_account: sa,
            cached_token: RwLock::new(None),
        })
    }

    /// Get a valid OAuth2 access token, refreshing if needed.
    async fn get_access_token(&self, http_client: &Client) -> AppResult<String> {
        // Check cached token
        {
            let guard = self.cached_token.read().await;
            if let Some(cached) = guard.as_ref() {
                if Instant::now() < cached.expires_at {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        self.refresh_access_token(http_client).await
    }

    async fn refresh_access_token(&self, http_client: &Client) -> AppResult<String> {
        // Acquire write lock first to prevent thundering herd:
        // multiple tasks observing an expired token would otherwise all refresh concurrently.
        let mut guard = self.cached_token.write().await;

        // Double-check: another task may have refreshed while we waited for the write lock
        if let Some(cached) = guard.as_ref() {
            if Instant::now() < cached.expires_at {
                return Ok(cached.access_token.clone());
            }
        }

        let now = chrono::Utc::now();
        let exp = now + chrono::Duration::hours(1);

        let claims = serde_json::json!({
            "iss": self.service_account.client_email,
            "scope": "https://www.googleapis.com/auth/firebase.messaging",
            "aud": "https://oauth2.googleapis.com/token",
            "iat": now.timestamp(),
            "exp": exp.timestamp(),
        });

        let key =
            jsonwebtoken::EncodingKey::from_rsa_pem(self.service_account.private_key.as_bytes())
                .map_err(|e| AppError::Internal(format!("Invalid FCM private key: {e}")))?;

        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        let assertion = jsonwebtoken::encode(&header, &claims, &key)
            .map_err(|e| AppError::Internal(format!("Failed to sign FCM JWT assertion: {e}")))?;

        let resp = http_client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &assertion),
            ])
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("FCM token request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(AppError::Internal(format!(
                "FCM token exchange failed: {body}"
            )));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Internal(format!("FCM token parse failed: {e}")))?;

        let expires_at = Instant::now()
            + std::time::Duration::from_secs(token_resp.expires_in.saturating_sub(60));

        let access_token = token_resp.access_token.clone();

        *guard = Some(CachedToken {
            access_token: token_resp.access_token,
            expires_at,
        });

        Ok(access_token)
    }
}

/// Result of sending an FCM notification.
pub enum FcmSendResult {
    /// Successful delivery (FCM accepted the message).
    Success {
        #[allow(dead_code)]
        message_name: String,
    },
    /// The device token is invalid (UNREGISTERED); caller should remove it.
    TokenInvalid,
    /// Send failed for another reason.
    Failed { reason: String },
}

#[derive(Serialize)]
struct FcmMessagePayload {
    message: FcmMessage,
}

#[derive(Serialize)]
struct FcmMessage {
    token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<FcmNotification>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    android: Option<FcmAndroidConfig>,
}

#[derive(Serialize)]
struct FcmNotification {
    title: String,
    body: String,
}

#[derive(Serialize)]
struct FcmAndroidConfig {
    priority: String,
    notification: FcmAndroidNotification,
}

#[derive(Serialize)]
struct FcmAndroidNotification {
    channel_id: String,
    sound: String,
    default_sound: bool,
    default_vibrate_timings: bool,
    notification_priority: String,
}

/// Parse a successful FCM response into an `FcmSendResult`.
async fn parse_fcm_success(resp: reqwest::Response) -> FcmSendResult {
    #[derive(Deserialize)]
    struct FcmResponse {
        name: String,
    }
    let fcm_resp: FcmResponse = resp.json().await.unwrap_or(FcmResponse {
        name: String::new(),
    });
    FcmSendResult::Success {
        message_name: fcm_resp.name,
    }
}

/// Send a push notification via FCM HTTP v1 API.
pub async fn send_fcm_notification(
    http_client: &Client,
    fcm_auth: &FcmAuth,
    project_id: &str,
    device_token: &str,
    title: &str,
    body: &str,
    data: &HashMap<String, String>,
) -> AppResult<FcmSendResult> {
    let access_token = fcm_auth.get_access_token(http_client).await?;

    let payload = FcmMessagePayload {
        message: FcmMessage {
            token: device_token.to_string(),
            notification: Some(FcmNotification {
                title: title.to_string(),
                body: body.to_string(),
            }),
            data: if data.is_empty() {
                None
            } else {
                Some(data.clone())
            },
            android: Some(FcmAndroidConfig {
                priority: "high".to_string(),
                notification: FcmAndroidNotification {
                    channel_id: "approvals".to_string(),
                    sound: "default".to_string(),
                    default_sound: true,
                    default_vibrate_timings: true,
                    notification_priority: "PRIORITY_MAX".to_string(),
                },
            }),
        },
    };

    let url = format!("https://fcm.googleapis.com/v1/projects/{project_id}/messages:send");

    let resp = http_client
        .post(&url)
        .bearer_auth(&access_token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("FCM send request failed: {e}")))?;

    let status = resp.status();

    if status.is_success() {
        return Ok(parse_fcm_success(resp).await);
    }

    let resp_body = resp.text().await.unwrap_or_default();

    match status.as_u16() {
        404 => {
            tracing::warn!("FCM token unregistered, marking for removal");
            Ok(FcmSendResult::TokenInvalid)
        }
        401 => {
            // Token expired -- try refresh once
            tracing::warn!("FCM 401: refreshing access token and retrying");
            let new_token = fcm_auth.refresh_access_token(http_client).await?;

            let retry_resp = http_client
                .post(&url)
                .bearer_auth(&new_token)
                .json(&payload)
                .send()
                .await
                .map_err(|e| AppError::Internal(format!("FCM retry failed: {e}")))?;

            if retry_resp.status().is_success() {
                Ok(parse_fcm_success(retry_resp).await)
            } else {
                let retry_body = retry_resp.text().await.unwrap_or_default();
                tracing::warn!("FCM retry also failed: {retry_body}");
                Ok(FcmSendResult::Failed {
                    reason: format!("FCM retry failed (401): {retry_body}"),
                })
            }
        }
        400 => {
            tracing::warn!("FCM 400 bad request: {resp_body}");
            Ok(FcmSendResult::Failed {
                reason: format!("FCM bad request: {resp_body}"),
            })
        }
        _ => {
            tracing::warn!(status = %status, "FCM send failed: {resp_body}");
            Ok(FcmSendResult::Failed {
                reason: format!("FCM error {status}: {resp_body}"),
            })
        }
    }
}

/// Send a silent (data-only) FCM notification for decision updates.
pub async fn send_fcm_silent(
    http_client: &Client,
    fcm_auth: &FcmAuth,
    project_id: &str,
    device_token: &str,
    data: &HashMap<String, String>,
) -> AppResult<FcmSendResult> {
    let access_token = fcm_auth.get_access_token(http_client).await?;

    let payload = FcmMessagePayload {
        message: FcmMessage {
            token: device_token.to_string(),
            notification: None,
            data: Some(data.clone()),
            android: None,
        },
    };

    let url = format!("https://fcm.googleapis.com/v1/projects/{project_id}/messages:send");

    let resp = http_client
        .post(&url)
        .bearer_auth(&access_token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("FCM silent send failed: {e}")))?;

    if resp.status().is_success() {
        Ok(FcmSendResult::Success {
            message_name: String::new(),
        })
    } else if resp.status().as_u16() == 404 {
        Ok(FcmSendResult::TokenInvalid)
    } else {
        let body = resp.text().await.unwrap_or_default();
        Ok(FcmSendResult::Failed {
            reason: format!("FCM silent error: {body}"),
        })
    }
}

// ---------------------------------------------------------------------------
// APNs (Apple Push Notification service)
// ---------------------------------------------------------------------------

/// Holds the APNs provider token credentials and a cached JWT.
pub struct ApnsAuth {
    encoding_key: jsonwebtoken::EncodingKey,
    key_id: String,
    team_id: String,
    cached_token: RwLock<Option<CachedToken>>,
}

impl ApnsAuth {
    /// Load an APNs .p8 key and prepare the signing state.
    pub fn new(key_path: &str, key_id: &str, team_id: &str) -> AppResult<Self> {
        let key_bytes = std::fs::read(key_path).map_err(|e| {
            AppError::Internal(format!("Failed to read APNs key at {key_path}: {e}"))
        })?;

        let encoding_key = jsonwebtoken::EncodingKey::from_ec_pem(&key_bytes)
            .map_err(|e| AppError::Internal(format!("Invalid APNs .p8 key: {e}")))?;

        Ok(Self {
            encoding_key,
            key_id: key_id.to_string(),
            team_id: team_id.to_string(),
            cached_token: RwLock::new(None),
        })
    }

    /// Get a valid APNs provider JWT, refreshing if needed.
    async fn get_token(&self) -> AppResult<String> {
        {
            let guard = self.cached_token.read().await;
            if let Some(cached) = guard.as_ref() {
                if Instant::now() < cached.expires_at {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        self.refresh_token().await
    }

    async fn refresh_token(&self) -> AppResult<String> {
        // Acquire write lock first to prevent redundant JWT signing
        let mut guard = self.cached_token.write().await;

        // Double-check: another task may have refreshed while we waited
        if let Some(cached) = guard.as_ref() {
            if Instant::now() < cached.expires_at {
                return Ok(cached.access_token.clone());
            }
        }

        let now = chrono::Utc::now();

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(self.key_id.clone());

        let claims = serde_json::json!({
            "iss": self.team_id,
            "iat": now.timestamp(),
        });

        let token = jsonwebtoken::encode(&header, &claims, &self.encoding_key)
            .map_err(|e| AppError::Internal(format!("Failed to sign APNs JWT: {e}")))?;

        // Cache for 50 minutes (APNs tokens valid up to 1 hour)
        let expires_at = Instant::now() + std::time::Duration::from_secs(50 * 60);

        let token_clone = token.clone();
        *guard = Some(CachedToken {
            access_token: token,
            expires_at,
        });

        Ok(token_clone)
    }
}

/// Result of sending an APNs notification.
pub enum ApnsSendResult {
    /// Successfully accepted by APNs.
    Success,
    /// Device token is invalid (BadDeviceToken or Unregistered); remove it.
    TokenInvalid,
    /// Send failed for another reason.
    Failed { reason: String },
}

/// Send an alert notification via APNs HTTP/2 API.
///
/// Uses a dedicated HTTP/2 client (`APNS_HTTP_CLIENT`) instead of the shared
/// client to avoid protocol version mismatch errors with Apple's servers.
pub async fn send_apns_notification(
    _http_client: &Client,
    apns_auth: &ApnsAuth,
    device_token: &str,
    topic: &str,
    sandbox: bool,
    title: &str,
    body: &str,
    data: &HashMap<String, String>,
) -> AppResult<ApnsSendResult> {
    let jwt = apns_auth.get_token().await?;

    let host = if sandbox {
        "api.sandbox.push.apple.com"
    } else {
        "api.push.apple.com"
    };

    let url = format!("https://{host}/3/device/{device_token}");

    // Store-and-forward for up to 5 minutes (matches typical approval timeout)
    let apns_expiration = format!("{}", chrono::Utc::now().timestamp() + 300);

    let mut payload = serde_json::json!({
        "aps": {
            "alert": {
                "title": title,
                "body": body,
            },
            "sound": "default",
            "mutable-content": 1,
            "category": "APPROVAL_REQUEST",
        }
    });

    // Merge data fields into the top-level payload
    if let Some(obj) = payload.as_object_mut() {
        for (k, v) in data {
            obj.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
    }

    let resp = APNS_HTTP_CLIENT
        .post(&url)
        .bearer_auth(&jwt)
        .header("apns-topic", topic)
        .header("apns-push-type", "alert")
        .header("apns-priority", "10")
        .header("apns-expiration", &apns_expiration)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("APNs send failed for {url}: {e:?}");
            AppError::Internal(format!("APNs send failed: {e}"))
        })?;

    let status = resp.status();

    if status.is_success() {
        return Ok(ApnsSendResult::Success);
    }

    let resp_body = resp.text().await.unwrap_or_default();

    match status.as_u16() {
        400 if resp_body.contains("BadDeviceToken") => {
            tracing::warn!("APNs BadDeviceToken, marking for removal");
            Ok(ApnsSendResult::TokenInvalid)
        }
        410 => {
            tracing::warn!("APNs 410 Unregistered, marking for removal");
            Ok(ApnsSendResult::TokenInvalid)
        }
        403 if resp_body.contains("ExpiredProviderToken") => {
            // Refresh JWT and retry once
            tracing::warn!("APNs ExpiredProviderToken: refreshing and retrying");
            let new_jwt = apns_auth.refresh_token().await?;

            let retry_resp = APNS_HTTP_CLIENT
                .post(&url)
                .bearer_auth(&new_jwt)
                .header("apns-topic", topic)
                .header("apns-push-type", "alert")
                .header("apns-priority", "10")
                .header("apns-expiration", &apns_expiration)
                .json(&payload)
                .send()
                .await
                .map_err(|e| {
                    tracing::error!("APNs retry failed for {url}: {e:?}");
                    AppError::Internal(format!("APNs retry failed: {e}"))
                })?;

            if retry_resp.status().is_success() {
                Ok(ApnsSendResult::Success)
            } else {
                let retry_body = retry_resp.text().await.unwrap_or_default();
                Ok(ApnsSendResult::Failed {
                    reason: format!("APNs retry failed: {retry_body}"),
                })
            }
        }
        _ => {
            tracing::warn!(status = %status, "APNs send failed: {resp_body}");
            Ok(ApnsSendResult::Failed {
                reason: format!("APNs error {status}: {resp_body}"),
            })
        }
    }
}

/// Send a silent (background) notification via APNs.
///
/// Uses the dedicated HTTP/2 client (`APNS_HTTP_CLIENT`).
pub async fn send_apns_silent(
    _http_client: &Client,
    apns_auth: &ApnsAuth,
    device_token: &str,
    topic: &str,
    sandbox: bool,
    data: &HashMap<String, String>,
) -> AppResult<ApnsSendResult> {
    let jwt = apns_auth.get_token().await?;

    let host = if sandbox {
        "api.sandbox.push.apple.com"
    } else {
        "api.push.apple.com"
    };

    let url = format!("https://{host}/3/device/{device_token}");

    let mut payload = serde_json::json!({
        "aps": {
            "content-available": 1,
        }
    });

    if let Some(obj) = payload.as_object_mut() {
        for (k, v) in data {
            obj.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
    }

    let resp = APNS_HTTP_CLIENT
        .post(&url)
        .bearer_auth(&jwt)
        .header("apns-topic", topic)
        .header("apns-push-type", "background")
        .header("apns-priority", "5")
        .header("apns-expiration", "0")
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("APNs silent send failed for {url}: {e:?}");
            AppError::Internal(format!("APNs silent send failed: {e}"))
        })?;

    let resp_status = resp.status().as_u16();

    if resp.status().is_success() {
        Ok(ApnsSendResult::Success)
    } else {
        // Read body once before branching to avoid losing error details
        let body = resp.text().await.unwrap_or_default();
        if resp_status == 410 || (resp_status == 400 && body.contains("BadDeviceToken")) {
            Ok(ApnsSendResult::TokenInvalid)
        } else {
            Ok(ApnsSendResult::Failed {
                reason: format!("APNs silent error {resp_status}: {body}"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fcm_send_result_variants() {
        let success = FcmSendResult::Success {
            message_name: "projects/test/messages/123".to_string(),
        };
        assert!(matches!(success, FcmSendResult::Success { .. }));
        assert!(matches!(
            FcmSendResult::TokenInvalid,
            FcmSendResult::TokenInvalid
        ));
        assert!(matches!(
            FcmSendResult::Failed {
                reason: String::new()
            },
            FcmSendResult::Failed { .. }
        ));
    }

    #[test]
    fn apns_send_result_variants() {
        assert!(matches!(ApnsSendResult::Success, ApnsSendResult::Success));
        assert!(matches!(
            ApnsSendResult::TokenInvalid,
            ApnsSendResult::TokenInvalid
        ));
        assert!(matches!(
            ApnsSendResult::Failed {
                reason: String::new()
            },
            ApnsSendResult::Failed { .. }
        ));
    }

    #[test]
    fn fcm_message_payload_serialization() {
        let payload = FcmMessagePayload {
            message: FcmMessage {
                token: "device-token-123".to_string(),
                notification: Some(FcmNotification {
                    title: "Test Title".to_string(),
                    body: "Test Body".to_string(),
                }),
                data: Some(HashMap::from([
                    ("type".to_string(), "approval_request".to_string()),
                    ("request_id".to_string(), "req-1".to_string()),
                ])),
                android: Some(FcmAndroidConfig {
                    priority: "high".to_string(),
                    notification: FcmAndroidNotification {
                        channel_id: "approvals".to_string(),
                        sound: "default".to_string(),
                        default_sound: true,
                        default_vibrate_timings: true,
                        notification_priority: "PRIORITY_MAX".to_string(),
                    },
                }),
            },
        };

        let json = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(json["message"]["token"], "device-token-123");
        assert_eq!(json["message"]["notification"]["title"], "Test Title");
        assert_eq!(json["message"]["notification"]["body"], "Test Body");
        assert_eq!(json["message"]["android"]["priority"], "high");
        assert_eq!(
            json["message"]["android"]["notification"]["channel_id"],
            "approvals"
        );
        assert_eq!(
            json["message"]["android"]["notification"]["notification_priority"],
            "PRIORITY_MAX"
        );
        assert!(json["message"]["data"]["type"].is_string());
    }

    #[test]
    fn fcm_message_payload_omits_none_fields() {
        let payload = FcmMessagePayload {
            message: FcmMessage {
                token: "tok".to_string(),
                notification: None,
                data: None,
                android: None,
            },
        };

        let json = serde_json::to_value(&payload).expect("serialize");
        assert!(json["message"]["notification"].is_null());
        assert!(json["message"]["data"].is_null());
        assert!(json["message"]["android"].is_null());
    }

    #[test]
    fn apns_alert_payload_structure() {
        let payload = serde_json::json!({
            "aps": {
                "alert": {
                    "title": "Approval Required",
                    "body": "A service is requesting access",
                },
                "sound": "default",
                "mutable-content": 1,
                "category": "APPROVAL_REQUEST",
            },
            "type": "approval_request",
            "request_id": "req-123",
        });

        assert_eq!(payload["aps"]["alert"]["title"], "Approval Required");
        assert_eq!(payload["aps"]["category"], "APPROVAL_REQUEST");
        // Badge should NOT be present (app manages its own badge count)
        assert!(payload["aps"]["badge"].is_null());
        assert_eq!(payload["type"], "approval_request");
    }

    #[test]
    fn apns_silent_payload_structure() {
        let mut payload = serde_json::json!({
            "aps": {
                "content-available": 1,
            }
        });

        let data = HashMap::from([
            ("type".to_string(), "approval_decision".to_string()),
            ("request_id".to_string(), "req-456".to_string()),
        ]);

        if let Some(obj) = payload.as_object_mut() {
            for (k, v) in &data {
                obj.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
        }

        assert_eq!(payload["aps"]["content-available"], 1);
        assert_eq!(payload["type"], "approval_decision");
        assert_eq!(payload["request_id"], "req-456");
        // Silent push should NOT have alert or badge
        assert!(payload["aps"]["alert"].is_null());
    }

    #[test]
    fn fcm_service_account_deserialization() {
        let json = r#"{"client_email":"test@project.iam.gserviceaccount.com","private_key":"-----BEGIN RSA PRIVATE KEY-----\nfake\n-----END RSA PRIVATE KEY-----\n"}"#;
        let sa: FcmServiceAccount = serde_json::from_str(json).expect("deserialize");
        assert_eq!(sa.client_email, "test@project.iam.gserviceaccount.com");
        assert!(sa.private_key.contains("RSA PRIVATE KEY"));
    }
}
