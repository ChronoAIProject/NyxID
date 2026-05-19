# NyxID Mobile App — Screen & API Inventory

> Inventory of all screens on the `main` branch, their features, UI elements, and API endpoints.

---

## Navigation Structure

**File:** `mobile/src/app/AppNavigator.tsx`

- Unauthenticated → `Auth` screen
- Authenticated → `Activity`, `ActivityDetail`, `AccountSettings`, legal screens
- Bottom nav shown on main tab screens only
- Deep linking: `nyxid://challenge/{challengeId}` (from push notifications)

---

## 1. Auth

### AuthHomeScreen (`features/auth/AuthHomeScreen.tsx`)

**Purpose:** Social-only login (Google, GitHub, Apple)

| UI Element | Detail |
|---|---|
| Section badge | "SOCIAL ONLY" |
| Title | "Continue to NyxID" |
| Buttons | Google, GitHub, Apple sign-in |
| Legal links | Terms of Service, Privacy Policy |
| Loading state | Spinner during OAuth flow |
| Toast overlay | Error/info messages |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/auth/social/{provider}?redirect_uri=nyxid://auth/social/callback` | Initiate OAuth (via WebBrowser) |

**Flow:** Tap provider → WebBrowser OAuth → deep link callback with tokens → `signInWithSession()` → activate push → navigate to Activity.

---

## 2. Activity (Unified Hub)

### ActivityScreen (`features/activity/ActivityScreen.tsx`)

**Purpose:** Primary screen — 3-tab hub replacing Dashboard + Challenges + Approvals.

| UI Element | Detail |
|---|---|
| Header | "Activity" + subtitle with pending/active counts |
| Segment control | 3 tabs: Pending (with count), Active (with count), History |
| Offline banner | Shown when network disconnected |
| Pending tab | FlatList of `ChallengeCard` with inline Approve/Deny buttons |
| Active tab | FlatList of `GrantCard` sorted by expiry (urgent first), with Revoke button |
| History tab | SectionList of `HistoryCard` grouped by date |
| Empty states | Per-tab empty messages |
| Pull-to-refresh | All tabs |
| Toast overlay | Success/error feedback |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/approvals/requests?status=pending&page=1&per_page=100` | Pending challenges |
| GET | `/approvals/grants?page=1&per_page=100` | Active grants |
| GET | `/approvals/requests?page=1&per_page=50` | History (filters out PENDING client-side) |
| GET | `/notifications/settings` | Grant expiry days config |
| POST | `/approvals/requests/{id}/decide` | Approve/Deny (with idempotency key) |
| DELETE | `/approvals/grants/{id}` | Revoke grant |

**Key behaviors:**
- Inline approve/deny on pending cards (no navigation needed)
- Inline revoke on active cards (with confirmation)
- `mutatingIds` Set prevents double-taps
- Auto-switches to "Active" tab after successful approval
- Network status monitoring via `useNetworkStatus()`

---

### ActivityDetailScreen (`features/activity/ActivityDetailScreen.tsx`)

**Purpose:** Full challenge detail with decision UI. Opened from pending card tap or push notification deep link.

| UI Element | Detail |
|---|---|
| Title | "Approval Detail" |
| Request context card | Action, Resource, Client, Risk Level (color-coded), Status, Grant Duration, Location |
| State notice | Shown if challenge already decided/expired |
| Action buttons | "Approve" + "Deny" (disabled if not PENDING) |
| Loading/error states | With retry |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/approvals/requests/{challengeId}` | Challenge detail |
| GET | `/notifications/settings` | Grant expiry config |
| POST | `/approvals/requests/{challengeId}/decide` | Submit decision (with idempotency key) |

**Flow:** View details → Approve/Deny → auto-navigate back to Activity after 600ms.

---

## 3. Legacy Challenge Screens (`features/challenges/`)

> These exist alongside the Activity module. They represent the original multi-screen flow.

### DashboardScreen (`features/challenges/DashboardScreen.tsx`)

**Purpose:** Summary metrics only — no actions.

