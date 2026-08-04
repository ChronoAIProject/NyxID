# OAuth Chat Popup — Pilot Implementation Plan (PR-A + PR-B + PR-C)

Scope: §11 of `docs/OAUTH_POPUP_FLOW_PLAN.md` only — the chat-only pilot, one
branch (`feature/oauth-chat-popup-flow`), PR target `main`. Everything else in
the design doc (providers grid, SA providers, CLI wizard, social login, D-3 /
full retry transaction, Phase-2 embed) is OUT of scope. D-1 and D-2 stay
separate PRs (D-2 assessment: §7 below).

This plan was verified against HEAD (`fc93d51c`). Where the design doc drifted
from reality, the correction is recorded in §1 and the tasks follow the
corrected reality, not the doc.

---

## 1. Verified file/line map and corrections to the design doc

### 1.1 Backend — verified, matches the design doc

| Claim | Reality at HEAD |
|---|---|
| `OAuthState` fields | `backend/src/models/oauth_state.rs:7-59`. No `flow_kind` / `attempt_nonce` yet. All-fields struct literals (no `..Default`), so new fields are compiler-enforced at every constructor. |
| `OAuthState` constructors | Exactly five: `services/user_token_service.rs:731` (OAuth initiate), `:1021` (device-code initiate), `handlers/user_tokens.rs:1407` (test helper), `services/user_api_key_service.rs:2067` (test helper, used by `live_oauth_state` / `live_oauth_state_for_org`), `models/oauth_state.rs:72,99,126` (model tests). Plus the legacy-doc bson test at `models/oauth_state.rs:152`. |
| Initiate handler | `handlers/user_tokens.rs:361` `initiate_oauth_connect`; query struct `OAuthInitiateQuery` at `:71` (`redirect_path`, `scope`, `scope_override`, `target_org_id`, `key_id`). Calls service at `:395`. Service fn `services/user_token_service.rs:576` (10 params, already `#[allow(clippy::too_many_arguments)]` at `:567`, returns `String`). |
| Generic callback | `handlers/user_tokens.rs:492` `generic_oauth_callback_impl`. Branch order: provider-error (`:500`), missing code (`:550`), missing state (`:556`), peek (`:564`), session mismatch (`:587`), `handle_oauth_callback` (`:655`) with success / sync-failure / exchange-failure arms. POST form_post variant at `:811` funnels into the same impl. |
| Redirect builders | `redirect_callback` `:826` → `{FRONTEND_URL}/providers/callback?status=&message=`; `redirect_to_path` `:842` → `{FRONTEND_URL}{path}?provider_status=&message=`. |
| Session check | `ensure_callback_user_matches_state` `:1279` — returns `Ok(())` when `auth_user` is `None` (D-1, confirmed present, out of scope). |
| State claim | `services/user_token_service.rs:1550-1558` — atomic `consumed` flip only inside `handle_oauth_callback`; denial branch peeks without claiming (D-2, confirmed present). `peek_oauth_state` `:1504` is a bare `find_one` — it returns expired-but-unreaped and already-consumed rows, which §4 exploits for error attribution. |
| SA initiate | `handlers/admin_sa_providers.rs:263` `initiate_oauth_for_sa`, service call at `:281` — separate call site that must be updated for the service signature change (passes `None` flow; SA flows stay legacy). |
| Placeholder failing | `services/user_api_key_service.rs:621` `fail_oauth_placeholders` → `fail_connection_placeholder` `:585` only flips rows with `status ∈ {pending_auth}`. |
| Existing callback tests | `handlers/user_tokens.rs` tests mod (`:1292` on): `generic_oauth_callback_impl` invoked directly, `LOCATION` header asserted via `redirect_location` / `redirect_query_param` helpers, Mongo via `connect_test_database` + `test_app_state` (skips when no local Mongo). Model new tests on these. |

### 1.2 Frontend — verified, with corrections

| Claim | Reality at HEAD |
|---|---|
| `AddKeyDialog` OAuth handoff | `components/dashboard/add-key-dialog.tsx`: `OAuthStep` at `:1445`; `handleConnect` `:1508` awaits `ensureKey()` then `initiateOAuth.mutateAsync` (with `redirectPath: /keys/{key.id}`, `keyId`), then renders the `<a target="_blank">` anchor at `:1565-1573`; polling via `useKeyAuthorizationStatus(pendingKeyId, authorizing)` `:1503`. Outer dialog props at `:2449-2480`; `OAuthStep` mounted at `:3058-3097`. `cleanupPendingAuthKey` `:1387`. |
| Poll hook | `hooks/use-keys.ts:54-91` `useKeyAuthorizationStatus` — matches the doc. |
| Initiate hook | `hooks/use-providers.ts:110` `useInitiateOAuth` — builds query params; needs a `flow` param. `OAuthInitiateResponse` lives in `types/api.ts:637`. |
| Router | `router.tsx:97` `rootRoute`; `/oauth-consent` at `:165`; `dashboardLayout` + auth `beforeLoad` at `:319-352` (doc said `:318` — off by one, same code); `/providers/callback` nests under `providersRedirectRoute` (`:457`) under `dashboardLayout` — confirmed NOT a session-free precedent. Route tree assembly `:833-914`. Lazy-page registry: `pages/lazy.ts`. |
| Connect card | `components/assistant/blocks/connect-card.tsx` — `AddKeyDialog` mounted at `:246-255` ✓. |

**Correction C1 — the connect card does NOT open `ManageConnectionModal`.**
The design doc (§4.1) says the card "can already open [the modal] via its
'Manage' affordance (`blocks/connect-card.tsx:112`)". False at HEAD: `:112`
is `actionLabel` selection, and the card's Manage action **navigates** to
`/keys/$keyId` (`:175-180`). `ManageConnectionModal`
(`components/assistant/manage-connection-modal.tsx:458`, props
`{keyIds, serviceName, iconSlug?, onClose}`) is only mounted by
`components/assistant/plugins-view.tsx:359`. The pilot must newly mount it
from the connect card for the "View connection" CTA (task F9). Its props fit:
`keyIds` are `KeyInfo.id`s (UserService ids), exactly what `OAuthStep`'s
`pendingKeyId` holds.

