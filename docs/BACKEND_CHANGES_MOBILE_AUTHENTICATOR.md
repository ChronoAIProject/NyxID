# Backend Changes — Mobile Authenticator & Approval Flow

**Commit**: `8990a80` (feat: add NyxID Mobile authenticator app and approval push flow)  
**Scope**: 9 files under `backend/` — approvals, users, social auth, notification, admin_user_service, models, routes.

---

## 1. Summary

This round of backend changes supports the NyxID Mobile Authenticator app:

- **Approvals**: Single-request fetch, decide with `duration_sec` and `Idempotency-Key`, decision replay handling.
- **Push**: Approval notifications now include `challenge_id` and `deeplink` for mobile deep link.
- **Users**: Self-service account deletion (`DELETE /users/me`).
- **Social auth**: Mobile client support via `client=mobile` and `redirect_uri`, redirect to `nyxid://` with tokens in query.

---

## 2. API Changes

### 2.1 Approvals

| Change | Before | After |
|--------|--------|--------|
| **GET** `/api/v1/approvals/requests/{request_id}` | — | **New.** Returns one approval request; 403 if not owned by current user. |
| **POST** `/api/v1/approvals/requests/{request_id}/decide` | Body: `{ approved: bool }` | Body: `{ approved: bool, duration_sec?: number }`. Optional header: `Idempotency-Key`. |

- **duration_sec**: When `approved === true`, grant expiry = `now + duration_sec` if provided and &gt; 0; otherwise uses channel default days.
- **Idempotency-Key**: Same key + same decision → idempotent success; same key + different decision → 409 Conflict.

### 2.2 Users

| Change | Before | After |
|--------|--------|--------|
| **DELETE** `/api/v1/users/me` | — | **New.** Deletes current user and cascade (sessions, API keys, MFA, connections, approval-related data, etc.). Returns `{ status, deleted_at }`. |

- Requires authenticated user. Audit event: `user.account.deleted` with `self_service: true`.

### 2.3 Social Auth (no new routes)

| Endpoint | Change |
|----------|--------|
| **GET** `/api/v1/auth/social/{provider}` | Query: optional `client`, `redirect_uri`. If `client=mobile`, `redirect_uri` is required and must be `nyxid://` or `exp://`; cookies store client and redirect for callback. |
| **GET** `/api/v1/auth/social/{provider}/callback` | On success, if mobile client was set: redirect to stored `redirect_uri` with `status=success&provider=…&user_id=…&access_token=…&refresh_token=…&expires_in=…`. Errors redirect to same URI with `status=error&error=…`. |

- Allowed mobile redirect schemes: `nyxid://`, `exp://` only.

---

## 3. Model Changes

### 3.1 `ApprovalRequest` (`backend/src/models/approval_request.rs`)

- **New field**: `decision_idempotency_key: Option<String>`  
  - Persisted when a decision is made with an `Idempotency-Key` header; used to detect idempotent replays.

---

## 4. Service-Layer Changes

### 4.1 `approval_service`

- **`process_decision`**  
  - New parameters: `duration_sec: Option<i64>`, `idempotency_key: Option<&str>`.  
  - When status is no longer `pending`, checks `is_idempotent_replay` (same key + same decision → return existing request; same key + different decision → 409).  
  - Grant expiry: `resolve_grant_expiry(now, duration_sec, default_days)` — uses `duration_sec` if present and positive, else default days.  
  - Writes `decision_idempotency_key` in the update when provided.

- **New helpers** (with tests):  
  - `is_idempotent_replay`  
  - `resolve_grant_expiry`

### 4.2 `notification_service`

- **`send_approval_notification`** (push payload):  
  - Adds `challenge_id` (same as `request_id`).  
  - Adds `deeplink`: `nyxid://challenge/{request_id}` for mobile to open the challenge screen.

### 4.3 `admin_user_service`

- **`delete_user_cascade`**: Logic moved into internal `delete_user_cascade_internal`.  
- **New**: `delete_current_user_cascade(db, user_id)` — same cascade as admin delete, for self-service `DELETE /users/me` (no admin check).

### 4.4 `telegram_poller`

- Call to `approval_service::process_decision` updated to pass `duration_sec: None`, `idempotency_key: None` (unchanged behavior).

---

## 5. Handler & Route Changes

### 5.1 `handlers/approvals.rs`

- **`list_requests`**: Response mapping factored into `to_approval_request_item`.
- **New**: `get_request_by_id` — `GET /approvals/requests/{request_id}`; uses `ensure_request_owned_by_user`.
- **`decide_request`**:  
  - Reads `Idempotency-Key` from headers.  
  - Validates `duration_sec` &gt; 0 when present.  
  - Passes `duration_sec` and `idempotency_key` into `process_decision`; `decision_channel` remains `"web"` for this handler.
- **Tests**: `ensure_request_owned_by_user`, `to_approval_request_item` mapping.

### 5.2 `handlers/users.rs`

- **New**: `delete_me` — `DELETE /users/me`; calls `admin_user_service::delete_current_user_cascade`, then audit log, returns `DeleteAccountResponse`.

### 5.3 `handlers/social_auth.rs`

- **`authorize`**: Accepts `AuthorizeQuery { client?, redirect_uri? }`. If `client == "mobile"`, validates `redirect_uri` (nyxid:// or exp://), sets cookies `nyx_social_client=mobile` and `nyx_social_redirect=<encoded>`.
- **`callback`**: Uses `resolve_redirect_target(frontend_url, headers)` — if mobile cookies present, redirect target is the stored `redirect_uri`; otherwise web `frontend_url`. Success redirect appends `status=success&provider=…&user_id=…&access_token=…&refresh_token=…&expires_in=…`; error redirect appends `status=error&error=…`. Clears `nyx_social_client` and `nyx_social_redirect` cookies after use.

### 5.4 `routes.rs`

- **Users**: `DELETE /me` → `handlers::users::delete_me`.
- **Approvals**: `GET /requests/{request_id}` → `handlers::approvals::get_request_by_id`.

---

## 6. File-Level Summary

| File | Change summary |
|------|----------------|
| `handlers/approvals.rs` | get_request_by_id, decide with idempotency + duration_sec, tests |
| `handlers/social_auth.rs` | Mobile query params, redirect cookies, redirect_target + token-in-query redirect |
| `handlers/users.rs` | delete_me, DeleteAccountResponse, audit |
| `models/approval_request.rs` | decision_idempotency_key |
| `routes.rs` | DELETE /users/me, GET /approvals/requests/{request_id} |
| `services/admin_user_service.rs` | delete_current_user_cascade, delete_user_cascade_internal refactor |
| `services/approval_service.rs` | process_decision idempotency + duration_sec, resolve_grant_expiry, tests |
| `services/notification_service.rs` | Push data: challenge_id, deeplink |
| `services/telegram_poller.rs` | process_decision call with new args (None, None) |

---

## 7. Compatibility

- **Backward compatible**: Existing callers of `POST .../decide` without `duration_sec` or `Idempotency-Key` behave as before (default grant days, no idempotency).  
- **New optional behaviour**: `duration_sec` and `Idempotency-Key` are optional.  
- **Social auth**: Web flow unchanged when `client` / `redirect_uri` are omitted.  
- **Database**: New field `decision_idempotency_key` is optional; existing documents remain valid.