| UI Element | Detail |
|---|---|
| Section badge | "SECURE" |
| Title | "Dashboard" |
| Security Status card | Pending Challenges count, Active Approvals count, Last Refresh (30s interval) |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/approvals/requests?status=pending&page=1&per_page=100` | Pending count |
| GET | `/approvals/grants?page=1&per_page=100` | Active count |

---

### ChallengesInboxScreen (`features/challenges/ChallengesInboxScreen.tsx`)

**Purpose:** List of pending challenges — tap to navigate to detail.

| UI Element | Detail |
|---|---|
| Section badge | "PENDING" |
| Title | "Pending Challenges" |
| Challenge cards | Action, Resource, Risk badge (HIGH/MEDIUM), Approval mode info |
| Empty state | "No pending challenges" |
| Pull-to-refresh | Yes |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/approvals/requests?status=pending&page=1&per_page=100` | Pending list |
| GET | `/notifications/settings` | Grant expiry config |

**Actions:** Tap card → navigate to `ChallengeDetail`.

---

### ChallengeDetailScreen (`features/challenges/ChallengeDetailScreen.tsx`)

**Purpose:** Full detail view with Approve/Deny.

| UI Element | Detail |
|---|---|
| Section badge | "DETAIL" |
| Request context card | Action, Resource, Client, Status, Grant Duration, Location |
| State notice | If already decided |
| Action buttons | Approve + Deny |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/approvals/requests/{challengeId}` | Challenge detail |
| GET | `/notifications/settings` | Grant expiry config |
| POST | `/approvals/requests/{challengeId}/decide` | Submit decision |

**Flow:** Approve/Deny → navigate to Dashboard.

---

### ChallengeMinimalScreen (`features/challenges/ChallengeMinimalScreen.tsx`)

**Purpose:** Quick-approve flow (streamlined view).

| UI Element | Detail |
|---|---|
| Section badge | "CHALLENGE" |
| Title | "Approve This Request?" |
| Action summary card | Action, Resource, Status, Grant Duration |
| Buttons | Approve, More Options, Deny |

**APIs:** Same as ChallengeDetailScreen.

**Flow:** Approve → Approvals screen | More Options → ChallengeOptions | Deny → Dashboard.

---

### ChallengeOptionsScreen (`features/challenges/ChallengeOptionsScreen.tsx`)

**Purpose:** Preview approval options before confirming.

| UI Element | Detail |
|---|---|
| Section badge | "OPTIONS" |
| Preview card | Action, Status, Grant Duration |
| Buttons | Approve, Back to Challenge |

**APIs:** Same as ChallengeDetailScreen.

---

### RevokeConfirmScreen (`features/challenges/RevokeConfirmScreen.tsx`)

**Purpose:** Confirmation before revoking an active grant.

| UI Element | Detail |
|---|---|
| Section badge | "WARNING" |
| Approval info card | Service name, Requester, Approval ID |
| Warning card | "This action takes effect immediately" |
| Buttons | Confirm Revoke (danger), Cancel |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/approvals/grants?page=1&per_page=100` | Find approval to display |
| DELETE | `/approvals/grants/{approvalId}` | Revoke |

---

### RevokeSuccessScreen (`features/challenges/RevokeSuccessScreen.tsx`)

**Purpose:** Post-revoke confirmation.

| UI Element | Detail |
|---|---|
| Section badge | "SUCCESS" |
| Checkmark | "Revoke Completed" |
| Toast | "Security update applied" |
| Buttons | Back to Approvals, Go Dashboard |

**APIs:** None.

---

## 4. Approvals

### ApprovalsScreen (`features/approvals/ApprovalsScreen.tsx`)

**Purpose:** Active grants list with revoke capability.

