# Mobile Push Notification Architecture Plan

> **Status: IMPLEMENTED** (March 2026). This plan has been fully implemented and reviewed. See the implementation in `backend/src/services/push_service.rs` and `backend/src/handlers/device_tokens.rs`. Post-review hardening includes: platform-specific device token validation, atomic array-size enforcement to prevent TOCTOU races, `last_used_at` tracking on successful push delivery, and `app_id` length validation.

## Overview

This plan extends NyxID's existing approval notification system to support mobile push notifications via **Firebase Cloud Messaging (FCM)** and **Apple Push Notification service (APNs)**, in addition to the existing Telegram channel. A future NyxID mobile app will register device tokens and receive real-time approval request notifications.

**Key design goals:**
- Multi-channel delivery: Telegram + FCM + APNs fire in parallel; at least one success is sufficient
- Multiple devices per user (e.g., iPhone + iPad + Android phone)
- Automatic stale token cleanup when FCM/APNs reports tokens as invalid
- No sensitive data in push payloads (approval details fetched via API after tap)
- Follows existing codebase patterns exactly: `reqwest` HTTP calls (no heavy SDKs), layered architecture, dedicated response structs

---

## 1. Data Model Changes

### 1.1 Device Token Sub-Document

**File:** `backend/src/models/notification_channel.rs`

Add a `DeviceToken` sub-document struct and a `push_devices` array field to `NotificationChannel`. Device tokens are stored directly on the user's notification channel document (not a separate collection) to keep the model co-located with other notification preferences.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceToken {
    /// Unique device ID (UUID v4 string, generated server-side)
    pub device_id: String,

    /// Platform: "fcm" or "apns"
    pub platform: String,

    /// The device registration token from FCM or APNs
    pub token: String,

    /// Human-readable device name (e.g. "iPhone 15", "Pixel 8")
    pub device_name: Option<String>,

    /// App bundle ID / package name (e.g. "dev.nyxid.app")
    /// Used for APNs apns-topic header.
    pub app_id: Option<String>,

    /// When the token was registered or last refreshed
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub registered_at: DateTime<Utc>,

    /// When a push was last successfully sent to this token
    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
}
```

New fields on `NotificationChannel`:

```rust
pub struct NotificationChannel {
    // ... existing fields ...

    // -- Push Notifications --
    /// Whether push notifications (FCM/APNs) are enabled
    #[serde(default)]
    pub push_enabled: bool,

    /// Registered device tokens for push notifications
    #[serde(default)]
    pub push_devices: Vec<DeviceToken>,
}
```

**Why sub-document array instead of a separate collection:**
- A user will have at most ~5 devices; this never grows unboundedly
- Atomic updates: adding/removing a device token is a single `$push`/`$pull` on the existing document
- Co-located with `telegram_enabled`, `push_enabled`, and other notification preferences
- Avoids an extra collection and index management overhead
- Consistent with MongoDB document-oriented design for small, bounded arrays

### 1.2 Defaults for New Fields

Update `get_or_create_channel` in `notification_service.rs` to initialize:
```rust
push_enabled: false,
push_devices: vec![],
```

### 1.3 Test Updates

Update `make_notification_channel()` in model tests to include the new fields. Add a BSON roundtrip test verifying `push_devices` serialization with nested `DeviceToken` structs.

---

## 2. Configuration Changes

### 2.1 New Fields in `AppConfig`

**File:** `backend/src/config.rs`

```rust
pub struct AppConfig {
    // ... existing fields ...

    // -- FCM (Firebase Cloud Messaging) --
    /// Path to FCM service account JSON file.
    /// Required for FCM push notifications.
    pub fcm_service_account_path: Option<String>,

    /// FCM project ID (extracted from service account JSON at startup).
    /// Not set directly via env var -- derived from the JSON file.
    pub fcm_project_id: Option<String>,

    // -- APNs (Apple Push Notification service) --
    /// Path to APNs .p8 private key file.
    pub apns_key_path: Option<String>,

