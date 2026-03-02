# Approval System Architecture Plan

## Overview

The approval system adds push-based transaction approval to NyxID. When a downstream service is accessed through the proxy or LLM gateway using user credentials, NyxID can require explicit user approval before forwarding the request. Approval notifications are sent via Telegram (with the architecture designed for future mobile app push notifications via FCM/APNs).

**Key design goals:**
- Non-blocking: proxy requests that need approval return `403 approval_required` immediately; callers poll or retry after approval
- Cached grants: once approved, access is granted for a configurable period (default 30 days) without re-prompting
- Extensible: notification delivery is behind a trait so Telegram, mobile push, and future channels share a common interface
- Secure: idempotency keys, replay prevention, constant-time webhook verification, cryptographically bound approval context

---

## 1. Data Model

### 1.1 New Collection: `approval_requests`

**File:** `backend/src/models/approval_request.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "approval_requests";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// UUID v4 string
    #[serde(rename = "_id")]
    pub id: String,

    /// The user who must approve this request
    pub user_id: String,

    /// The downstream service being accessed
    pub service_id: String,

    /// Human-readable service name (denormalized for Telegram message)
    pub service_name: String,

    /// The service slug (denormalized for display)
    pub service_slug: String,

    /// Who is making the request: "user" (direct proxy), "service_account",
    /// or "delegated" (OAuth client acting on behalf)
    pub requester_type: String,

    /// ID of the requester (user_id, service_account_id, or client_id)
    pub requester_id: String,

    /// Human-readable requester label (e.g. SA name, OAuth client name)
    pub requester_label: Option<String>,

    /// What operation is being performed (e.g. "proxy:GET /v1/chat/completions")
    pub operation_summary: String,

    /// "pending" | "approved" | "rejected" | "expired"
    pub status: String,

    /// Client-provided idempotency key to prevent duplicate approval requests.
    /// Format: SHA-256 of (user_id + service_id + requester_id + requester_type).
    /// If a pending/approved request with this key exists, return it instead
    /// of creating a new one.
    pub idempotency_key: String,

    /// Which notification channel delivered this request (e.g. "telegram")
    pub notification_channel: Option<String>,

    /// Telegram message_id for editing the message after decision
    pub telegram_message_id: Option<i64>,

    /// Telegram chat_id where the notification was sent
    pub telegram_chat_id: Option<i64>,

    /// When the approval request expires (auto-reject after this time)
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,

    /// When the user made their decision (approved/rejected)
    #[serde(default, with = "bson_datetime::optional")]
    pub decided_at: Option<DateTime<Utc>>,

    /// Channel through which the decision was made (e.g. "telegram", "web")
    pub decision_channel: Option<String>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
```

**Indexes** (added to `db.rs` `ensure_indexes()`):

```rust
// -- approval_requests --
let approval_requests = db.collection::<Document>("approval_requests");

// Fast lookup by user + status (for pending requests)
approval_requests.create_index(
    IndexModel::builder()
        .keys(doc! { "user_id": 1, "status": 1 })
        .build(),
).await?;

// Idempotency: find existing request by key
approval_requests.create_index(
    IndexModel::builder()
        .keys(doc! { "idempotency_key": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
).await?;

// TTL: auto-delete old requests after 90 days
approval_requests.create_index(
    IndexModel::builder()
        .keys(doc! { "created_at": 1 })
        .options(
            IndexOptions::builder()
                .expire_after(Duration::from_secs(90 * 24 * 60 * 60))
                .build(),
        )
        .build(),
).await?;
```

### 1.2 New Collection: `approval_grants`

Cached approval decisions that allow subsequent requests without re-prompting.

**File:** `backend/src/models/approval_grant.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "approval_grants";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalGrant {
    /// UUID v4 string
    #[serde(rename = "_id")]
    pub id: String,

    /// The user who granted approval
    pub user_id: String,

    /// The service this grant applies to
    pub service_id: String,

    /// Who was granted access (requester_type + requester_id pair)
    pub requester_type: String,
    pub requester_id: String,

    /// The approval_request._id that created this grant
    pub approval_request_id: String,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub granted_at: DateTime<Utc>,

    /// When this grant expires (user-configurable, default 30 days)
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,

    /// Whether this grant has been explicitly revoked
    #[serde(default)]
    pub revoked: bool,
}
```

**Indexes:**

