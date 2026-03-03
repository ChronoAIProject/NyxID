# Approval System Code & Security Review

**Reviewer:** reviewer (automated code review agent)
**Date:** 2026-03-03
**Scope:** All new and modified files for the approval system feature

---

## Summary

The approval system is well-architected overall. Models follow MongoDB conventions correctly (UUID strings in `_id`, bson datetime helpers, `COLLECTION_NAME` constants, no `skip_serializing`). Layer separation is clean (handler -> service -> model). Dedicated response structs are used in handlers. Security fundamentals (constant-time webhook verification, atomic replay prevention via `findOneAndUpdate`, chat_id binding, idempotency keys) are properly implemented.

**Findings:** 3 HIGH, 7 MEDIUM, 6 LOW

---

## HIGH Severity

### [HIGH] Backend/Frontend type mismatch: ApprovalGrantItem missing fields
**File:** `backend/src/handlers/approvals.rs:37-44` and `frontend/src/types/approvals.ts:48-57`
**Description:** The backend `ApprovalGrantItem` response struct has no `service_name`, `requester_label` fields. The frontend `ApprovalGrantItem` type declares `service_name: string` and `requester_label: string | null`, and the UI references these (e.g., `approval-grants.tsx:113` renders `grant.service_name`). The result is `undefined` displayed in the UI for these columns.
**Suggested Fix:** Either:
- (A) Add `service_name` and `requester_label` to the backend `ApprovalGrantItem` struct. In `list_grants`, join with the `downstream_services` collection or denormalize these fields onto `ApprovalGrant` at creation time.
- (B) Remove `service_name` and `requester_label` from the frontend type and adjust the UI to show `service_id` with a secondary lookup.

Option (A) is preferred since the data is already available on the `ApprovalRequest` that created the grant, and could be denormalized onto the grant at creation time.

---

### [HIGH] Race condition in `get_or_create_channel`
**File:** `backend/src/services/notification_service.rs:93-121`
**Description:** Two concurrent requests for the same user can both call `get_or_create_channel`, both find no existing channel, and both attempt `insert_one`. The second insert will fail with a MongoDB duplicate key error (code 11000) due to the unique index on `user_id`. This error propagates as an unhandled `AppError::DatabaseError`, resulting in a 500 Internal Server Error.
**Suggested Fix:** Handle the duplicate key error gracefully, similar to how `create_approval_request` handles idempotency key conflicts:
```rust
match collection.insert_one(&channel).await {
    Ok(_) => Ok(channel),
    Err(e) if is_duplicate_key_error(&e) => {
        // Another request created it first; fetch it
        collection
            .find_one(doc! { "user_id": user_id })
            .await?
            .ok_or_else(|| AppError::Internal("Channel creation conflict".to_string()))
    }
    Err(e) => Err(AppError::DatabaseError(e)),
}
```

---

### [HIGH] No validation of `status` query parameter
**File:** `backend/src/handlers/approvals.rs:74-79` and `backend/src/services/approval_service.rs:327`
**Description:** The `ApprovalRequestsQuery.status` field is `Option<String>` with no validation. Any string value is passed directly into the MongoDB filter. While this doesn't enable NoSQL injection (it's a string equality filter), it means clients can pass invalid statuses (e.g., `?status=foobar`) without receiving a validation error, silently returning empty results. More critically, this inconsistency could mask bugs.
**Suggested Fix:** Validate `status` against the known set before querying:
```rust
if let Some(ref status) = query.status {
    if !["pending", "approved", "rejected", "expired"].contains(&status.as_str()) {
        return Err(AppError::ValidationError(
            "status must be one of: pending, approved, rejected, expired".to_string(),
        ));
    }
}
```

---

## MEDIUM Severity

### [MEDIUM] Hardcoded bot username "NyxIDBot"
**File:** `backend/src/handlers/notifications.rs:181`
**Description:** The Telegram bot username is hardcoded as `"NyxIDBot"`. If the bot is renamed or a different bot is used per environment, this will be incorrect. The link instructions sent to the user will point to the wrong bot.
**Suggested Fix:** Either:
- Derive the bot username from the `telegram_webhook_url` config, or
- Add a `TELEGRAM_BOT_USERNAME` environment variable to `AppConfig`, or
- Call the Telegram `getMe` API at startup to get the actual bot username.

---

### [MEDIUM] HTML injection risk in Telegram messages
**File:** `backend/src/services/telegram_service.rs:70-76`
**Description:** User-controlled values (`service_name`, `service_slug`, `requester_label`, `operation_summary`) are interpolated into HTML-formatted Telegram messages without escaping. While Telegram's Bot API only supports a limited HTML subset (`<b>`, `<i>`, `<code>`, etc.) and will reject/strip unknown tags, characters like `<`, `>`, `&` in service names could still break the HTML parsing and cause display issues or message delivery failures.
**Suggested Fix:** HTML-escape user-controlled values before interpolation:
```rust
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
}
```
Apply to `service_name`, `service_slug`, `requester_label`, and `operation_summary` in both `send_approval_message` and `edit_message_after_decision`.