    /// APNs Key ID (from Apple Developer portal).
    pub apns_key_id: Option<String>,

    /// APNs Team ID (from Apple Developer portal).
    pub apns_team_id: Option<String>,

    /// APNs topic (bundle ID of the iOS app, e.g. "dev.nyxid.app").
    pub apns_topic: Option<String>,

    /// Use APNs sandbox (api.sandbox.push.apple.com) instead of production.
    /// Default: true in development, false otherwise.
    pub apns_sandbox: bool,
}
```

### 2.2 Environment Variables

```bash
# FCM (optional)
FCM_SERVICE_ACCOUNT_PATH=keys/fcm-service-account.json

# APNs (optional)
APNS_KEY_PATH=keys/apns-auth-key.p8
APNS_KEY_ID=ABC123DEFG
APNS_TEAM_ID=TEAMID1234
APNS_TOPIC=dev.nyxid.app
APNS_SANDBOX=true   # default: true in dev, false in production
```

### 2.3 Startup Validation

At startup, if `FCM_SERVICE_ACCOUNT_PATH` is set:
1. Read and parse the JSON file
2. Extract `project_id` and store in `config.fcm_project_id`
3. Validate that `client_email` and `private_key` fields exist
4. Log: `"FCM push notifications enabled (project: {project_id})"`

If `APNS_KEY_PATH` is set:
1. Verify the .p8 file exists and is readable
2. Verify `APNS_KEY_ID` and `APNS_TEAM_ID` are also set (panic if not)
3. Log: `"APNs push notifications enabled (team: {team_id})"`

### 2.4 Test Config Update

Add the new fields to `make_config()` in config tests with `None`/default values.

---

## 3. Push Service Implementation

### 3.1 New File: `backend/src/services/push_service.rs`

This service follows the same pattern as `telegram_service.rs`: stateless functions that take `&reqwest::Client` and config parameters, returning `AppResult<T>`.

#### 3.1.1 FCM Implementation

**Authentication:** FCM HTTP v1 API requires a short-lived OAuth2 access token derived from a Google service account. We implement the JWT assertion flow manually (no SDK dependency):

1. Read service account JSON at startup
2. Create a JWT assertion: `{"iss": client_email, "scope": "https://www.googleapis.com/auth/firebase.messaging", "aud": "https://oauth2.googleapis.com/token", "iat": now, "exp": now + 3600}`
3. Sign with RS256 using the service account's private key
4. POST to `https://oauth2.googleapis.com/token` with `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer&assertion=<jwt>`
5. Cache the access token until `exp - 60s` (refresh 1 minute before expiry)

**Token caching:** Use `tokio::sync::RwLock<Option<CachedToken>>` where `CachedToken` holds the access token and its expiry time. The lock is taken for read on every send; write only when refreshing.

```rust
struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

struct FcmAuth {
    service_account: FcmServiceAccount,
    cached_token: tokio::sync::RwLock<Option<CachedToken>>,
}
```

**Send function:**

```rust
pub async fn send_fcm_notification(
    http_client: &Client,
    fcm_auth: &FcmAuth,
    project_id: &str,
    device_token: &str,
    title: &str,
    body: &str,
    data: &HashMap<String, String>,
) -> AppResult<FcmSendResult>
```

**Request format:**
```
POST https://fcm.googleapis.com/v1/projects/{project_id}/messages:send
Authorization: Bearer {access_token}
Content-Type: application/json

{
  "message": {
    "token": "{device_token}",
    "notification": {
      "title": "Approval Required",
      "body": "A service is requesting access"
    },
    "data": {
      "request_id": "...",
      "type": "approval_request"
    },
    "android": {
      "priority": "high",
      "notification": {
        "channel_id": "approvals",
        "sound": "default"
      }
    }
  }
}
```