**Correction C2 — `/oauth/launching` cannot exist; the interstitial is
`/oauth-launching`.** Prod nginx (`frontend/nginx.conf.template:35`) has
`location /oauth/ { proxy_pass BACKEND }` — every path under `/oauth/` is
proxied to Axum, which nests the OAuth2 IdP router at `/oauth`
(`backend/src/routes.rs:1426`). `/oauth/launching` would 404 at the backend
in prod. Bare `/oauth` does NOT match the `location /oauth/` prefix, so the
completion route `/oauth` (with or without query) falls through to the SPA
`try_files` — the completion route survives, the interstitial path does not.
Decision: interstitial route is **`/oauth-launching`** (root-level sibling,
like `/oauth-consent`). Zero infra changes.

**Correction C3 — vite dev proxy must mirror nginx.** `vite.config.ts:51`
proxies regex `^/oauth(?:/.*)?$`, which matches **bare `/oauth`** (unlike
prod nginx). In dev, refreshing the popup after the URL is scrubbed to
`/oauth`, or opening `/oauth` directly, would proxy to the backend and 404.
Change the key to `^/oauth/` so dev matches prod semantics (`/oauth/token`
etc. still proxied; bare `/oauth` served by the SPA). One line, task F11.

**Correction C4 — the window name cannot be `nyxid_oauth_{nonce}`.** The
popup must open synchronously in the click handler, but the server nonce only
exists after the initiate response. Use a client-minted launch id
(`crypto.randomUUID()`) for the window name — uniqueness per attempt is the
requirement (Codex 9/10), not which side minted it. The server
`attempt_nonce` remains the broadcast-correlation capability.

**Correction C5 — the initiate response must return `attempt_nonce`.** The
design stores the nonce on `OAuthState` and puts it in the completion URL,
but the opener must also know it to match broadcasts. `OAuthInitiateResponse`
(Rust + `types/api.ts`) gains `attempt_nonce` (optional; only set when `flow`
was sent). Legacy responses are byte-identical via
`#[serde(skip_serializing_if = "Option::is_none")]`.

**Correction C6 — "every terminal branch" means every *attributable*
branch in the pilot.** With missing state, or a state id whose row is gone
(unknown, or TTL-reaped — Mongo reaps within ~60s after `expires_at`), the
backend cannot know the flow was `cc`, and in the pilot the same callback
still serves live legacy flows — routing unattributable branches to `/oauth`
would break them. Those branches keep today's behavior byte-identical; a `cc`
popup landing there shows the legacy `/providers/callback` page inside the
popup and the opener's 2s poll still resolves the card. This is the pilot
analogue of design §3.2's accepted degraded case. Every branch where the row
resolves IS routed: expired-but-unreaped rows peek fine (→ `state_expired`),
and already-consumed rows peek fine (→ `state_replayed`).

**Correction C7 — retry cannot relaunch from the opener.** Design §4.2 has
the opener relaunch the popup on `action:"retry"` — but a broadcast handler
has no user activation in the opener's context, so `window.open` there gets
popup-blocked, and the opener's retained handle may be unusable post-COOP.
Instead the existing popup relaunches itself: opener re-runs initiate and
broadcasts the fresh authorize URL back; the popup (same-origin `/oauth`
page) sets `location.href` on itself, which needs no gesture and no handle.
Protocol in §5.3.

**Correction C8 — do not touch `hooks/use-keys.ts`.** It is in the CLI
wizard bundle source closure (`cli/src/wizard/bundle-meta/index.manifest`
line 67); editing it fails the Wizard Bundle Freshness job. All new frontend
logic goes in new files. None of the other files this pilot touches
(`add-key-dialog.tsx`, `use-providers.ts`, `router.tsx`, `connect-card.tsx`,
`vite.config.ts`… — verified against the manifest) are in the closure.
`vite.config.ts` is not `vite.wizard.config.ts` and is not hashed. Still run
`cargo test -p nyxid-cli --test wizard_bundle_freshness` before pushing; only
if red, `npm --prefix frontend run build:wizard` and commit `cli/src/wizard/`.

**Correction C9 — initiate-failure handling in the popup.** Design §5 says
"navigate the interstitial to an error state". Simpler and better UX for the
pilot: if `ensureKey()`/initiate fails after the sync-open, the opener closes
the popup and shows the error inline in the dialog (the existing `setError`
path at `add-key-dialog.tsx:1545-1553`). The user's context is the dialog;
an error marooned in a popup helps nobody.

---

## 2. Ordered task list

Order matters: backend first (frontend tests can then be written against the
real contract), each task compiles and passes tests on its own.

### B1 — `backend/src/models/oauth_flow_kind.rs` (new)

The flow-kind enum + wire tokens. Model layer: no business logic, no HTTP.

- `OAuthFlowKind` enum (§3 for exact shape) with `as_wire() -> &'static str`,
  `parse(&str) -> Option<Self>`, and `ALL_WIRE_TOKENS: [&str; 6]` (the TS
  parity anchor).
- Register `pub mod oauth_flow_kind;` in `models/mod.rs`.
- Unit tests: round-trip every variant; `parse` rejects unknown/empty/case
  variants; `ALL_WIRE_TOKENS` matches the documented set exactly.

### B2 — `backend/src/models/oauth_state.rs`

Add two serde-defaulted fields:

```rust
/// Wire token of the NyxID surface that initiated this flow
/// (`OAuthFlowKind`). Stored as the raw token so a row written by a
/// newer server with an unknown token still deserializes; parse at the
/// use site via `OAuthFlowKind::parse`. `None` = legacy / non-popup flow.
#[serde(default)]
pub flow_kind: Option<String>,
/// High-entropy per-attempt completion nonce (UUID v4). Minted at
/// initiate when `flow_kind` is set; echoed on the `/oauth` completion
/// URL so the opener can correlate broadcasts to the attempt it started.
/// Never an authorization credential — the DB row stays the source of truth.
#[serde(default)]
pub attempt_nonce: Option<String>,
```

Stored as `Option<String>`, not `Option<OAuthFlowKind>`, deliberately: an
unknown token must degrade to legacy fall-through, not a deserialization
error (forward compatibility when later PRs add variants).

- Update the three model-test constructors; extend
  `bson_backward_compat_missing_new_fields` to assert both new fields
  default to `None`; extend one roundtrip test with `Some` values.

### B3 — remaining `OAuthState` constructors

Compiler-driven: `services/user_token_service.rs:731` (values threaded in,
see B4) and `:1021` (device-code: both `None`),
`handlers/user_tokens.rs:1407` test helper (`None`, `None`),
`services/user_api_key_service.rs:2067` test helper (`None`, `None`).
`cargo build` must be clean before moving on.

### B4 — service: `initiate_oauth_connect` accepts the flow, mints the nonce

`services/user_token_service.rs:576`:

- New params: `flow_kind: Option<OAuthFlowKind>` (last position).
- When `flow_kind.is_some()`: mint `attempt_nonce = Uuid::new_v4().to_string()`.
- Store `flow_kind.map(|f| f.as_wire().to_string())` and the nonce on the
  `OAuthState` row.
- Return type changes from `String` to a dedicated struct:

```rust
pub struct OAuthInitiateResult {
    pub authorization_url: String,
    pub attempt_nonce: Option<String>,
}
```

- Update call sites: `handlers/user_tokens.rs:395` (threads the parsed
  flow, returns nonce in the response), `handlers/admin_sa_providers.rs:281`
  (pass `None`, use `.authorization_url` — SA behavior unchanged), handler
  test at `handlers/user_tokens.rs:1770`.
- The authorization URL itself is untouched — nothing new rides the provider
  redirect; `state` stays an opaque uuid (design §3.2).

### B5 — handler: initiate accepts `flow`, response carries the nonce

`handlers/user_tokens.rs`:

- `OAuthInitiateQuery` += `pub flow: Option<String>`.
- Parse: absent → `None`; present-but-unknown token →
  `AppError::ValidationError("unknown flow kind")` (explicit beats silent —
  a typo'd `flow` must not silently produce a legacy-routed callback).
  All six tokens parse; only `cc` changes callback routing (§2 B6), so a
  future surface sending `kc` today falls through to legacy behavior at the
  callback — additive by construction.
- `OAuthInitiateResponse` += `#[serde(skip_serializing_if = "Option::is_none")]
  pub attempt_nonce: Option<String>`.
- Audit metadata for `provider_oauth_initiated` may add `"flow": <token>`
  (fixed enum token, not free text) — metadata-only, optional.
- Device-code initiate (`DeviceCodeInitiateQuery`) is NOT given `flow` in the
  pilot — chat's device-code path keeps today's in-dialog code display, which
  already never navigates.

### B6 — handler: callback routes `cc` to `/oauth` on every attributable branch

`handlers/user_tokens.rs` `generic_oauth_callback_impl`. Structure the change
so non-`cc` code paths are provably untouched: at each branch, compute
`let cc = oauth_state.flow_kind.as_deref().and_then(OAuthFlowKind::parse)
== Some(OAuthFlowKind::ChatConnect)` from the peeked row **only where a row
is available**, and only the final redirect selection changes; every
side-effect (placeholder failing, audit rows, token writes, sync) stays
byte-identical for all flows including `cc`.

New builder alongside `redirect_callback` / `redirect_to_path`:

```rust
/// Redirect to the SPA popup-completion route. Fixed enum tokens and an
/// opaque nonce only — never free text, ids, or provider-controlled strings.
fn redirect_to_oauth_completion(
    frontend_url: &str,
    status: &str,               // "complete" | "error"
    flow: &str,                 // wire token
    code: Option<&str>,         // fixed error-code token, errors only
    nonce: Option<&str>,
) -> axum::response::Redirect  // → {FRONTEND_URL}/oauth?...
```

Branch-by-branch (all redirects carry `flow=cc` and `nonce=<row nonce>`;
`nonce` omitted only if the row unexpectedly has none):

| Branch (line at HEAD) | `cc` behavior | non-`cc` / unattributable |
|---|---|---|
| Provider error, row peeked ok (`:500-548`) | `status=error`, `code=access_denied` when `normalized_oauth_error_code(error) == "access_denied"`, else `provider_error`. `fail_oauth_placeholders` + audit unchanged. | unchanged (`redirect_callback` with message) |
| Provider error, no/unknown state | — (unattributable) | unchanged |
| Missing code, state present + row peeks ok | peek (new lookup on this branch, `cc` only decides the redirect), `status=error`, `code=provider_error` | unchanged (`Missing authorization code`) — including when the peek fails |
| Missing state (`:556`) | — (unattributable, C6) | unchanged (`Missing state parameter`) |
| Peek failed — unknown or reaped state (`:564-583`) | — (unattributable, C6) | unchanged (`Invalid or expired OAuth state`) |
| Session mismatch (`:587-651`) | `status=error`, `code=session_mismatch`. Audit + placeholder-failing (with the masked-email message, which stays in the DB error_message and audit as today) unchanged; the masked emails just no longer ride the URL for `cc`. | unchanged |
| Success (`:665-753`) | `status=complete` (`redirect_path` ignored for `cc`; the row's `redirect_path` is unset anyway since popup mode stops sending it, F8) | unchanged |
| Legacy-sync failure (`:695-747`) | `status=error`, `code=exchange_failed` (near-unreachable for `cc` — it always has `connection_id` — but handled) | unchanged |
| Exchange failure (`:755-802`) | If the peeked row was already expired (`oauth_state.expires_at < Utc::now()`): `code=state_expired`. Else if the peeked row had `consumed == true` (a prior claim won; this is a replay): `code=state_replayed`. Else `code=exchange_failed`. Placeholder-failing + audit unchanged. | unchanged |

No error-message string matching anywhere — expiry/replay classification uses
the already-peeked row's fields (C6 note: `peek_oauth_state` returns expired
and consumed rows, verified).

### B7 — backend tests

See §5.1.

### F1 — `frontend/src/schemas/oauth-popup.ts` + `frontend/src/types/oauth-popup.ts` (new)

- `OAUTH_FLOW_TOKENS = ["cc","kc","pc","sa","wz","sl"] as const` (TS mirror,
  §3), `OAUTH_ERROR_CODES` (§4), `OauthFlowKind` / `OauthErrorCode` types.
- Zod schema `oauthCompletionSearchSchema` parsing the `/oauth` search:
  `status` ∈ {complete,error}; unknown/absent `flow` → `undefined` (generic
  copy — never an error, per design §3); unknown/absent `code` →
  `undefined`; `nonce` optional string (bounded length, e.g. ≤ 64).
- Broadcast message types + type guards:
  `OAuthResultMessage {type:"oauth_result", status, flow?, code?, nonce}`,
  `OAuthActionMessage {type:"oauth_action", action:"view_result"|"retry"|"cancel", nonce}`,
  `OAuthAckMessage {type:"oauth_ack", nonce}`,
  `OAuthRetryMessage {type:"oauth_retry", nonce, nextNonce, url}` — with a
  parse guard that validates `url` is same-origin-relative or same-origin
  absolute before the popup will navigate to it (any same-origin page can
  post to the channel — Codex 11 — so the popup must never navigate to an
  unvalidated URL; nonce scoping is the capability, URL validation is the
  belt-and-suspenders).
- Vitest: `schemas/oauth-popup.test.ts` (§5.2).

### F2 — `frontend/src/lib/oauth-popup.ts` (new)

The popup manager. Framework-free module (testable without React):

- `OAUTH_CHANNEL = "nyxid.oauth"`, `openChannel()` guard for environments
  without `BroadcastChannel` (returns null; callers degrade to poll-only).
- `openOAuthPopup(): OAuthPopupHandle | null` — **synchronous**:
  `window.open("/oauth-launching", `nyxid_oauth_${launchId}`,
  "popup,width=760,height=820")` with a fresh `crypto.randomUUID()` launch
  id per call (C4). Returns `null` when blocked (caller falls back to the
  existing anchor flow). Handle: `{ launchId, navigate(url), close(),
  isClosed() }` — `isClosed()` documented as a **soft hint only** (COOP,
  Codex 7): it may inform a retry affordance, it must never fail a
  placeholder.
- `postResult` / `postAction` / `postAck` / `postRetry` helpers over the
  channel.

### F3 — `frontend/src/stores/oauth-popup-store.ts` (new)

Single-flow lock (design §5): Zustand store, app-level not component-level.

```ts
interface OAuthPopupAttempt {
  launchId: string;
  nonce: string | null;   // set once initiate resolves
  keyId: string | null;   // UserService id of the placeholder
  slug: string;           // catalog slug, for the "already in progress" copy
  startedAt: number;
}
// state: attempt: OAuthPopupAttempt | null
// actions: begin(attempt) -> boolean (false if one is active), setNonce/keyId, end(launchId)
```

`begin` refuses while an attempt is active; the caller surfaces "a connection
is already in progress" with a cancel affordance (`end` + close handle).
`end` is keyed by `launchId` so a stale attempt can't clear a newer one.

### F4 — `/oauth-launching` interstitial: `pages/oauth-launching.tsx` (new)

- First effect on mount: `window.opener = null` — this is the
  opener-severance point (Codex 6): the provider is navigated to from a
  context whose opener is already cleared, so a malicious catalog provider
  cannot reverse-tabnab the NyxID tab. The parent keeps its own handle to
  the popup (that direction is unaffected) and navigates it after initiate.
- Renders a minimal spinner card, "Connecting…" (no auth guard, no data
  fetching, no telemetry).

### F5 — `/oauth` completion page: `pages/oauth-complete.tsx` (new)

Mount sequence, in order:

1. Capture + parse search params via `oauthCompletionSearchSchema` into
   state (before scrubbing).
2. Broadcast `oauth_result` — first, before any delay.
3. `history.replaceState(null, "", "/oauth")` — scrub the query (history /
   log / referrer hygiene, Codex 20).
4. Render flow/code copy + CTAs (tables below), `role="status"
   aria-live="polite"`, focus on the primary CTA.

Behavior:

- **Success**: visible ~3s countdown then `window.close()`. Any pointerdown
  / keydown / focus interaction on the page cancels the countdown
  permanently. `prefers-reduced-motion`: same timing, no animated counter.
  Secondary CTA "Stay here" cancels the countdown.
- **Errors never auto-close.**
- ~300ms after any close attempt, if still alive: swap to "You can close
  this window" + manual close button; when `window.opener == null` (always
  true here — C2 note: opener is severed; the check is for direct opens
  where there was never an opener *and* no live receiver) also render a
  "Back to NyxID" link to `/`.
- **Direct open** (no `status` param): generic "Nothing to complete" copy +
  "Back to NyxID" link. No broadcast.

CTA protocol (design §4.1, with C7 retry):

- CTA click → `postAction({action, nonce})` → wait 400ms for `oauth_ack`
  with the same nonce → on ack `window.close()`; on timeout, self-navigate
  this window to the fallback destination.