```rust
// -- approval_grants --
let approval_grants = db.collection::<Document>("approval_grants");

// Primary lookup: does this requester have a valid grant for this service?
approval_grants.create_index(
    IndexModel::builder()
        .keys(doc! {
            "user_id": 1,
            "service_id": 1,
            "requester_type": 1,
            "requester_id": 1,
        })
        .build(),
).await?;

// TTL: auto-delete expired grants
approval_grants.create_index(
    IndexModel::builder()
        .keys(doc! { "expires_at": 1 })
        .options(
            IndexOptions::builder()
                .expire_after(Duration::from_secs(0))
                .build(),
        )
        .build(),
).await?;

// List grants for a user (settings page)
approval_grants.create_index(
    IndexModel::builder()
        .keys(doc! { "user_id": 1, "granted_at": -1 })
        .build(),
).await?;
```

### 1.3 New Collection: `notification_channels`

User notification preferences and connected messaging accounts.

**File:** `backend/src/models/notification_channel.rs`

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "notification_channels";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationChannel {
    /// UUID v4 string
    #[serde(rename = "_id")]
    pub id: String,

    /// Owner user ID (unique per user -- one notification config per user)
    pub user_id: String,

    // -- Telegram --
    /// Telegram chat ID for sending messages
    pub telegram_chat_id: Option<i64>,

    /// Telegram username (for display in settings UI)
    pub telegram_username: Option<String>,

    /// Whether Telegram notifications are enabled
    #[serde(default)]
    pub telegram_enabled: bool,

    /// One-time linking code for connecting Telegram account.
    /// User sends this code to the NyxID Telegram bot to link.
    pub telegram_link_code: Option<String>,

    /// Expiry for the link code (5 minutes)
    #[serde(default, with = "bson_datetime::optional")]
    pub telegram_link_code_expires_at: Option<DateTime<Utc>>,

    // -- Future: Mobile Push --
    // pub fcm_device_tokens: Vec<String>,
    // pub apns_device_tokens: Vec<String>,
    // pub push_enabled: bool,

    // -- User preferences --
    /// How long to wait for user response before auto-rejecting (seconds).
    /// Default: 30. Min: 10. Max: 300.
    #[serde(default = "default_approval_timeout")]
    pub approval_timeout_secs: u32,

    /// How many days an approval grant lasts before re-prompting.
    /// Default: 30. Min: 1. Max: 365.
    #[serde(default = "default_grant_expiry_days")]
    pub grant_expiry_days: u32,

    /// Whether approval is required for proxy/LLM requests using this user's
    /// credentials. When false, all requests are auto-approved (legacy behavior).
    #[serde(default)]
    pub approval_required: bool,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_approval_timeout() -> u32 {
    30
}

fn default_grant_expiry_days() -> u32 {
    30
}
```

**Indexes:**

```rust
// -- notification_channels --
let notification_channels = db.collection::<Document>("notification_channels");

// One config per user
notification_channels.create_index(
    IndexModel::builder()
        .keys(doc! { "user_id": 1 })
        .options(IndexOptions::builder().unique(true).build())
        .build(),
).await?;

// Telegram link code lookup (when bot receives /start <code>)
notification_channels.create_index(
    IndexModel::builder()
        .keys(doc! { "telegram_link_code": 1 })
        .options(IndexOptions::builder().sparse(true).build())
        .build(),
).await?;

// Telegram chat_id lookup (when webhook receives callback)
notification_channels.create_index(
    IndexModel::builder()
        .keys(doc! { "telegram_chat_id": 1 })
        .options(IndexOptions::builder().sparse(true).build())
        .build(),
).await?;
```

### 1.4 Model Registration

Add to `backend/src/models/mod.rs`:

```rust
pub mod approval_grant;
pub mod approval_request;
pub mod notification_channel;
```

---

## 2. Configuration

### 2.1 New Environment Variables

Add to `backend/src/config.rs` `AppConfig`:

```rust
/// Telegram Bot API token for sending approval notifications
pub telegram_bot_token: Option<String>,

/// Secret token for verifying Telegram webhook callbacks.
/// Set via `setWebhook` API with `secret_token` parameter.
pub telegram_webhook_secret: Option<String>,

/// Public URL where Telegram sends webhook callbacks.
/// e.g. https://auth.nyxid.dev/api/v1/webhooks/telegram
pub telegram_webhook_url: Option<String>,
```

Load from env:

```rust
telegram_bot_token: env::var("TELEGRAM_BOT_TOKEN").ok().filter(|s| !s.is_empty()),
telegram_webhook_secret: env::var("TELEGRAM_WEBHOOK_SECRET").ok().filter(|s| !s.is_empty()),
telegram_webhook_url: env::var("TELEGRAM_WEBHOOK_URL").ok().filter(|s| !s.is_empty()),
```

### 2.2 Updated `.env.example`

```bash
# Approval system (optional -- approval disabled when not set)
TELEGRAM_BOT_TOKEN=                # From @BotFather
TELEGRAM_WEBHOOK_SECRET=           # Random string for webhook verification
TELEGRAM_WEBHOOK_URL=              # e.g. https://auth.nyxid.dev/api/v1/webhooks/telegram
```

---

## 3. Backend Services

### 3.1 `approval_service.rs` (core orchestrator)

**File:** `backend/src/services/approval_service.rs`

Responsibilities:
- Check if a valid approval grant exists for a (user, service, requester) triple
- Create approval requests with idempotency
- Process approval/rejection decisions
- Create/revoke approval grants
- Expire timed-out approval requests

```rust
/// Check whether the request has a valid (non-expired, non-revoked) approval grant.
/// Returns Ok(true) if access is granted, Ok(false) if approval is needed.
pub async fn check_approval(
    db: &Database,
    user_id: &str,
    service_id: &str,
    requester_type: &str,
    requester_id: &str,
) -> AppResult<bool>

/// Create an approval request (idempotent via idempotency_key).
/// If a pending request with the same key exists, returns it.
/// Sends notification via the configured channel.
pub async fn create_approval_request(
    db: &Database,
    config: &AppConfig,
    user_id: &str,
    service_id: &str,
    service_name: &str,
    service_slug: &str,
    requester_type: &str,
    requester_id: &str,
    requester_label: Option<&str>,
    operation_summary: &str,
    timeout_secs: u32,
) -> AppResult<ApprovalRequest>

/// Process a user's approval decision (from Telegram callback or web UI).
/// - Atomically updates status from "pending" to "approved"/"rejected"
/// - On approval: creates an ApprovalGrant with the user's configured expiry
/// - Prevents replay: only processes if status == "pending"
/// - Edits the Telegram message to show the decision
pub async fn process_decision(
    db: &Database,
    config: &AppConfig,
    request_id: &str,
    approved: bool,
    decision_channel: &str,
) -> AppResult<ApprovalRequest>

/// List approval requests for a user (for history page).
pub async fn list_requests(
    db: &Database,
    user_id: &str,
    status_filter: Option<&str>,
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<ApprovalRequest>, u64)>

/// List active approval grants for a user.
pub async fn list_grants(
    db: &Database,
    user_id: &str,
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<ApprovalGrant>, u64)>

/// Revoke a specific approval grant.
pub async fn revoke_grant(
    db: &Database,
    user_id: &str,
    grant_id: &str,
) -> AppResult<()>

/// Revoke all grants for a user (e.g. when disabling approval).
pub async fn revoke_all_grants(
    db: &Database,
    user_id: &str,
) -> AppResult<u64>
```

**Idempotency key generation:**

```rust
fn compute_idempotency_key(
    user_id: &str,
    service_id: &str,
    requester_type: &str,
    requester_id: &str,
) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hasher.update(b":");
    hasher.update(service_id.as_bytes());
    hasher.update(b":");
    hasher.update(requester_type.as_bytes());
    hasher.update(b":");
    hasher.update(requester_id.as_bytes());
    hex::encode(hasher.finalize())
}
```

### 3.2 `telegram_service.rs` (Telegram Bot API client)

**File:** `backend/src/services/telegram_service.rs`

Uses raw `reqwest` HTTP calls to the Telegram Bot API (no teloxide dependency).

```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org/bot";

