# Backend: Push Device & Notification Changes

## Summary

**What changed**  
This backend update focuses on push devices and notifications: the register endpoint now supports `previous_token` for token rotation; a new "unregister current device by token" endpoint was added; on register, the backend enforces one-token-per-user and detaches the token from other users; after register/rotation it cleans up duplicate device entries with the same token; notifications are sent only to a deduplicated list by token. Additionally: a sparse index on `push_devices.token`, reqwest with `http2` enabled, and the APNs default `app_id` fallback was updated in the notification service.

**Problems solved**  
- **Push still delivered to old account after sign-out or account switch**: Enforcing one-token-per-user and calling "remove current device" on sign-out prevents the token from remaining on the previous account and leaking pushes.  
- **Duplicate or stale device records when the same device’s token rotates**: The `previous_token` path updates the same device record with the new token and then removes other entries with the same token but different `device_id`.  
- **Same token sent multiple times within a channel**: Sending is done after deduplication by token, avoiding duplicate pushes and redundant calls.

---

## 1. Dependencies & config

| File | Change |
|------|--------|
| `Cargo.toml` | `reqwest` gains the `http2` feature for FCM/APNs HTTP/2 requests. |

---

## 2. Database

| File | Change |
|------|--------|
| `db.rs` | New **sparse index** on `notification_channels`: `push_devices.token` (asc). Used for efficient query/cleanup by token (e.g. account switch, logout detach). |

---

## 3. Device register & unregister (handlers/device_tokens.rs)

### 3.1 Request bodies & routes

- **RegisterDeviceRequest** gains optional `previous_token: Option<String>`. Used for "same device, new token" rotation: replace old token with new token while keeping the same `device_id`.
- **UnregisterCurrentDeviceRequest** (new): `platform` + `token`, for "remove the current user’s device identified by this push token".
- **Route**: New `DELETE /api/v1/notifications/devices/current`, handler `remove_current_device`, authenticated.

### 3.2 Register flow (register_device) logic

1. **One token, one user**  
   Before registering, `detach_token_from_other_users` runs: it `$pull`s the current `token` from **other users’** `notification_channels`; if a user ends up with no devices, their `push_enabled` is set to `false`. This prevents one physical device token from being associated with multiple users and leaking pushes.

2. **Token rotation path (previous_token)**  
   If `previous_token` is provided and differs from `token`, and the current user’s channel has a device with `token == previous_token`:
   - Validate platform match (`ensure_platform_matches`);
   - `$set` that device’s `token`, `platform`, `registered_at`, and optionally `device_name`, `app_id`;
   - Call `remove_duplicate_token_entries` to remove other entries in the same channel with the same token but different `device_id`;
   - Return immediately (no "new device" or "same-token refresh" branch).

3. **Other paths (new device / same-token refresh / new token same device_id)**  
   After the write that adds or updates the device, `remove_duplicate_token_entries` is always called so that within a channel only one device record per token remains (the one for the current `device_id`).

### 3.3 New: Remove current device by token (remove_current_device)

- **DELETE /api/v1/notifications/devices/current**, Body: `{ "platform", "token" }`.
- `$pull` from the current user’s channel any `push_devices` entry with `push_devices.token == body.token`, and update `updated_at`.
- If the channel has no devices left and `push_enabled` is true, set `push_enabled = false`.
- Audit log `push_device_removed_on_logout` for integration with sign-out flow.

Purpose: Called by the client on sign-out or account switch so the device token is no longer associated with the current user and does not keep receiving that account’s pushes.

### 3.4 Internal helpers

- **detach_token_from_other_users**: Remove the given token from all channels whose `user_id` is not the current user; keep `push_enabled` in sync when a user loses their last device.
- **remove_duplicate_token_entries**: Within one channel, keep the given `device_id` and remove other device entries that share the same token.

### 3.5 Validation & tests

- **validate_register_request**: Validates `previous_token` with the same platform rules as `token` (length, format, invalid chars) via `validate_token_for_platform(..., "previous_token")`.
- Unit tests: All `RegisterDeviceRequest` constructions include `previous_token: None`; new tests `validate_previous_token_too_long` and `validate_previous_token_rejects_invalid_chars`.

---

## 4. Route registration (routes.rs)

- Under notifications `/devices`, new sub-route:  
  `DELETE /devices/current` → `handlers::device_tokens::remove_current_device`.

---

## 5. Notification service (services/notification_service.rs)

### 5.1 Deduplicate by token before sending

- New **unique_devices_by_token(devices)**: Deduplicates by `device.token`, keeps first occurrence, returns `Vec<&DeviceToken>`.
- **send_approval_notification**, **notify_decision**, and **send_silent_push_to_user** now run `unique_devices_by_token` on `channel.push_devices` before sending.  
  Avoids duplicate pushes when the same token appears multiple times due to history or concurrency.

### 5.2 Other

- APNs silent push default `app_id` fallback changed from `"dev.nyxid.app"` to `"fun.chrono-ai.nyxid"` (aligned with current project config).

---

## 6. Changed files

| File | Description |
|------|--------------|
| `backend/Cargo.toml` | reqwest http2 enabled |
| `backend/src/db.rs` | Sparse index on notification_channels.push_devices.token |
| `backend/src/handlers/device_tokens.rs` | Register rotation, one-user-one-token, dedup, remove_current_device, validation & tests |
| `backend/src/routes.rs` | DELETE /devices/current |
| `backend/src/services/notification_service.rs` | unique_devices_by_token, APNs default app_id |

---

## 7. Client integration

- **Before sign-out or account switch**: Call `DELETE /api/v1/notifications/devices/current` with current `platform` and `token` so the device is detached from the current user.
- **Token refresh (same device, new token)**: Register with `previous_token` set to the old token and `token` to the new one to keep the same `device_id` and avoid duplicate device records.