- `view_result` (cc success primary, "View connection"): opener opens
  `ManageConnectionModal` in place (F9). No-ack fallback: navigate this
  window to `/keys` (the popup becomes the app only when the original tab is
  gone — design rule 1's stated exception).
- `retry` ("Try again" / "Start over" on retryable codes): opener re-runs
  the connect (F8) and answers `oauth_retry {nonce, nextNonce, url}`; the
  popup validates the URL (F1 guard), adopts `nextNonce`, and
  `location.href = url` — self-navigation, no gesture needed (C7). Timeout
  (2s): render "Couldn't restart from here — go back to your chat and click
  Connect again." (no auto-close).
- `cancel`: `postAction` then close after ack-or-400ms regardless (cancel
  must never strand the window).

Copy tables (pilot): flow `cc` → "Connected. Your credential is ready to
use." / service-generic; unknown/absent flow → generic. Error copy per §4
code table. The `flow`/`code` tokens select from these fixed tables and are
never rendered raw.

Register in `pages/lazy.ts`; routes in `router.tsx` as direct children of
`rootRoute` (before `dashboardLayout` in the children array, like the other
root routes): `oauthCompletionRoute` (`path: "/oauth"`, `validateSearch`
via the zod schema) and `oauthLaunchingRoute` (`path: "/oauth-launching"`,
`validateSearch: () => ({})`). No `beforeLoad` auth guards on either.

### F6 — `hooks/use-oauth-popup.ts` (new; NOT `use-keys.ts` — C8)

Opener-side receiver, nonce-scoped:

```ts
useOAuthPopupReceiver({
  nonce,                        // null until initiate resolves → hook inert
  keyId,
  onResult?: (msg) => void,     // UI nudge; poll remains the source of truth
  onViewResult?: () => boolean, // return true = handled (ack is sent)
  onRetry?: () => Promise<{ url: string; nextNonce: string } | null>,
})
```

- Ignores every message whose `nonce` doesn't match (Codex 11 — unknown
  nonce is *ignored*, no side effects).
- On `oauth_result` (matching): `queryClient.invalidateQueries(["keys"])` and
  `["keys", keyId]` — a wakeup for the existing poll, not a state
  transition; the DB row stays the source of truth (FI-004).
- On `oauth_action view_result`: call `onViewResult`; if it returns true,
  `postAck({nonce})`.
- On `oauth_action retry`: run `onRetry`; on success `postAck` +
  `postRetry({nonce, nextNonce, url})`, and the hook's owner swaps its
  active nonce to `nextNonce`.
- On `oauth_action cancel`: `postAck`, run cleanup callback.
- Tears down the channel subscription on unmount (a closed dialog stops
  acking; the popup's no-ack fallback covers it).

### F7 — `hooks/use-providers.ts` + `types/api.ts`

- `useInitiateOAuth` input += `readonly flow?: "cc"` → `query.set("flow",
  params.flow)`. (Type the param as the TS union later when more surfaces
  ship; pilot accepts only `"cc"`.)
- `types/api.ts` `OAuthInitiateResponse` += `readonly attempt_nonce?: string`.

### F8 — `components/dashboard/add-key-dialog.tsx`

- New props: `launch?: "popup" | "tab"` (default `"tab"`) and
  `flow?: "cc"` — threaded to `OAuthStep`. `/keys` and every existing caller
  pass nothing → today's behavior, byte-identical (default path does not
  touch the new code).
- `OAuthStep` popup branch in `handleConnect` (only when
  `launch === "popup"`):
  1. `begin()` on the single-flow store; refused → show "a connection is
     already in progress — cancel it first?" affordance, return.
  2. **Synchronously** `openOAuthPopup()` — before any await (Codex 5).
     `null` (blocked) → `end()` the lock and fall through to the existing
     anchor flow unchanged (design §5 fallback).
  3. `await ensureKey()`; `await initiateOAuth.mutateAsync({...,
     flow: "cc", keyId: key.id})` — **without `redirectPath`** (meaningless
     for `cc`; and against a not-yet-deployed backend the flow degrades to
     the legacy `/providers/callback` page inside the popup, which the poll
     still resolves — mixed-version safe).
  4. Store `nonce`/`keyId` on the attempt; `handle.navigate(authorization_url)`.
  5. On throw: close the popup, `end()` the lock, existing `setError` +
     `cleanupPendingAuthKey` path unchanged (C9).
  - The "authorizing" UI keeps the poll (`useKeyAuthorizationStatus` —
    unchanged, still the completion authority) and keeps the anchor link as
    a visible fallback ("Popup didn't open?"). Mount
    `useOAuthPopupReceiver` with `onRetry` = re-run initiate against the
    same placeholder (initiate already flips a `failed` placeholder back to
    pending server-side, `handlers/user_tokens.rs:409-422`) and
    `onViewResult` = delegate upward via a new optional dialog prop
    `onPopupViewResult?: (keyId: string) => boolean`.
  - `end()` the lock when the step unmounts or the flow reaches a terminal
    status.

### F9 — `components/assistant/blocks/connect-card.tsx`