/// Send an approval request message with Approve/Reject inline keyboard.
pub async fn send_approval_message(
    http_client: &Client,
    bot_token: &str,
    chat_id: i64,
    request_id: &str,
    service_name: &str,
    service_slug: &str,
    requester_label: &str,
    operation_summary: &str,
    expires_in_secs: u32,
) -> AppResult<i64>  // returns message_id

/// Edit a message to show the decision result.
pub async fn edit_message_after_decision(
    http_client: &Client,
    bot_token: &str,
    chat_id: i64,
    message_id: i64,
    approved: bool,
    service_name: &str,
) -> AppResult<()>

/// Answer a Telegram callback query (removes loading spinner).
pub async fn answer_callback_query(
    http_client: &Client,
    bot_token: &str,
    callback_query_id: &str,
    text: &str,
) -> AppResult<()>

/// Register the webhook URL with Telegram.
/// Called once during setup via admin endpoint or startup.
pub async fn set_webhook(
    http_client: &Client,
    bot_token: &str,
    webhook_url: &str,
    secret_token: &str,
) -> AppResult<()>
```

**Telegram message format:**

```
Access Request

Service: {service_name} ({service_slug})
Requester: {requester_label}
Action: {operation_summary}
Expires: {expires_in_secs}s

[Approve] [Reject]
```

**Callback data format** (max 64 bytes):

```
a:{request_id_short}   -- approve (first 20 chars of UUID, enough to be unique)
r:{request_id_short}   -- reject
```

Since UUID v4 without hyphens is 32 chars, and callback_data is limited to 64 bytes, we use the full UUID without hyphens (32 chars + 2 prefix = 34 chars, well within limit):

```
a:550e8400e29b41d4a716446655440000
r:550e8400e29b41d4a716446655440000
```

### 3.3 `notification_service.rs` (abstraction layer)

**File:** `backend/src/services/notification_service.rs`

Provides an abstraction over notification channels for future extensibility.

```rust
/// Send an approval notification to the user via their configured channel.
/// Currently only supports Telegram. Returns the channel name used.
pub async fn send_approval_notification(
    db: &Database,
    config: &AppConfig,
    http_client: &Client,
    user_id: &str,
    request: &ApprovalRequest,
) -> AppResult<String>  // returns channel name, e.g. "telegram"

