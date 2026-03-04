# NyxID Mobile Authenticator PRD

- Version: `v1.1 (Implementation-Aligned MVP)`
- Status: `Active`
- Owner: `NyxID Product & Engineering`
- Updated: `2026-03-03`

## 1. Background

NyxID backend can generate high-risk authorization requests, and users need a mobile-first approval endpoint for timely decisions.

Current product baseline focuses on **push-driven challenge handling** with secure decision and revoke flows on mobile.

## 2. Product Positioning

NyxID Mobile Authenticator is a **mobile authorization terminal**:

- Receive challenge notifications in the status bar.
- Open challenge detail via deep link.
- Approve or deny with explicit duration selection.
- View active approvals and revoke them.
- Manage account session and self-service account deletion.

## 3. Goals (Current MVP)

1. Build closed loop: `request created -> push/deep link -> decision -> backend enforcement`.
2. Keep challenge decision flow fast and reliable on mobile.
3. Ensure critical actions are secure and idempotent.
4. Support fallback browse/refresh when push is delayed.

## 4. Scope

### 4.1 In Scope (Implemented)

- Email/password login.
- Email/password registration (backend `/auth/register`).
- Push permission request + device token register/rotate after login.
- Deep link routing to challenge detail (`nyxid://challenge/{id}`).
- Challenge list/detail, approve/deny, duration selection.
- Decision idempotency via `Idempotency-Key`.
- Active approvals list + revoke.
- Pull-to-refresh and empty/error states.
- Session restore from secure storage and 401 refresh-retry behavior.
- Sign out and delete account.

### 4.2 Out of Scope (Current Release)

- Mobile audit timeline module.
- Push receipt reporting endpoint integration (`received/opened/actioned`).
- In-app social login completion (UI exists, backend flow not wired in app yet).
- Wearables native workflows.
- SMS/voice fallback channels.

## 5. Functional Requirements

- `FR-01` App must register device and push token after login.
- `FR-02` App must update backend when push token rotates.
- `FR-03` Notification payload must avoid sensitive raw data.
- `FR-04` Deep link opens challenge by ID and fetches detail from backend.  
  - Canonical runtime ID is `request_id`; `challenge_id` remains alias for compatibility.
- `FR-05` Approve/deny action must support idempotent submission.
- `FR-06` Expired or already-processed challenges must be blocked with clear UX.
- `FR-07` Approval list must support revoke and immediate state refresh.
- `FR-08` Account actions must support secure sign-out and self-delete.

## 6. Non-Functional Requirements

- Security: HTTPS required; auth tokens in secure OS storage.
- Reliability: idempotent decision handling; single-key replay safety.
- Availability: challenge/approval APIs follow production API SLO.
- Performance: challenge list/detail and decision path meet mobile UX latency targets.

## 7. UX Mapping (from design)

- Auth flow: `A1 ~ A5`
- Simplified challenge flow: `S0 ~ S5`
- Core modules: `C1 ~ C5` (with Audit currently deferred)
- Unified navigation: `NyxID Mobile - Unified Navigation`

Reference file:

- `/Users/potter/Desktop/sbt_project/NyxID/frontend/pencil/NyxID-Mobile.pen`

## 8. Acceptance Criteria (Current Release)

1. Push notification opens the correct challenge detail page via deep link.
2. Approve/deny updates backend and reflects on list state.
3. Duplicate taps with same idempotency key do not create duplicate approvals.
4. Custom duration (`duration_sec`) affects approval expiry.
5. Revoke immediately removes active approval validity.
6. Expired/processed challenges are non-actionable with clear messaging.
7. Session expiration triggers refresh attempt; failure signs user out safely.

## 9. Risks and Mitigations

- Push delays/failures -> inbox pull-to-refresh fallback.
- Token churn -> rotate token registration + secure local token sync.
- Duplicate decision race -> backend idempotency key + state guard.
- Ambiguous IDs (`challenge_id` vs `request_id`) -> keep alias compatibility and document canonical ID.

## 10. Deferred Items (Next Phase)

1. Mobile audit event timeline.
2. Push receipt pipeline (`received/opened/actioned`) and endpoint.
3. Full social login flow in app.