- Pass `launch="popup"` and `flow="cc"` to its `AddKeyDialog`.
- Implement `onPopupViewResult`: set local state → render
  `<ManageConnectionModal keyIds={[keyId]} serviceName={serviceName}
  iconSlug={block.catalog_slug} onClose={...} />` **in place over the
  transcript** — no navigation (C1; design's two CTA rules). Return `true`.
- Everything else on the card (poll-driven state flips, guidance copy)
  unchanged.

### F10 — `vite.config.ts`

Proxy key `"^/oauth(?:/.*)?$"` → `"^/oauth/"` (C3). Verify dev login and
`/oauth/token` traffic still proxy (they have subpaths; they do).

### F11 — docs touch-up

Amend `docs/OAUTH_POPUP_FLOW_PLAN.md` with a short "pilot implementation
notes" pointer to this file for corrections C1-C9 (don't rewrite the design
doc's body in this PR).

### Final gate (before handing back)

1. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
   -D warnings`, `cargo test -p nyxid` (needs local Mongo via
   `docker compose up -d`; Mongo-less runs skip the integration tests —
   don't mistake skips for green).
2. `cd frontend && npm run lint && npm run test && npm run build` —
   **`npm run build` is the real type gate** (tsc -b with
   `noUncheckedIndexedAccess`); `tsc --noEmit` passing means nothing.
3. `cargo test -p nyxid-cli --test wizard_bundle_freshness` — only if red,
   rebuild + commit the wizard bundle (C8).
4. Before trusting any red vitest run: `lsof -ti :3000 :3001` — a live dev
   server answering test fetches with real 401s is a known flake source.

---

## 3. `flow_kind` enum — exact shape

Rust — `backend/src/models/oauth_flow_kind.rs` (single source of truth):

```rust
/// NyxID-owned identifier of which surface initiated an OAuth flow, used
/// by the callback to route popup flows to the `/oauth` completion route.
/// Wire tokens are stable and mirrored in
/// `frontend/src/types/oauth-popup.ts` (`OAUTH_FLOW_TOKENS`); both sides
/// pin the full set in tests. Pilot: only `ChatConnect` is emitted; the
/// rest are reserved (accepted at initiate, legacy-routed at callback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuthFlowKind {
    ChatConnect,           // "cc" — assistant chat connect card
    KeyConnect,            // "kc" — AddKeyDialog on /keys
    ProviderConnect,       // "pc" — admin providers grid
    ServiceAccountConnect, // "sa" — admin SA providers
    WizardConnect,         // "wz" — CLI wizard
    SocialLogin,           // "sl" — sign-in with GitHub/Google
}