| UI Element | Detail |
|---|---|
| Section badge | "ACTIVE APPROVALS" |
| Title | "Approved Sessions" |
| Status card | Active count + Last Sync (30s refresh interval) |
| Approval cards | Service name, Requester (type + label), Grant expiry, Revoke button |
| Empty state | "No active approvals" |
| Pull-to-refresh | Yes |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/approvals/grants?page=1&per_page=100` | Active grants |

**Actions:** Tap Revoke → navigate to `RevokeConfirm`.

---

## 5. Account

### AccountSettingsScreen (`features/account/AccountSettingsScreen.tsx`)

**Purpose:** Profile info, sign out, account deletion.

| UI Element | Detail |
|---|---|
| Section badge | "ACCOUNT" |
| Account info card | Email, Display Name (with loading/error/retry) |
| Session card | Sign Out button |
| Danger zone card | Delete Account button + permanent deletion warning |

**APIs:**
| Method | Endpoint | Purpose |
|---|---|---|
| GET | `/users/me` | User profile |
| DELETE | `/users/me` | Delete account |

**Flows:**
- Sign Out → confirmation alert → `signOut()` → deactivate push → clear queries → Auth screen
- Delete Account → confirmation alert → `DELETE /users/me` → sign out → Auth screen

---

## 6. Legal (Static)

### PrivacyPolicyScreen (`features/legal/PrivacyPolicyScreen.tsx`)
- Static content, 13 sections, effective 2026-03-11
- Contact: privacy@chrono-ai.fun

### TermsOfServiceScreen (`features/legal/TermsOfServiceScreen.tsx`)
- Static content, 14 sections, effective 2026-03-11
- Contact: legal@chrono-ai.fun

---

## Complete API Endpoint Summary

| Method | Endpoint | Used By |
|---|---|---|
| GET | `/auth/social/{provider}` | AuthHomeScreen |
| GET | `/approvals/requests?status=pending` | Activity, Dashboard, Inbox |
| GET | `/approvals/requests` (all statuses) | Activity > History tab |
| GET | `/approvals/requests/{id}` | ActivityDetail, ChallengeDetail, ChallengeMinimal, ChallengeOptions |
| POST | `/approvals/requests/{id}/decide` | ActivityDetail, Activity (inline), ChallengeDetail, ChallengeMinimal, ChallengeOptions |
| GET | `/approvals/grants` | Activity, Dashboard, Approvals, RevokeConfirm |
| DELETE | `/approvals/grants/{id}` | Activity (inline), RevokeConfirm |
| GET | `/notifications/settings` | All challenge/approval screens |
| POST | `/notifications/devices` | After login (push registration) |
| DELETE | `/notifications/devices/current` | On logout (push deregistration) |
| GET | `/users/me` | AccountSettings |
| DELETE | `/users/me` | AccountSettings |

---

## Key Shared Utilities

| File | Purpose |
|---|---|
| `lib/api/mobileApi.ts` | High-level API methods |
| `lib/api/http.ts` | HTTP layer, token refresh, data sanitization |
| `lib/api/types.ts` | TypeScript types (ChallengeItem, ApprovalItem, etc.) |
| `lib/api/idempotency.ts` | Idempotency key generation for decisions |
| `lib/auth/sessionStore.ts` | Expo SecureStore session persistence |
| `lib/notifications/pushNotifications.ts` | Push notification registration |
| `features/challenges/challengeUiState.ts` | Shared UI helpers (formatGrantDuration, getChallengeActionState) |
| `features/activity/challengeUiState.ts` | Activity module's copy of UI helpers |

---

## Notes

1. **Two parallel screen sets exist:** The `features/activity/` module (ActivityScreen + ActivityDetailScreen) is a unified replacement for the original `features/challenges/` screens (Dashboard + Inbox + Detail + Minimal + Options) and `features/approvals/` (ApprovalsScreen). The Activity module consolidates all functionality into 2 screens with a 3-tab layout.

2. **History is already supported:** `GET /approvals/requests` without a status filter returns all requests. The mobile API client (`mobileApi.getHistory()`) filters out PENDING status client-side to show only decided/expired items.

3. **Idempotency:** All decision mutations include an `Idempotency-Key` header to prevent duplicate approvals/denials on retry.

4. **Push notification flow:** Login → register device token → receive challenge push → deep link to detail → decide → invalidate queries.