**Error handling:**
| HTTP Status | Action |
|---|---|
| 200 | Success |
| 400 (`INVALID_ARGUMENT`) | Log error, do not retry (bad payload) |
| 401 | Refresh OAuth2 token, retry once |
| 404 (`UNREGISTERED`) | **Remove device token from user's push_devices** |
| 429 | Log warning, do not retry (rate limited) |
| 500/503 | Log warning, do not retry (server error) |

**Result type:**
```rust
pub enum FcmSendResult {
    Success { message_name: String },
    TokenInvalid,  // 404 UNREGISTERED -- caller should remove token
    Failed { reason: String },
}
```

#### 3.1.2 APNs Implementation

**Authentication:** APNs uses provider JWT tokens signed with ES256 using the .p8 key. We use the existing `jsonwebtoken` crate (already in Cargo.toml).

1. Read .p8 key file at startup
2. Create JWT: header `{"alg": "ES256", "kid": "{key_id}"}`, payload `{"iss": "{team_id}", "iat": now}`
3. Sign with ES256 using `jsonwebtoken::encode`
4. Token valid for up to 1 hour; cache and refresh at `iat + 50min`

**Token caching:** Same `RwLock<Option<CachedToken>>` pattern as FCM.

```rust
struct ApnsAuth {
    encoding_key: jsonwebtoken::EncodingKey,
    key_id: String,
    team_id: String,
    cached_token: tokio::sync::RwLock<Option<CachedToken>>,
}
```

**Send function:**

```rust
pub async fn send_apns_notification(
    http_client: &Client,
    apns_auth: &ApnsAuth,
    device_token: &str,
    topic: &str,
    sandbox: bool,
    title: &str,
    body: &str,
    data: &HashMap<String, String>,
) -> AppResult<ApnsSendResult>
```

**Request format:**
```
POST https://api.push.apple.com/3/device/{device_token}
Authorization: Bearer {jwt}
apns-topic: dev.nyxid.app
apns-push-type: alert
apns-priority: 10
apns-expiration: 0
Content-Type: application/json

{
  "aps": {
    "alert": {
      "title": "Approval Required",
      "body": "A service is requesting access"
    },
    "sound": "default",
    "badge": 1,
    "mutable-content": 1,
    "category": "APPROVAL_REQUEST"
  },
  "request_id": "...",
  "type": "approval_request"
}
```

**Error handling:**
| HTTP Status | Reason | Action |
|---|---|---|
| 200 | Success | Return success |
| 400 | `BadDeviceToken` | **Remove device token** |
| 403 | `ExpiredProviderToken` | Refresh JWT, retry once |
| 410 | `Unregistered` | **Remove device token** |
| 429 | `TooManyRequests` | Log warning, do not retry |
| 500 | `InternalServerError` | Log warning, do not retry |

**Result type:**
```rust
pub enum ApnsSendResult {
    Success,
    TokenInvalid,  // 400 BadDeviceToken or 410 Unregistered
    Failed { reason: String },
}
```

#### 3.1.3 HTTP/2 for APNs

The existing `reqwest` dependency already has `rustls-tls` enabled. `reqwest` supports HTTP/2 natively via its `rustls-tls` feature. APNs requires HTTP/2, which `reqwest` handles automatically when connecting to `api.push.apple.com` (ALPN negotiation).

No additional dependencies needed. We do NOT use `http2_prior_knowledge()` since APNs expects standard TLS+ALPN negotiation.

#### 3.1.4 Push Auth in AppState

**File:** `backend/src/main.rs`

Add optional push auth to `AppState`:

```rust
pub struct AppState {
    // ... existing fields ...
    pub fcm_auth: Option<Arc<push_service::FcmAuth>>,
    pub apns_auth: Option<Arc<push_service::ApnsAuth>>,
}
```

Initialize at startup if config is present:

```rust
let fcm_auth = if let Some(path) = &config.fcm_service_account_path {
    Some(Arc::new(push_service::FcmAuth::from_service_account_file(path)?))
} else {
    None
};

let apns_auth = if let Some(path) = &config.apns_key_path {
    Some(Arc::new(push_service::ApnsAuth::new(
        path,
        config.apns_key_id.as_deref().expect("APNS_KEY_ID required"),
        config.apns_team_id.as_deref().expect("APNS_TEAM_ID required"),
    )?))
} else {
    None
};
```

---

## 4. Notification Service Changes

### 4.1 Multi-Channel Delivery

**File:** `backend/src/services/notification_service.rs`

Update `send_approval_notification` to deliver via all enabled channels in parallel:

```rust
pub async fn send_approval_notification(
    db: &Database,
    config: &AppConfig,
    http_client: &Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    user_id: &str,
    request: &ApprovalRequest,
) -> AppResult<NotificationResult> {
    let channel = get_or_create_channel(db, user_id).await?;

    let mut channels_used: Vec<String> = Vec::new();
    let mut telegram_chat_id = None;
    let mut telegram_message_id = None;
    let mut tokens_to_remove: Vec<String> = Vec::new();

    // 1. Telegram (existing behavior, unchanged)
    if channel.telegram_enabled {
        if let Some(chat_id) = channel.telegram_chat_id {
            match send_telegram(config, http_client, chat_id, request, &channel).await {
                Ok((cid, mid)) => {
                    channels_used.push("telegram".to_string());
                    telegram_chat_id = Some(cid);
                    telegram_message_id = Some(mid);
                }
                Err(e) => tracing::warn!("Telegram notification failed: {e}"),
            }
        }
    }

    // 2. Push notifications (FCM + APNs)
    if channel.push_enabled && !channel.push_devices.is_empty() {
        let push_futures: Vec<_> = channel.push_devices.iter().map(|device| {
            send_push_to_device(
                http_client, fcm_auth, apns_auth, config, device, request,
            )
        }).collect();

        let results = futures::future::join_all(push_futures).await;

        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(PushResult::Success) => {
                    let platform = &channel.push_devices[i].platform;
                    if !channels_used.contains(platform) {
                        channels_used.push(platform.clone());
                    }
                }
                Ok(PushResult::TokenInvalid) => {
                    tokens_to_remove.push(
                        channel.push_devices[i].device_id.clone()
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        device_id = %channel.push_devices[i].device_id,
                        "Push notification failed: {e}"
                    );
                }
            }
        }
    }

    // 3. Remove invalid tokens (fire-and-forget)
    if !tokens_to_remove.is_empty() {
        let db = db.clone();
        let channel_id = channel.id.clone();
        tokio::spawn(async move {
            remove_stale_device_tokens(&db, &channel_id, &tokens_to_remove).await;
        });
    }

    if channels_used.is_empty() {
        return Err(AppError::BadRequest(
            "No notification channel is configured and enabled".to_string(),
        ));
    }

    Ok(NotificationResult {
        channels: channels_used,
        telegram_chat_id,
        telegram_message_id,
    })
}
```

**Return type change:**

```rust
pub struct NotificationResult {
    pub channels: Vec<String>,
    pub telegram_chat_id: Option<i64>,
    pub telegram_message_id: Option<i64>,
}
```

The `approval_service.rs` call site updates from `(String, Option<i64>, Option<i64>)` to `NotificationResult`.

### 4.2 Push Payload Design (Security)

Push notification payloads MUST NOT contain sensitive information:

```
Title: "Approval Required"
Body:  "A service is requesting access"
Data:  { "request_id": "...", "type": "approval_request" }
```

The mobile app must:
1. Receive the push notification
2. Extract `request_id` from the data payload
3. Call `GET /api/v1/approvals/requests/{request_id}/status` to fetch full details
4. Display the approval UI with service name, requester, operation summary

This ensures that even if the push payload is intercepted, no operational details are exposed.

### 4.3 Stale Token Removal