impl OAuthFlowKind {
    pub const ALL_WIRE_TOKENS: [&'static str; 6] = ["cc", "kc", "pc", "sa", "wz", "sl"];
    pub fn as_wire(self) -> &'static str { /* match */ }
    pub fn parse(token: &str) -> Option<Self> { /* exact match, case-sensitive */ }
}
```

On `OAuthState` the field is `Option<String>` holding the wire token (B2
rationale: unknown tokens degrade, never fail deserialization).

TS mirror — `frontend/src/types/oauth-popup.ts`:

```ts
/** Mirrors backend/src/models/oauth_flow_kind.rs — both sides pin this set in tests. */
export const OAUTH_FLOW_TOKENS = ["cc", "kc", "pc", "sa", "wz", "sl"] as const;
export type OauthFlowKind = (typeof OAUTH_FLOW_TOKENS)[number];
```

"Parity test" means both sides assert the same literal list against their
own constant (cross-language execution isn't possible in either CI job); the
mirror comment points each file at the other.

---

## 4. `/oauth` query contract

```
/oauth?status=complete&flow=cc&nonce=<uuid-v4>
/oauth?status=error&flow=cc&code=<error_code>&nonce=<uuid-v4>
```

| Param | Values | Notes |
|---|---|---|
| `status` | `complete` \| `error` | required; anything else / absent → direct-open generic page, no broadcast |
| `flow` | wire token (§3) | pilot emits only `cc`; unknown/absent → generic copy |
| `code` | error-code set below | errors only; unknown/absent → generic error copy |
| `nonce` | opaque UUID v4 from `OAuthState.attempt_nonce` | correlation capability for the broadcast; receivers ignore unknown nonces |
| `provider` | catalog slug | **reserved, not emitted in the pilot** (would need a provider-slug lookup on every branch; the chat card already knows the service name) |

Nothing else, ever: no token material, no key/account/connection ids, no
free text (Codex 20). The page scrubs the query with `history.replaceState`
immediately after capture.

Fixed error-code set (full set reserved; **pilot emits** the marked ones):

| Code | Pilot emits | Produced by |
|---|---|---|
| `access_denied` | ✓ | provider callback `error=access_denied` |
| `provider_error` | ✓ | any other provider `error`; missing authorization code |
| `state_expired` | ✓ | exchange failed and the peeked row was past `expires_at` |
| `state_replayed` | ✓ | exchange failed and the peeked row was already `consumed` |
| `session_mismatch` | ✓ | browser session ≠ initiating user |
| `exchange_failed` | ✓ | token exchange / sync failure (residual) |
| `state_invalid` | — | reserved (unattributable in the pilot, C6) |
| `session_required` | — | reserved for D-1's fix |
| `server_error` | — | reserved (C9 keeps initiate failures in the dialog) |

Retryable codes (popup shows "Try again"/"Start over" wired to the retry
protocol): `access_denied`, `state_expired`, `state_replayed`,
`exchange_failed`, `provider_error`. `session_mismatch` primary CTA is
"Switch account" → `oauth_action` is not applicable; it renders a plain
link-out message (opener navigation to `/login` would dump the chat — for the
pilot the copy instructs, it does not navigate the opener).

Broadcast channel `"nyxid.oauth"` message shapes: §F1. All completion-page →
opener communication is BroadcastChannel only; `window.opener` is never used
(it is severed at the interstitial and by provider COOP anyway).

---

## 5. Test list

### 5.1 Rust (`cargo test -p nyxid`, live-Mongo integration style as at `handlers/user_tokens.rs:1292+`)

`models/oauth_flow_kind.rs`:
1. `wire_round_trip` — every variant `parse(as_wire(v)) == Some(v)`.
2. `parse_rejects_unknown` — `""`, `"CC"`, `"chat"`, `"xx"` → `None`.
3. `wire_token_set_pinned` — `ALL_WIRE_TOKENS == ["cc","kc","pc","sa","wz","sl"]` (the TS-parity anchor).

`models/oauth_state.rs`:
4. Extend `bson_roundtrip` with `flow_kind: Some("cc")` + nonce; assert restored.
5. Extend `bson_backward_compat_missing_new_fields` — legacy doc → both `None`.

`handlers/user_tokens.rs` (new; all model on the existing
`generic_oauth_callback_impl` tests, asserting the `LOCATION` header):
6. `initiate_with_flow_cc_stores_flow_and_nonce_and_returns_nonce` — call the
   initiate handler with `flow=cc`; assert the `OAuthState` row has
   `flow_kind == Some("cc")` and a non-empty `attempt_nonce`, and the JSON
   response carries the same nonce.
7. `initiate_with_unknown_flow_is_rejected` — `flow=zz` → `ValidationError`.
8. `initiate_without_flow_unchanged` — row has `None`/`None`; response JSON
   has **no** `attempt_nonce` key (serialize and assert absence — byte-parity
   with today).
9. `callback_success_cc_redirects_to_oauth_complete` — seeded `cc` state +
   pending key; assert `LOCATION` starts `{FRONTEND_URL}/oauth`, has
   `status=complete&flow=cc&nonce=<row nonce>`, and has no `message`/
   `provider_status` param. (Success requires a token exchange; follow the
   existing test suite's approach for the success path — if the suite only
   exercises error paths without an HTTP mock, split this into the
   redirect-builder unit test + the sync-failure branch test and note it in
   the PR.)
10. `callback_denial_cc_redirects_with_access_denied` — `error=access_denied`
    + `cc` row → `/oauth?...code=access_denied&nonce=...`; placeholder still
    flipped to `failed` with the same message as today (side-effect parity).
11. `callback_denial_non_cc_unchanged` — extend/keep
    `oauth_callback_denial_marks_placeholder_failed` asserting the exact
    legacy `LOCATION` (`/providers/callback?status=error&message=...`) —
    the byte-identity regression guard.
12. `callback_missing_code_cc_attributed` — no `code`, valid `cc` state →
    `/oauth?...code=provider_error`.
13. `callback_missing_code_without_state_unchanged` / missing-state branch →
    legacy `Missing authorization code` / `Missing state parameter` redirects
    byte-identical, even when the caller "was" a popup.
14. `callback_unknown_state_unchanged` — bogus state id → legacy
    `Invalid or expired OAuth state` redirect (unattributable, C6).
15. `callback_session_mismatch_cc` — mismatched `AuthUser` + `cc` row →
    `/oauth?...code=session_mismatch&nonce=...`; assert **no email fragment
    anywhere in the URL**; placeholder failed as today.
16. `callback_session_mismatch_non_cc_unchanged` — keep existing test green.
17. `callback_expired_row_cc_maps_state_expired` — seed `cc` row with
    `expires_at` in the past + a `code`; exchange path errors → `/oauth?...
    code=state_expired`.
18. `callback_consumed_row_cc_maps_state_replayed` — seed `cc` row with
    `consumed: true` → `/oauth?...code=state_replayed`.
19. `sa_initiate_unaffected` — existing SA handler tests keep passing with
    the new service signature (flow `None`); no new SA behavior.

### 5.2 Vitest (`frontend && npm run test`; jsdom has no `BroadcastChannel` — add a tiny test polyfill/mock in the new test files, and note the design-doc caveat that COOP/`closed` semantics are only validatable in a real browser, which stays on the manual matrix)

`schemas/oauth-popup.test.ts`:
1. Parses a complete success URL; parses each pilot error code.
2. Unknown `flow` / unknown `code` → `undefined` fields (generic), not a throw.
3. Missing `status` → direct-open shape.
4. `OAUTH_FLOW_TOKENS` pinned to `["cc","kc","pc","sa","wz","sl"]` (Rust-parity anchor).
5. `oauth_retry` URL guard: rejects cross-origin absolute URLs, accepts
   same-origin/relative.

`lib/oauth-popup.test.ts`:
6. `openOAuthPopup` calls `window.open` with `/oauth-launching`, a
   `popup,width=760,height=820` feature string, and a `nyxid_oauth_`-prefixed
   name; two calls → two distinct names.
7. Blocked open (`window.open` → null) returns null.

`stores/oauth-popup-store.test.ts`:
8. `begin` refuses while active; `end` with a stale `launchId` does not clear
   a newer attempt; `setNonce` attaches to the active attempt.

`pages/oauth-complete.test.tsx`:
9. Success mount: broadcasts `oauth_result` **before** the auto-close timer
   could fire; `history.replaceState` called scrubbing the query.
10. Success: countdown → `window.close` at ~3s (fake timers); a keydown
    before expiry cancels it and no close happens.
11. Error (`access_denied`, flow `cc`): renders the cc copy + "Try again";
    never calls `window.close` no matter how long timers advance.
12. Unknown flow token: generic copy, token never rendered in the DOM.
13. CTA ack path: click "View connection" → `oauth_action` posted; ack with
    matching nonce within 400ms → `window.close`; no ack → navigation
    fallback to `/keys` (assert via mocked navigation).
14. Retry path: `oauth_retry` with matching nonce and same-origin URL →
    `location.href` set; cross-origin URL → ignored + error copy.
15. Direct open (no params): generic page, no broadcast posted.

`hooks/use-oauth-popup.test.ts` (renderHook):
16. `oauth_result` with matching nonce → invalidates `["keys"]` and
    `["keys", keyId]`; non-matching nonce → nothing (spy on queryClient).
17. `oauth_action view_result` → `onViewResult` called; returns true → ack
    posted with the nonce; unknown nonce → neither.
18. `oauth_action retry` → `onRetry` result broadcast as `oauth_retry` with
    `nextNonce`.

`components/dashboard/add-key-dialog` (extend existing coverage or a focused
new test file for `OAuthStep` popup mode):
19. `launch="popup"`: clicking Connect calls `window.open` **synchronously**
    (assert it happened before the mocked initiate promise resolves).
20. Blocked popup → anchor fallback UI renders exactly as the default path;
    lock released.
21. Initiate rejection → popup handle closed, inline error shown (C9), lock
    released.
22. Default (`launch` absent) → `window.open` never called; initiate called
    **with** `redirectPath` and **without** `flow` (legacy byte-parity).
23. Popup mode → initiate called with `flow: "cc"`, `keyId`, and **no**
    `redirectPath`.

`components/assistant/blocks/connect-card`:
24. Passes `launch="popup"` / `flow="cc"` to `AddKeyDialog`.
25. `onPopupViewResult` opens `ManageConnectionModal` with the attempt's
    keyId and **does not navigate** (router mock untouched).

---

## 6. Risk list — what could regress, and how the plan prevents it

1. **Legacy OAuth flows change behavior** (providers grid, SA, wizard,
   social, `/keys` dialog). Prevention: the callback change is
   redirect-selection only, gated on `flow_kind == Some("cc")` read from the
   row; every side-effect line is shared; tests 11/13/14/16/19 assert exact
   legacy `LOCATION` strings; test 8 asserts the initiate response JSON is
   key-identical when `flow` is absent; the frontend default path never
   enters the new code (test 22).
2. **A missed `OAuthState` constructor.** Rust struct literals without
   `..Default` — the compiler finds every site (B3); serde defaults cover
   only old BSON, which test 5 pins.
3. **`/oauth` route collides with the backend IdP surface.** C2/C3: the
   completion route is exactly `/oauth` (SPA-served in prod; vite aligned in
   F10); the interstitial avoids `/oauth/*` entirely. Residual risk: other
   reverse proxies in front of prod (Cloudflare) routing `/oauth` — the
   known prod edge config (see memory: edge serves CORS headers) should be
   spot-checked at deploy: `curl -s https://<prod>/oauth | grep -q index`
   equivalent.
4. **Safari blocks the popup anyway** (activation lost). Prevention: open is
   the first statement in the click handler (test 19); blocked-open falls
   back to today's anchor flow (test 20), and polling remains the completion
   authority in every case.
5. **Broadcast spoofing / cross-talk** (Codex 11): any same-origin page can
   post to the channel. Prevention: receivers act only on the attempt nonce
   they hold (tests 16/17); results only trigger query invalidation (DB is
   truth); the popup never navigates to an unvalidated URL (tests 5/14).
6. **Chat tab navigated away / conversation destroyed** — the failure this
   design exists to prevent. Prevention: cc `view_result` opens
   `ManageConnectionModal` in place (test 25); the popup self-navigates only
   on ack timeout (original tab gone).
7. **Dialog closed while the popup is mid-consent.** Receiver unmounts →
   CTAs fall back to self-navigation; the card still flips because the keys
   list refetches on window focus when the popup closes and the DB row is
   authoritative. No placeholder cleanup runs on dialog-close during
   authorizing (unchanged from today).
8. **Wizard bundle freshness CI red on an unrelated-looking PR.** C8:
   `use-keys.ts` untouched; freshness test run locally before push; rebuild
   only on red.
9. **Frontend type gate.** `npm run build` (tsc -b, strict) is in the final
   gate; new files use `readonly` prop conventions matching the codebase.
10. **Mixed-version deploys** (new FE, old BE or vice versa). New FE + old
    BE: `flow` is an unknown query param to old axum `Query` → ignored;
    popup degrades to legacy `/providers/callback` page inside the popup;
    poll completes. Old FE + new BE: no `flow` sent → rows have `None` →
    byte-identical legacy behavior. No ordering constraint.
11. **Rate limiting / SEC-9 TODO**: initiate/callback rate limits unchanged;
    the popup flow adds no new endpoints and no new unauthenticated surface
    (`/oauth` and `/oauth-launching` are static SPA pages).

---

## 7. D-2 assessment — is it load-bearing for the `cc` denial → `/oauth` → "Try again" path?

**No. Recommendation: keep D-2 out of this branch, as its own PR, per the
design doc's §7/§12.** Reasoning, verified against HEAD:

- **Attribution does not need the claim.** The denial branch already peeks
  the row (`peek_oauth_state`, a bare `find_one`), which is exactly what the
  pilot uses to read `flow_kind`/`attempt_nonce`. D-2 unfixed means the row
  is *peeked but not consumed* — attribution works either way.
- **Retry correctness does not depend on the old row.** "Try again" makes
  the opener re-run initiate, which (a) mints a **fresh** state row with a
  fresh nonce and (b) already flips a `failed` placeholder back to
  `pending_auth` (`handlers/user_tokens.rs:409-422`,
  `mark_provider_connection_pending`). The stale unconsumed row plays no
  role in the new attempt; the popup navigates to the fresh authorize URL.
- **What D-2 actually leaves open, unchanged by the pilot:** a holder of the
  old `state` value can replay `?error=access_denied&state=…` and re-fail a
  placeholder — but `fail_connection_placeholder` only touches rows still in
  `pending_auth` (verified, `user_api_key_service.rs:585-598`), so the
  blast radius is "an in-flight attempt sharing that connection_id gets its
  placeholder flipped to failed", identical to today's tab flow. The pilot
  neither widens the replay window (the denial URL sits in the popup's
  history exactly as it sits in the tab's history today) nor adds new
  consumers of the stale row.
- **Marginal argument for folding a cc-only claim in:** it would make `cc`
  denial single-use. Rejected: it would fork claim semantics between `cc`
  and legacy inside one handler — precisely the kind of divergence the
  pilot's "all other flows byte-identical" invariant exists to avoid — and
  D-2's real fix (atomic claim on **every** terminal branch, all flows) is
  strictly better and already specced as an independent PR.
- **One coordination note for the future D-2 PR:** when denial switches from
  peek to claim-and-delete, the `cc` routing must read `flow_kind` /
  `attempt_nonce` from the *claimed* document (e.g. `find_one_and_update`'s
  returned row), not re-peek after deletion. Left as a comment-worthy note
  in the D-2 PR, nothing to do here.