/// Edit the notification message after a decision is made.
pub async fn notify_decision(
    db: &Database,
    config: &AppConfig,
    http_client: &Client,
    request: &ApprovalRequest,
    approved: bool,
) -> AppResult<()>

/// Get the user's notification channel settings, creating defaults if none exist.
pub async fn get_or_create_channel(
    db: &Database,
    user_id: &str,
) -> AppResult<NotificationChannel>
```

---

## 4. API Endpoints

### 4.1 Webhook Endpoint (unauthenticated, signature-verified)

**File:** `backend/src/handlers/webhooks.rs`

```
POST /api/v1/webhooks/telegram
```

This endpoint receives Telegram callback queries when users press Approve/Reject.

**Security:**
- Verified via `X-Telegram-Bot-Api-Secret-Token` header (constant-time comparison)
- No session/JWT auth required (Telegram server-to-server)
- Rate limited by global rate limiter

**Request body** (Telegram Update object -- only `callback_query` is relevant):

```json
{
  "update_id": 123456789,
  "callback_query": {
    "id": "unique_query_id",
    "from": { "id": 12345, "first_name": "User" },
    "message": {
      "message_id": 100,
      "chat": { "id": 12345 }
    },
    "data": "a:550e8400e29b41d4a716446655440000"
  }
}
```

**Handler logic:**

1. Verify `X-Telegram-Bot-Api-Secret-Token` matches config (constant-time)
2. Extract `callback_query.data` -- parse prefix (`a:` or `r:`) and request ID
3. Look up `approval_request` by ID (re-inserting hyphens into UUID)
4. Verify the callback `chat.id` matches the request's `telegram_chat_id`
5. Call `approval_service::process_decision()`
6. Call `telegram_service::answer_callback_query()` to dismiss loading spinner
7. Return `200 OK` (Telegram expects 200 to stop retrying)

**Response:** Always `200 OK` with empty body (webhook acknowledgment).

### 4.2 Notification Settings Endpoints (authenticated, human-only)

**File:** `backend/src/handlers/notifications.rs`

#### Get notification settings

```
GET /api/v1/notifications/settings
```

**Response (200):**
```json
{
  "telegram_connected": true,
  "telegram_username": "@user",
  "telegram_enabled": true,
  "approval_required": true,
  "approval_timeout_secs": 30,
  "grant_expiry_days": 30
}
```

#### Update notification settings

```
PUT /api/v1/notifications/settings
```

**Request:**
```json
{
  "telegram_enabled": true,
  "approval_required": true,
  "approval_timeout_secs": 60,
  "grant_expiry_days": 14
}
```

**Validation:**
- `approval_timeout_secs`: 10..=300
- `grant_expiry_days`: 1..=365

**Response (200):** Same shape as GET response.

#### Generate Telegram link code

```
POST /api/v1/notifications/telegram/link
```

Generates a one-time code that the user sends to the NyxID Telegram bot (`/start <code>`).

**Response (200):**
```json
{
  "link_code": "NYXID-A1B2C3",
  "bot_username": "NyxIDBot",
  "expires_in_secs": 300,
  "instructions": "Send /start NYXID-A1B2C3 to @NyxIDBot on Telegram"
}
```

#### Disconnect Telegram

```
DELETE /api/v1/notifications/telegram
```

Clears `telegram_chat_id`, `telegram_username`, sets `telegram_enabled: false`.

**Response (200):**
```json
{
  "message": "Telegram disconnected"
}
```

### 4.3 Telegram Bot Message Handler (webhook)

When the bot receives a `/start <link_code>` message (not a callback_query), it links the Telegram account:

1. Parse the link code from the message text
2. Look up `notification_channels` by `telegram_link_code`
3. Verify the code hasn't expired
4. Update the document: set `telegram_chat_id`, `telegram_username`, clear `telegram_link_code`
5. Send a confirmation message: "Your Telegram account has been linked to NyxID."

This is handled in the same `POST /api/v1/webhooks/telegram` endpoint -- the handler checks whether the update contains a `callback_query` (approval decision) or a `message` (link command).

### 4.4 Approval Management Endpoints (authenticated, human-only)

**File:** `backend/src/handlers/approvals.rs`

#### List approval requests (history)

```
GET /api/v1/approvals/requests?status=pending&page=1&per_page=20
```

**Response (200):**
```json
{
  "requests": [
    {
      "id": "uuid",
      "service_name": "OpenAI API",
      "service_slug": "openai",
      "requester_type": "service_account",
      "requester_label": "CI Pipeline",
      "operation_summary": "proxy:POST /v1/chat/completions",
      "status": "approved",
      "created_at": "2026-03-03T00:00:00Z",
      "decided_at": "2026-03-03T00:00:05Z",
      "decision_channel": "telegram"
    }
  ],
  "total": 42,
  "page": 1,
  "per_page": 20
}
```

#### List active grants

```
GET /api/v1/approvals/grants?page=1&per_page=20
```

**Response (200):**
```json
{
  "grants": [
    {
      "id": "uuid",
      "service_id": "uuid",
      "service_name": "OpenAI API",
      "requester_type": "service_account",
      "requester_id": "uuid",
      "requester_label": "CI Pipeline",
      "granted_at": "2026-03-03T00:00:00Z",
      "expires_at": "2026-04-02T00:00:00Z"
    }
  ],
  "total": 5,
  "page": 1,
  "per_page": 20
}
```

#### Revoke a grant

```
DELETE /api/v1/approvals/grants/{grant_id}
```

**Response (200):**
```json
{
  "message": "Grant revoked"
}
```

#### Approve/reject via web UI

```
POST /api/v1/approvals/requests/{request_id}/decide
```

**Request:**
```json
{
  "approved": true
}
```

**Response (200):**
```json
{
  "id": "uuid",
  "status": "approved",
  "decided_at": "2026-03-03T00:00:05Z"
}
```

This allows users to approve/reject from the NyxID web dashboard (not just Telegram).

---

## 5. Proxy/LLM Gateway Integration

### 5.1 Approval Check in Proxy Flow

The approval check is added to the proxy handler (`handlers/proxy.rs`) **after** resolving the proxy target but **before** forwarding the request. The approval check is skipped if the downstream service has `auth_method == "none"` or if the user has not enabled approval.

**Modified flow in `execute_proxy()`:**

```rust
// 1. Resolve proxy target (existing)
let target = proxy_service::resolve_proxy_target(...).await?;