```rust
async fn remove_stale_device_tokens(
    db: &Database,
    channel_id: &str,
    device_ids: &[String],
) {
    let result = db
        .collection::<NotificationChannel>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": channel_id },
            doc! {
                "$pull": {
                    "push_devices": {
                        "device_id": { "$in": device_ids }
                    }
                }
            },
        )
        .await;

    if let Err(e) = result {
        tracing::warn!("Failed to remove stale device tokens: {e}");
    }
}
```

### 4.4 Notify Decision Changes

Update `notify_decision` to send silent push notifications when a decision is made (so the app can refresh its UI):

```rust
pub async fn notify_decision(
    config: &AppConfig,
    http_client: &Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    db: &Database,
    request: &ApprovalRequest,
    approved: bool,
) -> AppResult<()> {
    // 1. Edit Telegram message (existing behavior)
    // ...

    // 2. Send silent push to update mobile app UI
    let channel = get_or_create_channel(db, &request.user_id).await?;
    if channel.push_enabled {
        for device in &channel.push_devices {
            let _ = send_silent_push(
                http_client, fcm_auth, apns_auth, config, device,
                &request.id, approved,
            ).await;
        }
    }

    Ok(())
}
```

---

## 5. New API Endpoints

### 5.1 Device Token Management

**File:** `backend/src/handlers/device_tokens.rs` (new file)

Three endpoints for the mobile app to manage device tokens:

#### POST /api/v1/notifications/devices -- Register a Device Token

**Request:**
```json
{
  "platform": "fcm",
  "token": "dGVzdC1kZXZpY2UtdG9rZW4...",
  "device_name": "iPhone 15 Pro",
  "app_id": "dev.nyxid.app"
}
```

**Validation:**
- `platform`: Must be `"fcm"` or `"apns"` (reject anything else)
- `token`: Required, non-empty string, max 4096 chars (FCM tokens can be ~163 chars, APNs are 64 hex chars)
- `device_name`: Optional, max 100 chars
- `app_id`: Required for APNs (used as `apns-topic`), optional for FCM

**Behavior:**
1. Get or create the user's notification channel
2. Check if a device with the same `token` already exists:
   - If yes: update `registered_at`, `device_name`, `app_id` (token refresh)
   - If no: add new `DeviceToken` to `push_devices` array
3. Enforce max 10 devices per user (reject with 400 if exceeded)
4. Auto-enable `push_enabled = true` when first device is registered

**Response (201 Created or 200 OK):**
```json
{
  "device_id": "550e8400-e29b-41d4-a716-446655440000",
  "platform": "fcm",
  "device_name": "iPhone 15 Pro",
  "registered_at": "2026-03-03T12:00:00Z"
}
```

**MongoDB operation (upsert existing token):**
```rust
// Try to update existing token first
let result = collection.update_one(
    doc! {
        "_id": &channel.id,
        "push_devices.token": &body.token,
    },
    doc! {
        "$set": {
            "push_devices.$.registered_at": bson::DateTime::from_chrono(now),
            "push_devices.$.device_name": &body.device_name,
            "push_devices.$.app_id": &body.app_id,
            "updated_at": bson::DateTime::from_chrono(now),
        }
    },
).await?;

if result.matched_count == 0 {
    // New token -- push to array
    collection.update_one(
        doc! { "_id": &channel.id },
        doc! {
            "$push": {
                "push_devices": bson::to_bson(&device_token)?
            },
            "$set": {
                "push_enabled": true,
                "updated_at": bson::DateTime::from_chrono(now),
            }
        },
    ).await?;
}
```

#### GET /api/v1/notifications/devices -- List Registered Devices

**Response (200 OK):**
```json
{
  "devices": [
    {
      "device_id": "550e8400-e29b-41d4-a716-446655440000",
      "platform": "fcm",
      "device_name": "iPhone 15 Pro",
      "registered_at": "2026-03-03T12:00:00Z",
      "last_used_at": "2026-03-03T14:30:00Z"
    },
    {
      "device_id": "660e8400-e29b-41d4-a716-446655440001",
      "platform": "apns",
      "device_name": "iPad Air",
      "registered_at": "2026-03-01T08:00:00Z",
      "last_used_at": null
    }
  ],
  "push_enabled": true
}
```