---

### [MEDIUM] Status polling endpoint exposes request status without ownership verification
**File:** `backend/src/handlers/approvals.rs:141-151` and `backend/src/routes.rs:280-281`
**Description:** `get_request_status` takes no `AuthUser` extractor and is mounted in the `api_v1_delegated` router (accessible by service accounts and delegated tokens). Any authenticated caller who knows a request UUID can query its status. While the request_id is returned in the 403 error (so the legitimate caller has it), this means any valid token holder could poll status for any approval request if they can guess/know the UUID.
**Suggested Fix:** This is acceptable by design (callers need to poll after receiving `approval_required`), but add a comment documenting this security decision. Optionally, consider verifying that the caller's token was associated with the original request (e.g., by checking `requester_id` matches the token's subject).

---

### [MEDIUM] `u32` to `i32` cast in BSON update
**File:** `backend/src/handlers/notifications.rs:102-105`
**Description:** `approval_timeout_secs` (u32) and `grant_expiry_days` (u32) are cast to `i32` via `v as i32` before inserting into the BSON update document. While the validation bounds (10-300 and 1-365) ensure the values always fit in `i32`, the `as` cast is a footgun -- if validation bounds are ever raised above `i32::MAX`, this would silently wrap.
**Suggested Fix:** Use explicit conversion: `i32::try_from(v).unwrap_or(v as i32)` or simply `bson::Bson::Int32(v as i32)` with a comment noting the validation constraint. Better yet, keep as `bson::Bson::Int32` and add an assertion: `debug_assert!(v <= i32::MAX as u32);`

---

### [MEDIUM] `per_page` to `i64` cast without bounds check
**File:** `backend/src/services/approval_service.rs:344,393`
**Description:** `per_page as i64` is used for the MongoDB `.limit()` call. The handler caps `per_page` at 100 via `.min(100)`, so this is safe in practice. However, if a new caller is added without the same validation, values above `i64::MAX` would wrap.
**Suggested Fix:** Use `i64::try_from(per_page).unwrap_or(100)` or add a `debug_assert!(per_page <= 100)` in the service function.

---

### [MEDIUM] `expire_pending_requests` loads all expired requests into memory
**File:** `backend/src/services/approval_service.rs:264-272`
**Description:** The function collects all pending+expired requests via `.try_collect()` into a `Vec` before batch-updating. For a system under heavy load with many simultaneous approval requests timing out, this could consume significant memory. Also, the background task runs every 5 seconds, which may be unnecessarily aggressive.
**Suggested Fix:**
- Add a `.limit(100)` to the find query to cap memory usage per tick.
- Use `update_many` directly with the filter instead of fetching first (only fetch when Telegram messages need editing).
- Consider making the interval configurable or increasing to 10-15 seconds.

---

## LOW Severity

### [LOW] Constant-time comparison leaks secret length
**File:** `backend/src/handlers/webhooks.rs:314-316`
**Description:** `subtle::ConstantTimeEq` for `[u8]` slices returns early (non-constant-time) when the slices have different lengths. This leaks the length of the webhook secret to a timing side-channel attacker. While this only reveals the secret's length (not content), and webhook secrets are typically long random strings, it's worth noting.
**Suggested Fix:** Pre-hash both values (e.g., SHA-256) before comparing, so the comparison is always on 32-byte values regardless of input length:
```rust
fn verify_webhook_secret(received: &str, expected: &str) -> bool {
    use sha2::{Sha256, Digest};
    use subtle::ConstantTimeEq;
    let h1 = Sha256::digest(received.as_bytes());
    let h2 = Sha256::digest(expected.as_bytes());
    h1.ct_eq(&h2).into()
}
```

---

### [LOW] Missing `#[serde(default)]` on optional Telegram fields in `ApprovalRequest`
**File:** `backend/src/models/approval_request.rs:47-52`
**Description:** `telegram_message_id` and `telegram_chat_id` are `Option<i64>` but lack `#[serde(default)]`. When deserializing a BSON document that was inserted before Telegram notification (these fields are `None` initially and updated separately via `$set`), if the field is missing from the document entirely, deserialization may fail. The current flow always sets them (even as `null`), but if any code path skips the notification update, documents without these fields would fail to deserialize.
**Suggested Fix:** Add `#[serde(default)]` to both fields for defensive deserialization:
```rust
#[serde(default)]
pub telegram_message_id: Option<i64>,
#[serde(default)]
pub telegram_chat_id: Option<i64>,
```

---