// 2. NEW: Check approval if user has it enabled
let requires_approval = approval_service::user_requires_approval(
    &state.db,
    &user_id_str,
).await?;

if requires_approval {
    let requester_type = if auth_user.acting_client_id.is_some() {
        "delegated"
    } else {
        "user"  // direct user access doesn't need approval from self
    };

    // Only require approval for delegated/SA access, not direct user access
    if requester_type != "user" {
        let has_grant = approval_service::check_approval(
            &state.db,
            &user_id_str,
            service_id,
            requester_type,
            auth_user.acting_client_id.as_deref()
                .unwrap_or(&user_id_str),
        ).await?;

        if !has_grant {
            // Get the user's notification settings for timeout
            let channel = notification_service::get_or_create_channel(
                &state.db,
                &user_id_str,
            ).await?;

            // Create and send approval request
            let _request = approval_service::create_approval_request(
                &state.db,
                &state.config,
                &user_id_str,
                service_id,
                &target.service.name,
                &target.service.slug,
                requester_type,
                auth_user.acting_client_id.as_deref()
                    .unwrap_or(&user_id_str),
                None, // TODO: resolve requester label
                &format!("proxy:{} {}", request_method, path),
                channel.approval_timeout_secs,
            ).await?;

            return Err(AppError::Forbidden(
                "Approval required. A notification has been sent to the resource owner.".to_string(),
            ));
        }
    }
}