Note: The actual device `token` is NOT returned in the response (it is a secret credential between the device and push service).

#### DELETE /api/v1/notifications/devices/{device_id} -- Remove a Device

**Behavior:**
1. Remove the device token from `push_devices` array via `$pull`
2. If `push_devices` becomes empty, auto-disable `push_enabled = false`
3. Audit log the removal

**Response (200 OK):**
```json
{
  "message": "Device removed"
}
```

**MongoDB operation:**
```rust
collection.update_one(
    doc! { "_id": &channel.id, "user_id": &user_id },
    doc! {
        "$pull": {
            "push_devices": { "device_id": &device_id }
        },
        "$set": { "updated_at": bson::DateTime::from_chrono(now) }
    },
).await?;

// If no devices left, disable push
let updated = get_or_create_channel(db, &user_id).await?;
if updated.push_devices.is_empty() {
    collection.update_one(
        doc! { "_id": &channel.id },
        doc! { "$set": { "push_enabled": false } },
    ).await?;
}
```

### 5.2 Settings Endpoint Updates

**File:** `backend/src/handlers/notifications.rs`

Update `NotificationSettingsResponse` to include push notification status:

```rust
pub struct NotificationSettingsResponse {
    // ... existing fields ...
    pub push_enabled: bool,
    pub push_device_count: usize,
}
```

Update `UpdateNotificationSettingsRequest` to allow toggling:

```rust
pub struct UpdateNotificationSettingsRequest {
    // ... existing fields ...
    pub push_enabled: Option<bool>,
}
```

Validation: Cannot enable push if `push_devices` is empty.

---

## 6. Route Registration

### 6.1 New Routes

**File:** `backend/src/routes.rs`

Add device token routes under the existing notification routes in the `api_v1_human_only` section:

```rust
let notification_routes = Router::new()
    .route(
        "/settings",
        get(handlers::notifications::get_settings)
            .put(handlers::notifications::update_settings),
    )
    .route(
        "/telegram/link",
        post(handlers::notifications::telegram_link),
    )
    .route(
        "/telegram",
        delete(handlers::notifications::telegram_disconnect),
    )
    // NEW: Device token management
    .route(
        "/devices",
        get(handlers::device_tokens::list_devices)
            .post(handlers::device_tokens::register_device),
    )
    .route(
        "/devices/{device_id}",
        delete(handlers::device_tokens::remove_device),
    );
```

These routes are in `api_v1_human_only` -- they require a regular user JWT (not service account or delegated tokens). The mobile app authenticates the same way as the web frontend: standard JWT in the `Authorization` header.

### 6.2 Handler Module Registration

**File:** `backend/src/handlers/mod.rs`

Add: `pub mod device_tokens;`

### 6.3 Service Module Registration

**File:** `backend/src/services/mod.rs`

Add: `pub mod push_service;`

---

## 7. Database Index Changes

### 7.1 No New Indexes Needed

Device tokens are stored in a sub-document array on `notification_channels`, which already has:
- Unique index on `user_id` (one channel doc per user)
- Sparse index on `telegram_link_code`
- Sparse index on `telegram_chat_id`

Since device lookups are always by `user_id` (which hits the existing unique index), no additional indexes are required.

The `push_devices.token` field is only queried within a single document (positional `$` update), which does not benefit from a collection-level index.

---

## 8. Error Handling

### 8.1 No New AppError Variants

All push notification errors map to existing variants:
- Invalid input (bad platform, empty token) -> `AppError::ValidationError`
- Push service unavailable -> `AppError::Internal` (logged but not surfaced to notification caller)
- Device limit exceeded -> `AppError::BadRequest`
- Device not found for deletion -> `AppError::NotFound`

Push notification delivery failures are **non-blocking**: if push fails but Telegram succeeds (or vice versa), the approval request is still created and the user can approve via any available channel including the web UI.