### [LOW] `notification_channel` field on `ApprovalRequest` also missing `#[serde(default)]`
**File:** `backend/src/models/approval_request.rs:46`
**Description:** Same issue as above -- `notification_channel: Option<String>` lacks `#[serde(default)]`. If the field is absent from the BSON document (not null, but missing), deserialization fails.
**Suggested Fix:** Add `#[serde(default)]` to `notification_channel`, `telegram_message_id`, `telegram_chat_id`, and `decision_channel`.

---

### [LOW] Background expiry task interval is not configurable
**File:** `backend/src/main.rs:180`
**Description:** The expiry task interval is hardcoded to 5 seconds. In production with many requests, this is fine. In development or testing, it adds unnecessary DB load. This isn't a bug, just a minor maintainability concern.
**Suggested Fix:** Add an `APPROVAL_EXPIRY_INTERVAL_SECS` env var (default 5) to `AppConfig`, or document the interval in the plan.

---

### [LOW] `telegram_link` generates link code using `rand::thread_rng` without `OsRng`
**File:** `backend/src/handlers/notifications.rs:148`
**Description:** `rand::thread_rng()` uses a CSPRNG seeded from the OS, which is fine for link codes. However, the code is 6 alphanumeric characters (36^6 ~ 2.18 billion combinations), providing roughly 31 bits of entropy. With the 5-minute expiry and Telegram's own rate limiting, this is adequate, but for higher-security environments, 8+ characters would provide more margin.
**Suggested Fix:** Consider increasing to 8 characters (36^8 ~ 2.8 trillion, ~41 bits) for additional security margin. This is optional.

---

### [LOW] `decision_channel` field not included in `ApprovalRequestItem` response
**File:** `backend/src/handlers/approvals.rs:26` vs `frontend/src/types/approvals.ts:38`
**Description:** The frontend type includes `decision_channel: string | null` and the backend handler does include it (line 125), so this is actually fine. No action needed -- this is a false positive I'm including for completeness of the review.

---

## Code Quality Observations (No Fix Required)

### Models
- All three new models follow MongoDB conventions correctly: UUID v4 `_id`, `COLLECTION_NAME`, bson datetime helpers, no `skip_serializing`.
- Comprehensive bson roundtrip tests for each model.

### Layer Separation
- Clean handler -> service -> model separation maintained throughout.
- Handlers use dedicated response structs (e.g., `ApprovalRequestItem`, `NotificationSettingsResponse`).

### Error Handling
- `AppError::ApprovalRequired` variant properly integrated with status code (403), error code (7000), and error key.
- `ErrorResponse` correctly includes `request_id` field with `skip_serializing_if`.
- Comprehensive error code uniqueness test maintained.

### Security Positives
- Webhook secret verification uses `subtle::ConstantTimeEq`.
- Replay prevention via atomic `findOneAndUpdate` with `status: "pending"` filter.
- Chat ID binding prevents cross-user approval spoofing.
- Idempotency keys prevent duplicate approval requests.
- Link codes expire after 5 minutes and are single-use.
- Audit logging present for all state-changing operations (decisions, linking, disconnecting, revoking).
- Internal/database errors don't leak details to clients.

### Frontend
- Proper immutability (`readonly` on all interface fields, `readonly` arrays).
- No `console.log` statements.
- Zod validation with matching backend constraints.
- TanStack Query hooks with proper cache invalidation.
- Error handling using `ApiError` with toast notifications.
- Lazy-loaded pages via `lazy.ts`.
- Proper pagination with bounds checking.

### Indexes
- All indexes from the architecture plan are implemented in `db.rs`.
- TTL indexes for automatic cleanup of expired requests (90 days) and grants.
- Unique index on `idempotency_key` for duplicate prevention.
- Composite indexes for common query patterns.

### Routes
- Webhook routes correctly placed outside auth middleware.
- Notification/approval routes correctly under human-only middleware.
- Status polling endpoint correctly in delegated router for SA/OAuth client access.

---

## Checklist Summary

| Check | Status |
|-------|--------|
| No `skip_serializing` on model fields | PASS |
| bson datetime helpers on all `DateTime<Utc>` | PASS |
| `COLLECTION_NAME` on all models | PASS |
| Dedicated response structs (not serializing models) | PASS |
| Handler -> Service -> Model separation | PASS |
| UUID v4 `_id` strings | PASS |
| Webhook secret constant-time comparison | PASS (with minor length leak) |
| Replay prevention (atomic status update) | PASS |
| Chat ID binding verification | PASS |
| Idempotency key handling | PASS |
| Audit logging for state changes | PASS |
| No secret leakage in error messages | PASS |
| Authentication on all appropriate endpoints | PASS |
| Input validation (timeout/expiry bounds) | PASS (missing status filter validation) |
| No console.log in frontend | PASS |
| Immutability in frontend | PASS |
| Proper error handling | PASS |
| No hardcoded secrets | PASS |
| Rate limiting | PASS (global limiter covers webhook) |