// 3. Continue with existing proxy logic...
```

### 5.2 New Error Variant

Add to `backend/src/errors/mod.rs`:

```rust
#[error("Approval required")]
ApprovalRequired { request_id: String },
```

With mappings:

```rust
// status_code()
Self::ApprovalRequired { .. } => StatusCode::FORBIDDEN,

// error_code()
Self::ApprovalRequired { .. } => 7000,

// error_key()
Self::ApprovalRequired { .. } => "approval_required",
```

Response body includes `request_id` so callers can poll for the decision:

```json
{
  "error": "approval_required",
  "error_code": 7000,
  "message": "Approval required. A notification has been sent to the resource owner.",
  "request_id": "uuid-of-approval-request"
}
```

### 5.3 LLM Gateway Integration

Same pattern as proxy: add the approval check in `handlers/llm_gateway.rs` in both `llm_proxy_request()` and `gateway_request()`, after resolving the service but before forwarding.

### 5.4 Approval Status Polling Endpoint

For callers that received `approval_required`, provide a polling endpoint:

```
GET /api/v1/approvals/requests/{request_id}/status
```

**Response (200):**
```json
{
  "status": "pending",
  "expires_at": "2026-03-03T00:00:30Z"
}
```

When status changes to "approved", the caller can retry the original proxy request (the grant will now exist).

---

## 6. Route Registration

### 6.1 New Routes in `routes.rs`

```rust
// Webhook (unauthenticated -- verified by secret token)
let webhook_routes = Router::new()
    .route("/telegram", post(handlers::webhooks::telegram_webhook));

// Notification settings (human-only)
let notification_routes = Router::new()
    .route("/settings", get(handlers::notifications::get_settings)
        .put(handlers::notifications::update_settings))
    .route("/telegram/link", post(handlers::notifications::telegram_link))
    .route("/telegram", delete(handlers::notifications::telegram_disconnect));

// Approval management (human-only)
let approval_routes = Router::new()
    .route("/requests", get(handlers::approvals::list_requests))
    .route("/requests/{request_id}/status",
        get(handlers::approvals::get_request_status))
    .route("/requests/{request_id}/decide",
        post(handlers::approvals::decide_request))
    .route("/grants", get(handlers::approvals::list_grants))
    .route("/grants/{grant_id}", delete(handlers::approvals::revoke_grant));
```

**Placement in router hierarchy:**

```rust
// Webhook route -- unauthenticated (outside api_v1 auth middleware)
// Add to the `private` router, alongside /health
let private = Router::new()
    .route("/health", get(handlers::health::health_check))
    .nest("/api/v1/webhooks", webhook_routes)  // NEW
    .nest("/api/v1", api_v1)
    ...

// Notification and approval routes -- authenticated, human-only
// Add to api_v1_human_only
let api_v1_human_only = Router::new()
    .nest("/notifications", notification_routes)  // NEW
    .nest("/approvals", approval_routes)          // NEW
    .nest("/auth", auth_routes)
    ...

// Approval status polling -- accessible by delegated tokens too
// (so service accounts and OAuth clients can poll for approval status)
// Add to api_v1_delegated
let api_v1_delegated = Router::new()
    .route("/approvals/requests/{request_id}/status",
        get(handlers::approvals::get_request_status))  // NEW
    .nest("/llm", llm_routes)
    ...