---

## 9. Dependency Changes

### 9.1 Cargo.toml

No new crate dependencies required:
- **`reqwest`** (already present) -- HTTP client for FCM and APNs API calls
- **`jsonwebtoken`** (already present) -- ES256 JWT for APNs auth; RS256 JWT assertion for FCM OAuth2
- **`serde_json`** (already present) -- JSON payload construction
- **`base64`** (already present) -- Encoding for JWT
- **`tokio`** (already present) -- `RwLock` for token caching, `spawn` for async cleanup

The FCM OAuth2 flow uses a self-signed JWT assertion (RS256) to obtain an access token from Google's token endpoint. This is implemented manually using `jsonwebtoken` and `reqwest` to avoid pulling in `google-auth` or `yup-oauth2` SDKs.

---

## 10. Security Considerations

### 10.1 Push Payload Security

Push payloads contain ONLY:
- `request_id` (UUID -- not guessable, not sensitive)
- `type` field ("approval_request")
- Generic title/body text ("Approval Required" / "A service is requesting access")

No service names, requester IDs, operation summaries, or any operational data appears in the push payload. The mobile app must authenticate and call the API to fetch full details.

### 10.2 Device Token Security

- Device tokens are stored in MongoDB (not encrypted) -- they are ephemeral identifiers issued by Google/Apple and are useless without the corresponding FCM/APNs server credentials
- Device tokens are NEVER returned in API responses (only `device_id`, `platform`, `device_name`, and timestamps)
- Each user can only manage their own device tokens (enforced by `AuthUser` middleware)
- Maximum 10 devices per user prevents abuse

### 10.3 FCM/APNs Credential Security

- FCM service account JSON file: stored on disk (not in env var), path configured via env var
- APNs .p8 key file: stored on disk, path configured via env var
- Neither credential is stored in MongoDB or logged
- Access tokens are cached in-memory only (not persisted)

### 10.4 Token Invalidation

When a user logs out of the mobile app or uninstalls it:
1. The app should call `DELETE /api/v1/notifications/devices/{device_id}` to deregister
2. If the app doesn't deregister, FCM/APNs will report the token as invalid on the next send attempt
3. The `push_service` automatically removes invalid tokens via `remove_stale_device_tokens`

---

## 11. Approval Service Integration Changes

### 11.1 Updated `create_approval_request` Signature

**File:** `backend/src/services/approval_service.rs`

The `notification_service::send_approval_notification` call needs the new auth parameters:

```rust
match notification_service::send_approval_notification(
    db,
    config,
    http_client,
    fcm_auth.as_deref(),   // Option<&FcmAuth>
    apns_auth.as_deref(),  // Option<&ApnsAuth>
    user_id,
    &request,
).await {
    Ok(result) => {
        let update = doc! {
            "$set": {
                "notification_channel": result.channels.join(","),
                "telegram_chat_id": result.telegram_chat_id,
                "telegram_message_id": result.telegram_message_id,
            }
        };
        // ...
    }
    // ...
}
```

The `notification_channel` field on `ApprovalRequest` changes from storing a single channel name (e.g., `"telegram"`) to a comma-separated list (e.g., `"telegram,fcm,apns"`). This is backward-compatible: existing logic checks `notification_channel.as_deref() == Some("telegram")` which still works if the value is just `"telegram"`.

### 11.2 Updated `process_decision` and `expire_pending_requests`

Both functions call `notify_decision` which now needs `fcm_auth` and `apns_auth`. These are passed through from the caller (handler or background task).

The background expiry task in `main.rs` needs access to `AppState` (which already has `fcm_auth` and `apns_auth`).

---

## 12. Frontend/Mobile API Contract Summary

