# NyxID Mobile Authenticator Technical Specification

- Version: `v1.1 (Implementation-Aligned MVP)`
- Status: `Active`
- Owner: `NyxID Engineering`
- Updated: `2026-03-03`

## 1. Objective

Document the **current production-aligned implementation** for NyxID mobile approval flows:

- Backend creates approval requests (mobile challenge equivalent).
- Push delivers request notifications.
- Mobile opens detail via deep link and submits decision.
- Backend enforces decision, supports idempotent replay, and manages active grants.

## 2. Architecture Overview

### 2.1 Backend Services (Current)

- `approval_service`: create/list/decide/expire approval requests.
- `notification_service`: send push payloads to registered devices.
- `device_tokens` handlers: register/list/remove push devices.
- `auth` handlers: login/register/refresh.
- `users` handlers: self profile and `DELETE /users/me`.

### 2.2 Mobile Modules (Current)

- Notification runtime + permission handling.
- Deep link resolver (`nyxid://challenge/{id}`).
- Challenge inbox/minimal/detail/options screens.
- Decision submission with `Idempotency-Key` and optional `duration_sec`.
- Approvals list + revoke flow.
- Secure session store + 401 refresh-retry wrapper.

## 3. Domain Model (Current Mapping)

### 3.1 Request Identity

- Canonical runtime ID: `request_id` (stored as `approval_requests._id`).
- Compatibility alias in push/mobile payload: `challenge_id` (same value).

### 3.2 `approval_requests` (Challenge Equivalent)

- `_id` (`request_id`)
- `user_id`
- `service_id`, `service_name`, `service_slug`
- `requester_type`, `requester_id`, `requester_label`
- `operation_summary`
- `status`: `pending | approved | rejected | expired`
- `idempotency_key` (request creation dedupe)
- `decision_idempotency_key` (decision replay dedupe)
- `expires_at`, `decided_at`, `created_at`

### 3.3 `approval_grants`

- `_id` (`grant_id`)
- `approval_request_id`
- `user_id`
- `service_id`, `service_name`
- `requester_type`, `requester_id`, `requester_label`
- `granted_at`, `expires_at`
- `revoked` (bool)

### 3.4 Notification Channel / Devices

- User-scoped push device list via `/notifications/devices`.
- Platform values normalized as `apns` / `fcm`.

## 4. State Machines

### 4.1 Approval Request

- `pending -> approved`
- `pending -> rejected`
- `pending -> expired`

Constraints:

- Only `pending` accepts first write.
- Replays with same `decision_idempotency_key` and same decision are accepted as idempotent.
- Replays with same key but different decision return conflict.

### 4.2 Approval Grant

- `active -> revoked`
- `active -> expired` (time-based)

## 5. API Contracts (Current)

All paths are under `/api/v1`.

### 5.1 Auth & Session

- `POST /auth/login`
- `POST /auth/register`
- `POST /auth/refresh` (cookie-based refresh token rotation)

### 5.2 Device / Push Token

- `POST /notifications/devices`
  - Used for both initial register and rotate (with changed token).
- `GET /notifications/devices`
- `DELETE /notifications/devices/{device_id}`

### 5.3 Challenge (Request) APIs

- `GET /approvals/requests?status=pending&page=&per_page=`
- `GET /approvals/requests/{request_id}`
- `POST /approvals/requests/{request_id}/decide`
  - Headers: `Idempotency-Key: <string>`
  - Body:
    - `approved: boolean`
    - `duration_sec?: number` (approve path; positive integer)

### 5.4 Approval Grant APIs

- `GET /approvals/grants?page=&per_page=`
- `DELETE /approvals/grants/{grant_id}`

### 5.5 Account

- `DELETE /users/me`

## 6. Push Notification Payload (Current)

Common fields currently consumed by mobile deep link parser:

- `deeplink` or `url` (if provided)
- `challenge_id` (legacy alias)
- `challengeId` (camelCase alias)
- `request_id` (canonical)

Deep link target:

- `nyxid://challenge/{id}` -> challenge detail route

## 7. Security & Consistency

- Access token sent as Bearer for protected APIs.
- Mobile stores auth tokens in secure storage.
- Request wrapper behavior:
  - protected request receives `401` -> call `/auth/refresh` -> retry once.
  - refresh/retry failure -> clear local auth session.
- Decision safety:
  - ownership check
  - pending-state guard
  - idempotency replay handling

## 8. Reliability

- Pull-to-refresh for inbox/approvals as push fallback.
- Explicit UI handling for expired/already-processed/not-found request states.
- Duration defaults to server grant policy when `duration_sec` is absent.

## 9. Deferred / Not in Current Release

- Mobile audit timeline API and screen (`/mobile/audit-events` style module).
- Push receipt ingestion (`received/opened/actioned` endpoint).
- In-app social auth completion (UI placeholders remain).

## 10. Testing Status

### 10.1 Automated

- Backend compiles: `cargo check`
- Mobile compiles: `npm run typecheck`
- Approval service tests include:
  - idempotent replay (same key/same decision)
  - replay conflict (same key/different decision)
  - duration expiry calculation

### 10.2 Recommended E2E

1. Push click -> deep link -> detail page.
2. Approve with custom duration and verify grant expiry.
3. Repeat approve with same idempotency key (no duplicate grant).
4. Expired request action blocked in app.

## 11. Implementation Checklist (Current)

- [x] Device registry APIs.
- [x] Deep link + decision flow integration.
- [x] Decision idempotency key handling.
- [x] Decision duration (`duration_sec`) handling.
- [x] Session refresh-retry wrapper on mobile.
- [ ] Push receipt endpoint and client mapping (deferred).
- [ ] Mobile audit timeline endpoint/UI (deferred).