```

---

## 7. Background Tasks

### 7.1 Approval Expiry Task

Add to `main.rs` alongside existing background tasks:

```rust
// Spawn background task to expire timed-out approval requests
let db_for_expiry = state.db.clone();
let config_for_expiry = state.config.clone();
let http_for_expiry = state.http_client.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        if let Err(e) = approval_service::expire_pending_requests(
            &db_for_expiry,
            &config_for_expiry,
            &http_for_expiry,
        ).await {
            tracing::warn!("Approval expiry task error: {e}");
        }
    }
});
```

The `expire_pending_requests` function:
1. Finds all `approval_requests` where `status == "pending"` and `expires_at < now`
2. Updates their status to `"expired"`
3. Edits the Telegram message to show "Expired (no response)"

---

## 8. Security Considerations

### 8.1 Webhook Verification

```rust
/// Verify the Telegram webhook secret token using constant-time comparison.
fn verify_webhook_secret(
    received: &str,
    expected: &str,
) -> bool {
    use subtle::ConstantTimeEq;
    received.as_bytes().ct_eq(expected.as_bytes()).into()
}
```

Add `subtle` crate to `Cargo.toml` for constant-time comparison.

### 8.2 Replay Prevention

- Approval decisions are processed atomically using MongoDB `findOneAndUpdate` with filter `{ status: "pending" }`
- Once status changes from "pending", subsequent callbacks for the same request are no-ops
- The callback handler returns 200 OK regardless (to stop Telegram retries)

### 8.3 Chat ID Binding

- When processing a Telegram callback, verify that `callback_query.message.chat.id` matches the `telegram_chat_id` stored on the approval request
- Prevents a user from approving another user's requests by spoofing callback data

### 8.4 Link Code Security

- Link codes are 6 alphanumeric characters (uppercase), prefixed with "NYXID-"
- Codes expire after 5 minutes
- Codes are single-use (cleared after successful linking)
- Codes are generated using `rand::distributions::Alphanumeric`

### 8.5 Rate Limiting

- Webhook endpoint is rate-limited by the global rate limiter
- `POST /notifications/telegram/link` is rate-limited (max 5 per minute per user) to prevent code flooding
- Approval request creation is deduplicated via idempotency keys

### 8.6 Error Message Safety

- Telegram messages show service name and requester label but never show credentials or internal IDs
- The `ApprovalRequired` error response includes the `request_id` but no credential information

---

## 9. Frontend Changes

### 9.1 New Types

**File:** `frontend/src/types/api.ts` (additions)

```typescript
export interface NotificationSettings {
  readonly telegram_connected: boolean;
  readonly telegram_username: string | null;
  readonly telegram_enabled: boolean;
  readonly approval_required: boolean;
  readonly approval_timeout_secs: number;
  readonly grant_expiry_days: number;
}

export interface TelegramLinkResponse {
  readonly link_code: string;
  readonly bot_username: string;
  readonly expires_in_secs: number;
  readonly instructions: string;
}

export interface ApprovalRequestItem {
  readonly id: string;
  readonly service_name: string;
  readonly service_slug: string;
  readonly requester_type: string;
  readonly requester_label: string | null;
  readonly operation_summary: string;
  readonly status: "pending" | "approved" | "rejected" | "expired";
  readonly created_at: string;
  readonly decided_at: string | null;
  readonly decision_channel: string | null;
}

export interface ApprovalGrantItem {
  readonly id: string;
  readonly service_id: string;
  readonly service_name: string;
  readonly requester_type: string;
  readonly requester_id: string;
  readonly requester_label: string | null;
  readonly granted_at: string;
  readonly expires_at: string;
}
```

### 9.2 New Zod Schemas

**File:** `frontend/src/schemas/notifications.ts`

```typescript
import { z } from "zod";

export const updateNotificationSettingsSchema = z.object({
  telegram_enabled: z.boolean(),
  approval_required: z.boolean(),
  approval_timeout_secs: z.number().int().min(10).max(300),
  grant_expiry_days: z.number().int().min(1).max(365),
});

export type UpdateNotificationSettingsFormData = z.infer<
  typeof updateNotificationSettingsSchema
>;
```

### 9.3 New TanStack Query Hooks

**File:** `frontend/src/hooks/use-approvals.ts`

```typescript
// Notification settings
export function useNotificationSettings()
export function useUpdateNotificationSettings()
export function useTelegramLink()
export function useTelegramDisconnect()

// Approval requests
export function useApprovalRequests(status?: string, page?: number)
export function useDecideApproval()

// Approval grants
export function useApprovalGrants(page?: number)
export function useRevokeGrant()
```

### 9.4 New Settings Tab: "Notifications"

Add a new tab to `frontend/src/pages/settings.tsx`:

```tsx
<TabsTrigger value="notifications">Notifications</TabsTrigger>

<TabsContent value="notifications">
  <NotificationsTab />
