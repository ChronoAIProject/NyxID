# NyxID Mobile App Integration Guide

This guide is for mobile app developers building the NyxID iOS/Android app. It covers authentication, push notification setup, device token management, and the approval flow UI.

---

## Table of Contents

- [Overview](#overview)
- [Server-Side Configuration](#server-side-configuration)
  - [Environment Variables](#environment-variables)
  - [FCM Setup (Android)](#fcm-setup-android)
  - [APNs Setup (iOS)](#apns-setup-ios)
- [Authentication](#authentication)
  - [Login](#login)
  - [Token Refresh](#token-refresh)
  - [Logout](#logout)
- [Push Notification Setup](#push-notification-setup)
  - [Platform Setup (FCM / APNs)](#platform-setup-fcm--apns)
  - [Registering a Device Token](#registering-a-device-token)
  - [Handling Token Refresh](#handling-token-refresh)
  - [Listing Registered Devices](#listing-registered-devices)
  - [Removing a Device](#removing-a-device)
- [Notification Settings](#notification-settings)
  - [Reading Settings](#reading-settings)
  - [Updating Settings](#updating-settings)
- [Push Notification Handling](#push-notification-handling)
  - [Payload Schemas](#payload-schemas)
  - [Handling Approval Request Notifications](#handling-approval-request-notifications)
  - [Handling Silent Decision Notifications](#handling-silent-decision-notifications)
- [Approval Flow](#approval-flow)
  - [Polling Approval Status](#polling-approval-status)
  - [Approving or Rejecting a Request](#approving-or-rejecting-a-request)
  - [Listing Approval History](#listing-approval-history)
  - [Managing Approval Grants](#managing-approval-grants)
- [Error Handling](#error-handling)
- [Sequence Diagrams](#sequence-diagrams)
- [Quick Reference: All Endpoints](#quick-reference-all-endpoints)

---

## Overview

The NyxID mobile app serves two primary purposes:

1. **Receive push notifications** when a downstream service requests access to the user's credentials (approval requests)
2. **Approve or reject** those requests directly from the device

The app authenticates using the same JWT-based auth flow as the web frontend. Push notifications are delivered via **FCM** (Android) or **APNs** (iOS). Notification payloads are intentionally minimal (no sensitive data) -- the app must fetch approval details via API after receiving a push.

**Base URL:** All API paths below are relative to the NyxID backend (e.g., `https://auth.nyxid.dev`).

---

## Server-Side Configuration

Before the mobile app can receive push notifications, the NyxID backend must be configured with the appropriate credentials for FCM and/or APNs. These are set via environment variables (or `.env` file).

### Environment Variables

Add these to your `.env` file alongside existing NyxID configuration:

```bash
# ─── Push Notifications (optional) ───

# FCM (Firebase Cloud Messaging) -- Android push notifications
# Path to the service account JSON file downloaded from Firebase Console.
# When set, FCM push notifications are enabled at startup.
FCM_SERVICE_ACCOUNT_PATH=keys/fcm-service-account.json

# APNs (Apple Push Notification service) -- iOS push notifications
# All 4 APNs variables must be set together; if APNS_KEY_PATH is set,
# APNS_KEY_ID, APNS_TEAM_ID, and APNS_TOPIC are required.
APNS_KEY_PATH=keys/apns-auth-key.p8       # Path to .p8 private key file
APNS_KEY_ID=ABC123DEFG                     # 10-char Key ID from Apple Developer portal
APNS_TEAM_ID=TEAMID1234                    # 10-char Team ID from Apple Developer portal
APNS_TOPIC=dev.nyxid.app                   # iOS app bundle ID (used as apns-topic header)
APNS_SANDBOX=true                          # true = sandbox (development), false = production
                                           # Defaults: true in development, false in production
```

| Variable | Required | Description |
|----------|----------|-------------|
| `FCM_SERVICE_ACCOUNT_PATH` | For Android | Path to Firebase service account JSON file |
| `APNS_KEY_PATH` | For iOS | Path to APNs `.p8` authentication key file |
| `APNS_KEY_ID` | For iOS | Key ID from Apple Developer portal (Certificates, Identifiers & Profiles) |
| `APNS_TEAM_ID` | For iOS | Team ID from Apple Developer portal (top-right of portal) |
| `APNS_TOPIC` | For iOS | Bundle ID of the iOS app (e.g., `dev.nyxid.app`) |
| `APNS_SANDBOX` | No | Use APNs sandbox environment. Default: `true` in development, `false` in production |

Push notifications are disabled per-platform when the corresponding credentials are not configured. Users can still approve requests via the web UI or Telegram.

### FCM Setup (Android)

1. Go to [Firebase Console](https://console.firebase.google.com/) and create or select a project
2. Navigate to **Project Settings** > **Service accounts**
3. Click **Generate new private key** to download a JSON file
4. Place the file in the `keys/` directory (e.g., `keys/fcm-service-account.json`)
5. Set `FCM_SERVICE_ACCOUNT_PATH=keys/fcm-service-account.json` in `.env`

The JSON file contains `project_id`, `client_email`, and `private_key`. At startup, the backend:
- Reads and validates the JSON file
- Extracts `project_id` for the FCM HTTP v1 API URL
- Uses the `private_key` to generate OAuth2 access tokens (cached, auto-refreshed)
- Logs: `"FCM push notifications enabled (project: {project_id})"`

**Security:** The service account JSON file is read from disk at startup. It is never stored in the database or included in logs. Keep it out of version control (already in `.gitignore` via `keys/`).

### APNs Setup (iOS)

1. Go to [Apple Developer Portal](https://developer.apple.com/account/) > **Certificates, Identifiers & Profiles**
2. Navigate to **Keys** and create a new key with **Apple Push Notifications service (APNs)** enabled
3. Download the `.p8` key file (only downloadable once)
4. Note the **Key ID** (10-character string shown in the portal)
5. Note your **Team ID** (top-right corner of the developer portal)
6. Place the `.p8` file in the `keys/` directory (e.g., `keys/apns-auth-key.p8`)
7. Set all four APNs variables in `.env`

At startup, the backend:
- Reads and validates the `.p8` key file
- Verifies `APNS_KEY_ID` and `APNS_TEAM_ID` are also set (panics if not)
- Uses the key to generate ES256 JWT provider tokens (cached, auto-refreshed)
- Logs: `"APNs push notifications enabled (team: {team_id})"`

**Sandbox vs Production:**
- Use `APNS_SANDBOX=true` during development (sends to `api.sandbox.push.apple.com`)
- Set `APNS_SANDBOX=false` for production (sends to `api.push.apple.com`)
- The `ENVIRONMENT` variable also influences the default: `development` defaults to sandbox, anything else defaults to production

**Security:** The `.p8` key file is read from disk at startup. Never commit it to version control.

### Production Deployment Checklist

- [ ] FCM service account JSON placed in a secure, non-public directory
- [ ] APNs `.p8` key placed in a secure, non-public directory with restricted permissions (`chmod 600`)
- [ ] `APNS_SANDBOX=false` for production deployments
- [ ] `APNS_TOPIC` matches the production app's bundle ID
- [ ] Key files excluded from container images (mount as secrets/volumes)
- [ ] Verify push delivery with a test device before going live

---

## Authentication

The mobile app uses email/password login to obtain a JWT access token and a refresh token (stored as an HttpOnly cookie, but the access token is also returned in the JSON body for use in `Authorization` headers).

### Login

```
POST /api/v1/auth/login
Content-Type: application/json
```

**Request:**
```json
{
  "email": "user@example.com",
  "password": "securepassword123"
}
```

**Response (200):**
```json
{
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "access_token": "eyJhbGciOiJSUzI1NiIs...",
  "expires_in": 900
}
```

**Response (200, MFA required):**
```json
{
  "error": "mfa_required",
  "error_code": 2002,
  "message": "MFA verification required",
  "session_token": "temp-session-token"
}
```

If MFA is required, prompt the user for their TOTP code and submit:

```
POST /api/v1/auth/login
Content-Type: application/json
```
```json
{
  "email": "user@example.com",
  "password": "securepassword123",
  "mfa_code": "123456"
}
```

**Important:** The response also sets `nyx_session` and `nyx_access_token` cookies. For mobile, use the `access_token` from the JSON body in the `Authorization: Bearer <token>` header for all subsequent requests.

### Token Refresh

Access tokens expire after 15 minutes (configurable). Use the refresh endpoint to obtain a new one. The refresh token is stored in the `nyx_refresh_token` cookie (automatically included if your HTTP client supports cookie jars).

```
POST /api/v1/auth/refresh
Cookie: nyx_refresh_token=...
```

**Response (200):**
```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIs...",
  "expires_in": 900
}
```

**Recommended:** Set up an HTTP interceptor that automatically refreshes the token when a request returns 401.

### Logout

```
POST /api/v1/auth/logout
Authorization: Bearer <access_token>
```

**Response (200):**
```json
{
  "message": "Logged out"
}
```

**Important:** On logout, also call `DELETE /api/v1/notifications/devices/{device_id}` to deregister the push device (see [Removing a Device](#removing-a-device)).

---

## Push Notification Setup

### Platform Setup (FCM / APNs)

**Android (FCM):**

1. Add your app to a Firebase project
2. Download the `google-services.json` and add to your Android project
3. Add the Firebase Cloud Messaging SDK dependency
4. Obtain the FCM registration token via `FirebaseMessaging.getInstance().token`

**iOS (APNs):**

1. Enable Push Notifications capability in Xcode
2. Register for remote notifications: `UIApplication.shared.registerForRemoteNotifications()`
3. Obtain the device token from `application(_:didRegisterForRemoteNotificationsWithDeviceToken:)`
4. Convert the raw `Data` token to a hex string

**Server-side:** The NyxID backend must be configured with the appropriate credentials. See [Server-Side Configuration](#server-side-configuration) for full setup instructions and environment variables.

### Registering a Device Token

Call this endpoint on every app launch and whenever the platform issues a new token (FCM `onNewToken` / APNs `didRegisterForRemoteNotificationsWithDeviceToken`).

```
POST /api/v1/notifications/devices
Authorization: Bearer <access_token>
Content-Type: application/json
```

**Request:**
```json
{
  "platform": "fcm",
  "token": "dGVzdC1kZXZpY2UtdG9rZW4...",
  "device_name": "Pixel 8 Pro",
  "app_id": "dev.nyxid.app"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `platform` | string | Yes | `"fcm"` (Android) or `"apns"` (iOS) |
| `token` | string | Yes | FCM registration token or APNs device token (hex). Max 4096 chars. |
| `device_name` | string | No | Human-readable device name. Max 100 chars. |
| `app_id` | string | APNs: Yes, FCM: No | Bundle ID (e.g., `"dev.nyxid.app"`). Required for APNs. Max 256 chars. |

**Validation rules:**
- `platform` must be `"fcm"` or `"apns"`
- APNs tokens must contain only hexadecimal characters (`[0-9a-fA-F]`)
- FCM tokens may contain alphanumeric characters, colons, hyphens, and underscores
- Maximum 10 devices per user

**Response (200 -- existing token refreshed, or new device created):**
```json
{
  "device_id": "550e8400-e29b-41d4-a716-446655440000",
  "platform": "fcm",
  "device_name": "Pixel 8 Pro",
  "registered_at": "2026-03-03T12:00:00+00:00"
}
```

This endpoint is **idempotent**: if the same `token` is registered again, it updates the device metadata (name, timestamp) rather than creating a duplicate. Store the returned `device_id` locally for use during logout/uninstall cleanup.

**Error (400):**
```json
{
  "error": "bad_request",
  "error_code": 1000,
  "message": "Maximum of 10 devices per user exceeded"
}
```

### Handling Token Refresh

Both FCM and APNs may issue new tokens at any time. When this happens:

- **Android:** Override `onNewToken(token: String)` in your `FirebaseMessagingService`
- **iOS:** Handle `application(_:didRegisterForRemoteNotificationsWithDeviceToken:)`

In both cases, call `POST /api/v1/notifications/devices` with the new token. The server automatically upserts based on the token value.

### Listing Registered Devices

```
GET /api/v1/notifications/devices
Authorization: Bearer <access_token>
```

**Response (200):**
```json
{
  "devices": [
    {
      "device_id": "550e8400-e29b-41d4-a716-446655440000",
      "platform": "fcm",
      "device_name": "Pixel 8 Pro",
      "registered_at": "2026-03-03T12:00:00+00:00",
      "last_used_at": "2026-03-03T14:30:00+00:00"
    },
    {
      "device_id": "660e8400-e29b-41d4-a716-446655440001",
      "platform": "apns",
      "device_name": "iPhone 15",
      "registered_at": "2026-03-01T08:00:00+00:00",
      "last_used_at": null
    }
  ],
  "push_enabled": true
}
```

Note: The actual device `token` is never returned in API responses -- it is a secret between the device and push service.

### Removing a Device

Call this on user logout or app uninstall (if detectable).

```
DELETE /api/v1/notifications/devices/{device_id}
Authorization: Bearer <access_token>
```

**Response (200):**
```json
{
  "message": "Device removed"
}
```

If no devices remain after removal, the server automatically sets `push_enabled = false`.

**Error (404):**
```json
{
  "error": "not_found",
  "error_code": 1003,
  "message": "Device not found"
}
```

---

## Notification Settings

### Reading Settings

```
GET /api/v1/notifications/settings
Authorization: Bearer <access_token>
```

**Response (200):**
```json
{
  "telegram_connected": true,
  "telegram_username": "@user",
  "telegram_enabled": true,
  "push_enabled": true,
  "push_device_count": 2,
  "approval_required": true,
  "approval_timeout_secs": 30,
  "grant_expiry_days": 30
}
```

| Field | Type | Description |
|-------|------|-------------|
| `telegram_connected` | boolean | Whether a Telegram account is linked |
| `telegram_username` | string? | Linked Telegram username |
| `telegram_enabled` | boolean | Whether Telegram notifications are active |
| `push_enabled` | boolean | Whether push notifications are active |
| `push_device_count` | number | Number of registered push devices |
| `approval_required` | boolean | Global approval toggle |
| `approval_timeout_secs` | number | Seconds before auto-reject (10-300) |
| `grant_expiry_days` | number | Days before grant expires (1-365) |

### Updating Settings

```
PUT /api/v1/notifications/settings
Authorization: Bearer <access_token>
Content-Type: application/json
```

**Request:**
```json
{
  "telegram_enabled": true,
  "push_enabled": true,
  "approval_required": true,
  "approval_timeout_secs": 60,
  "grant_expiry_days": 14
}
```

**Response (200):** Same shape as GET response.

**Validation:**
- `push_enabled` cannot be set to `true` if no devices are registered
- `approval_timeout_secs` must be 10-300
- `grant_expiry_days` must be 1-365

---

## Push Notification Handling

### Payload Schemas

Push notifications contain **no sensitive data**. The app must authenticate and call the API to fetch full approval details.

#### Alert Notification (Approval Request)

Sent when a service requests access to the user's credentials.

**FCM payload:**
```json
{
  "message": {
    "notification": {
      "title": "Approval Required",
      "body": "A service is requesting access"
    },
    "data": {
      "type": "approval_request",
      "request_id": "550e8400-e29b-41d4-a716-446655440000"
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

**APNs payload:**
```json
{
  "aps": {
    "alert": {
      "title": "Approval Required",
      "body": "A service is requesting access"
    },
    "sound": "default",
    "mutable-content": 1,
    "category": "APPROVAL_REQUEST"
  },
  "type": "approval_request",
  "request_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Android:** Create a notification channel with ID `approvals` at app startup:
```kotlin
val channel = NotificationChannel(
    "approvals",
    "Approval Requests",
    NotificationManager.IMPORTANCE_HIGH
)
notificationManager.createNotificationChannel(channel)
```

**iOS:** Register a `UNNotificationCategory` with identifier `APPROVAL_REQUEST` to enable action buttons:
```swift
let approve = UNNotificationAction(identifier: "APPROVE", title: "Approve", options: [.authenticationRequired])
let reject = UNNotificationAction(identifier: "REJECT", title: "Reject", options: [.destructive, .authenticationRequired])
let category = UNNotificationCategory(identifier: "APPROVAL_REQUEST", actions: [approve, reject], intentIdentifiers: [])
UNUserNotificationCenter.current().setNotificationCategories([category])
```

#### Silent Notification (Decision Made)

Sent after a decision is made (via Telegram, web UI, or another device) so the app can refresh its UI.

**FCM data-only message:**
```json
{
  "data": {
    "type": "approval_decision",
    "request_id": "550e8400-e29b-41d4-a716-446655440000",
    "decision": "approved"
  }
}
```

**APNs silent push:**
```json
{
  "aps": {
    "content-available": 1
  },
  "type": "approval_decision",
  "request_id": "550e8400-e29b-41d4-a716-446655440000",
  "decision": "approved"
}
```

The `decision` field is `"approved"`, `"rejected"`, or `"expired"`.

### Handling Approval Request Notifications

When a push notification with `type: "approval_request"` is received:

1. **Foreground:** Show an in-app banner or modal with the approval UI
2. **Background/notification tap:** Navigate to the approval detail screen

In both cases:
1. Extract `request_id` from the `data` payload
2. Call `GET /api/v1/approvals/requests/{request_id}/status` to fetch full details
3. Display the approval screen with service name, requester, and operation
4. Let the user approve or reject

### Handling Silent Decision Notifications

When a push notification with `type: "approval_decision"` is received:

1. If the app is displaying the approval detail screen for this `request_id`, refresh the UI to show the decision
2. If the app is on the approval history screen, refresh the list
3. Otherwise, silently update any cached approval data

---

## Approval Flow

### Polling Approval Status

After receiving a push notification, fetch the full approval request details:

```
GET /api/v1/approvals/requests/{request_id}/status
Authorization: Bearer <access_token>
```

**Response (200):**
```json
{
  "status": "pending",
  "expires_at": "2026-03-03T12:00:30+00:00"
}
```

| Status | Meaning |
|--------|---------|
| `pending` | Awaiting user decision |
| `approved` | User approved the request |
| `rejected` | User rejected the request |
| `expired` | Request timed out without a decision |

If the status is `pending`, the app should show the Approve/Reject buttons. If the `expires_at` has passed, treat it as expired even if the status hasn't been updated yet.

### Approving or Rejecting a Request

```
POST /api/v1/approvals/requests/{request_id}/decide
Authorization: Bearer <access_token>
Content-Type: application/json
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
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "approved",
  "decided_at": "2026-03-03T12:00:05+00:00"
}
```

**Error (400 -- already decided):**
```json
{
  "error": "bad_request",
  "error_code": 1000,
  "message": "Request has already been decided"
}
```

After a successful decision, show a confirmation UI (e.g., checkmark for approved, X for rejected) and update the local state.

### Listing Approval History

```
GET /api/v1/approvals/requests?page=1&per_page=20&status=pending
Authorization: Bearer <access_token>
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `page` | number | 1 | Page number |
| `per_page` | number | 20 | Items per page |
| `status` | string | (all) | Filter: `pending`, `approved`, `rejected`, `expired` |

**Response (200):**
```json
{
  "requests": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "service_name": "OpenAI API",
      "service_slug": "openai",
      "requester_type": "service_account",
      "requester_label": "CI Pipeline",
      "operation_summary": "proxy:POST /v1/chat/completions",
      "status": "approved",
      "created_at": "2026-03-03T12:00:00+00:00",
      "decided_at": "2026-03-03T12:00:05+00:00",
      "decision_channel": "push"
    }
  ],
  "total": 42,
  "page": 1,
  "per_page": 20
}
```

### Managing Approval Grants

List active grants (approved access that hasn't expired):

```
GET /api/v1/approvals/grants?page=1&per_page=20
Authorization: Bearer <access_token>
```

**Response (200):**
```json
{
  "grants": [
    {
      "id": "660e8400-e29b-41d4-a716-446655440001",
      "service_id": "770e8400-e29b-41d4-a716-446655440002",
      "service_name": "OpenAI API",
      "requester_type": "service_account",
      "requester_id": "880e8400-e29b-41d4-a716-446655440003",
      "requester_label": "CI Pipeline",
      "granted_at": "2026-03-03T12:00:05+00:00",
      "expires_at": "2026-04-02T12:00:05+00:00"
    }
  ],
  "total": 5,
  "page": 1,
  "per_page": 20
}
```

Revoke a specific grant:

```
DELETE /api/v1/approvals/grants/{grant_id}
Authorization: Bearer <access_token>
```

**Response (200):**
```json
{
  "message": "Grant revoked"
}
```

---

## Error Handling

All errors follow a consistent JSON format:

```json
{
  "error": "error_key",
  "error_code": 1000,
  "message": "Human-readable description"
}
```

### Relevant Error Codes

| Code | Key | HTTP Status | Meaning |
|------|-----|-------------|---------|
| 1000 | `bad_request` | 400 | Invalid input, malformed request |
| 1001 | `unauthorized` | 401 | Missing or invalid/expired access token |
| 1002 | `forbidden` | 403 | Insufficient permissions |
| 1003 | `not_found` | 404 | Resource does not exist |
| 1004 | `conflict` | 409 | Duplicate resource |
| 1008 | `validation_error` | 400 | Input validation failure |
| 2002 | `mfa_required` | 200 | MFA code needed to complete login |

### Recommended Error Handling Strategy

1. **401 responses:** Attempt a token refresh. If refresh also returns 401, redirect to login
2. **Network errors:** Show a retry option. Push device registration can safely retry (idempotent)
3. **400/validation errors:** Display the `message` to the user
4. **404 on device removal:** The device was already removed (treat as success)
5. **404 on approval decide:** The request may have expired or been decided elsewhere

---

## Sequence Diagrams

### App Launch Flow

```
Mobile App                          NyxID Backend
    |                                    |
    |-- POST /auth/login --------------->|
    |<-- 200 { access_token } ----------|
    |                                    |
    |-- Get FCM/APNs token from OS       |
    |                                    |
    |-- POST /notifications/devices ---->|
    |<-- 200 { device_id } -------------|
    |                                    |
    |   (store device_id locally)        |
```

### Approval Notification Flow

```
Service Account       NyxID Backend        Push Service       Mobile App
    |                      |                    |                  |
    |-- proxy request ---->|                    |                  |
    |                      |-- check approval   |                  |
    |                      |   (no grant found) |                  |
    |                      |-- send FCM/APNs -->|                  |
    |<-- 403 approval_required                  |-- push --------->|
    |                      |                    |                  |
    |                      |                    |     User taps    |
    |                      |                    |     notification |
    |                      |                    |                  |
    |                      |<--- GET .../status --------------------|
    |                      |---- 200 { pending } ----------------->|
    |                      |                    |                  |
    |                      |<--- POST .../decide { approved } ----|
    |                      |---- 200 { approved } ---------------->|
    |                      |                    |                  |
    |                      |-- create grant     |                  |
    |                      |-- silent push ---->|-- silent push -->|
    |                      |                    |   (UI refresh)   |
    |                      |                    |                  |
    |-- retry proxy ------>|                    |                  |
    |                      |-- grant found!     |                  |
    |<-- 200 response -----|                    |                  |
```

### Logout Flow

```
Mobile App                          NyxID Backend
    |                                    |
    |-- DELETE /notifications/devices/{id} ->|
    |<-- 200 { message: "Device removed" } --|
    |                                    |
    |-- POST /auth/logout -------------->|
    |<-- 200 { message: "Logged out" } --|
    |                                    |
    |   (clear local tokens/state)       |
```

---

## Quick Reference: All Endpoints

### Authentication

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/v1/auth/login` | Log in (email + password + optional MFA) |
| `POST` | `/api/v1/auth/refresh` | Refresh access token (requires refresh cookie) |
| `POST` | `/api/v1/auth/logout` | Log out and revoke session |

### Push Device Management

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/v1/notifications/devices` | Register or refresh a device token |
| `GET` | `/api/v1/notifications/devices` | List registered devices |
| `DELETE` | `/api/v1/notifications/devices/{device_id}` | Remove a device |

### Notification Settings

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/notifications/settings` | Get notification preferences |
| `PUT` | `/api/v1/notifications/settings` | Update notification preferences |

### Approval Management

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/approvals/requests` | List approval requests (filterable, paginated) |
| `GET` | `/api/v1/approvals/requests/{id}/status` | Get approval request status |
| `POST` | `/api/v1/approvals/requests/{id}/decide` | Approve or reject a request |
| `GET` | `/api/v1/approvals/grants` | List active approval grants |
| `DELETE` | `/api/v1/approvals/grants/{grant_id}` | Revoke an approval grant |

### User Profile

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/users/me` | Get current user profile |
| `PUT` | `/api/v1/users/me` | Update current user profile |

All endpoints require `Authorization: Bearer <access_token>` unless otherwise noted.