### 12.1 Endpoints for Mobile App Team

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/api/v1/notifications/devices` | JWT | Register device token |
| `GET` | `/api/v1/notifications/devices` | JWT | List registered devices |
| `DELETE` | `/api/v1/notifications/devices/{device_id}` | JWT | Remove device |
| `GET` | `/api/v1/notifications/settings` | JWT | Get notification settings (updated) |
| `PUT` | `/api/v1/notifications/settings` | JWT | Update settings (push_enabled toggle) |
| `GET` | `/api/v1/approvals/requests/{request_id}/status` | JWT | Get approval request details (existing) |
| `POST` | `/api/v1/approvals/requests/{request_id}/decide` | JWT | Approve/reject (existing) |
| `GET` | `/api/v1/approvals/requests` | JWT | List approval history (existing) |
| `GET` | `/api/v1/approvals/grants` | JWT | List active grants (existing) |

### 12.2 Push Notification Payload Schema

**Alert notification (approval request):**
```json
{
  "title": "Approval Required",
  "body": "A service is requesting access",
  "data": {
    "type": "approval_request",
    "request_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Silent notification (decision made):**
```json
{
  "data": {
    "type": "approval_decision",
    "request_id": "550e8400-e29b-41d4-a716-446655440000",
    "decision": "approved"
  }
}
```

### 12.3 Mobile App Responsibilities

1. **On app launch:** Call `POST /api/v1/notifications/devices` with the current FCM/APNs token
2. **On token refresh:** Call `POST /api/v1/notifications/devices` again (upserts automatically)
3. **On notification tap:** Extract `request_id`, navigate to approval screen, call status API
4. **On logout/uninstall:** Call `DELETE /api/v1/notifications/devices/{device_id}`
5. **Authentication:** Standard NyxID JWT flow (login, refresh token, etc.)

---

## 13. Implementation Phases

### Phase 1: Model + Config + Push Service (Foundation)
1. Update `NotificationChannel` model with `DeviceToken`, `push_enabled`, `push_devices`
2. Add new config fields and env var parsing
3. Implement `push_service.rs` with FCM and APNs send functions
4. Add `FcmAuth` and `ApnsAuth` to `AppState`
5. Write unit tests for push_service (mock HTTP responses)

### Phase 2: Device Token API (Mobile Contract)
1. Create `handlers/device_tokens.rs` with register/list/remove handlers
2. Register routes in `routes.rs`
3. Update notification settings response/request structs
4. Write integration tests for device token CRUD

### Phase 3: Multi-Channel Delivery (Core Integration)
1. Update `notification_service::send_approval_notification` for multi-channel
2. Update `notify_decision` for push-based decision updates
3. Update `approval_service.rs` to pass push auth through
4. Update `main.rs` background tasks
5. Write integration tests for multi-channel delivery

### Phase 4: Testing + Hardening
1. End-to-end test: register device -> create approval -> receive push -> decide
2. Token invalidation test: send to invalid token -> verify removal
3. Graceful degradation test: FCM down -> Telegram still works
4. Load test: many devices per user
5. Security review of push payloads

---

## 14. Files Changed Summary

| File | Change Type | Description |
|------|------------|-------------|
| `models/notification_channel.rs` | Modified | Add `DeviceToken`, `push_enabled`, `push_devices` |
| `services/push_service.rs` | **New** | FCM + APNs send functions, auth token caching |
| `services/notification_service.rs` | Modified | Multi-channel delivery, stale token removal |
| `services/approval_service.rs` | Modified | Pass push auth to notification_service |
| `services/mod.rs` | Modified | Add `pub mod push_service` |
| `handlers/device_tokens.rs` | **New** | Register/list/remove device token handlers |
| `handlers/notifications.rs` | Modified | Add push fields to settings response |
| `handlers/mod.rs` | Modified | Add `pub mod device_tokens` |
| `config.rs` | Modified | FCM/APNs config fields |
| `routes.rs` | Modified | Register device token routes |
| `main.rs` | Modified | Initialize FcmAuth/ApnsAuth, add to AppState |
| `errors/mod.rs` | Unchanged | No new variants needed |
| `db.rs` | Unchanged | No new indexes needed |