</TabsContent>
```

**NotificationsTab** sections:
1. **Telegram Connection** -- Show linked status, link/unlink button, link code dialog
2. **Approval Settings** -- Toggle approval requirement, configure timeout and grant expiry
3. **Active Grants** -- Table of current grants with revoke buttons

### 9.5 New Page: Approval History

**File:** `frontend/src/pages/approval-history.tsx`

Shows a paginated table of past approval requests with status, service name, requester, decision time, and channel. Filterable by status.

### 9.6 Dashboard Widget

Add an "Approval Requests" card to `frontend/src/pages/dashboard.tsx` showing:
- Count of pending approval requests
- Recent approval activity (last 5 decisions)
- Quick link to approval history page

### 9.7 Router Registration

Add to `frontend/src/router.tsx`:

```typescript
// New route for approval history
createRoute({
  getParentRoute: () => authenticatedRoute,
  path: '/approval-history',
  component: ApprovalHistoryPage,
})
```

Add "Approvals" to the sidebar navigation in `frontend/src/components/layout/sidebar.tsx`.

---

## 10. Crate Dependencies

Add to `backend/Cargo.toml`:

```toml
subtle = "2"          # Constant-time comparison for webhook verification
# sha2 and hex are already dependencies
```

No new major dependencies. The Telegram Bot API is accessed via the existing `reqwest` client. No `teloxide` dependency.

---

## 11. Approval Flow Sequence Diagram

```
Service Account / OAuth Client          NyxID Proxy            NyxID Backend           Telegram
         |                                  |                       |                      |
         |-- POST /proxy/s/openai/v1/... -->|                       |                      |
         |                                  |-- resolve_proxy_target |                      |
         |                                  |-- check_approval ----->|                      |
         |                                  |   (no grant found)     |                      |
         |                                  |                        |-- create_approval_request
         |                                  |                        |-- sendMessage ------->|
         |                                  |                        |   (inline keyboard)   |
         |<-- 403 approval_required --------|                        |                      |
         |                                  |                        |                      |
         |                                  |                        |      User clicks     |
         |                                  |                        |      [Approve]       |
         |                                  |                        |<-- callback_query ----|
         |                                  |                        |-- verify webhook secret
         |                                  |                        |-- process_decision("approved")
         |                                  |                        |-- create ApprovalGrant
         |                                  |                        |-- editMessageText --->|
         |                                  |                        |   "Approved"          |
         |                                  |                        |-- answerCallbackQuery->|
         |                                  |                        |                      |
         |-- POST /proxy/s/openai/v1/... -->|                       |                      |
         |                                  |-- check_approval ----->|                      |
         |                                  |   (grant found!)       |                      |
         |                                  |-- forward_request ---> downstream service     |
         |<-- 200 response -----------------|                       |                      |
```

---

## 12. Implementation Phases

### Phase 1: Core Backend (approval_service, models, indexes)
1. Create models: `approval_request.rs`, `approval_grant.rs`, `notification_channel.rs`
2. Register models in `mod.rs`
3. Add indexes to `db.rs`
4. Add config fields to `config.rs`
5. Implement `approval_service.rs` (check, create, process, list, revoke)
6. Implement `telegram_service.rs` (send, edit, answer, set_webhook)
7. Implement `notification_service.rs` (abstraction layer)
8. Add `ApprovalRequired` error variant

### Phase 2: Handlers and Routes
1. Implement `handlers/webhooks.rs` (Telegram webhook)
2. Implement `handlers/notifications.rs` (settings CRUD, Telegram link)
3. Implement `handlers/approvals.rs` (history, grants, decide, status)
4. Register routes in `routes.rs`
5. Add background expiry task to `main.rs`

### Phase 3: Proxy Integration
1. Modify `handlers/proxy.rs` to call approval check
2. Modify `handlers/llm_gateway.rs` to call approval check
3. Add the `request_id` field to `ErrorResponse`

### Phase 4: Frontend
1. Add TypeScript types and Zod schemas
2. Create `use-approvals.ts` hooks
3. Add Notifications tab to Settings page
4. Create Approval History page
5. Add dashboard widget
6. Update router and sidebar navigation

### Phase 5: Testing
1. Unit tests for approval_service (idempotency, expiry, replay prevention)
2. Unit tests for telegram_service (message formatting, callback parsing)
3. Integration tests for webhook endpoint (signature verification)
4. Frontend schema tests
5. E2E test for the approval flow (mock Telegram)

---

## 13. Open Questions and Future Work

1. **Sync vs Async Approval:** The current design returns `403 approval_required` immediately. An alternative is holding the connection open (long-poll) for up to `timeout_secs` and returning the actual response if approved in time. This would be more convenient for CLI tools but adds complexity. For now, the async pattern with polling is simpler and more reliable.

2. **Per-Service Approval Toggle:** Currently approval is user-global. A future enhancement could allow users to configure approval per-service (e.g. "require approval for OpenAI but not for internal services").

3. **Mobile App Push:** The `notification_channels` model has commented-out fields for FCM/APNs device tokens. When a NyxID mobile app is built, add a new notification channel implementation that sends push notifications alongside or instead of Telegram.

4. **Admin Override:** Admins might want to force approval requirements for all users or specific service accounts. This is deferred to a future iteration.

5. **Webhook Registration UI:** The initial implementation requires manual webhook setup via the Telegram Bot API. A future admin UI button could automate `setWebhook` registration.
