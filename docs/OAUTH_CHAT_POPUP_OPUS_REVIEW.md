# Adversarial Review: Chat OAuth Popup Pilot Implementation

Reviewed `41b46065..1d8c036d` against `origin/main` (52 files, ~2661 insertions),
reading the diff first and the plan/SOL-review documents afterwards. Verification
run locally: `cargo clippy --workspace --all-targets` (clean),
`cargo test -p nyxid models::` (394 passed), `npx vitest run` (2580/2581).

Summary: the seven claimed P1 fixes are **4 fixed, 3 partially fixed**. The
security posture of the popup protocol itself is sound — opener severance, ack
authentication, and retry-URL binding all hold up under attack. The three
surviving P1s are on the *server* side of the attempt lifecycle: the guard is
one-directional, a live credential is destroyed before the user has approved
anything, and one callback branch escapes popup routing entirely.

---

## Verdict on the 7 claimed P1 fixes

| # | Claim | Status |
|---|-------|--------|
| P1-1 | Public routes (`/oauth`, `/oauth-launching`) | **FIXED** |
| P1-2 | Opener severance before external navigation | **FIXED** |
| P1-3 | Retry URL validation permits real providers | **FIXED** (one degradation, F-8) |
| P1-4 | Attempt-generation guard | **PARTIALLY FIXED** (F-1, F-3, F-4) |
| P1-5 | Legacy BSON identity | **FIXED** |
| P1-6 | Capability channel | **PARTIALLY FIXED** (F-5) |
| P1-7 | State prefix classifies without a DB call | **PARTIALLY FIXED** — holds for the missing-code branch only (F-2) |

---

## P1 findings

### F-1 [P1] Starting a chat attempt destroys a live credential before the user approves anything

`backend/src/services/user_api_key_service.rs:693-729`
(`begin_chat_oauth_attempt`), called from
`backend/src/services/user_token_service.rs:788-801`.

`begin_chat_oauth_attempt` matches on `{connection_id, credential_type != node_managed}`
with **no status predicate** and unconditionally `$set`s
`access_token_encrypted: null`, `refresh_token_encrypted: null`,
`token_scopes: null`, `expires_at: null`, `status: "pending_auth"`. It runs at
*initiate* time — the moment the user clicks Connect, before the popup has even
reached the provider.

The legacy path is deliberately not like this. `handlers/user_tokens.rs:422-428`
gates `mark_provider_connection_pending` on
`status ∈ {failed, refresh_failed, expired, pending_auth}`, so an **active** key
is never wiped by merely starting a flow. The chat path removes that gate.

Failure scenario (the ordinary one, not a corner case):
`ConnectCard` shows "Reconnect" whenever `block.reason_code === "NYXID_UNAUTHORIZED"`
(`frontend/src/components/assistant/blocks/connect-card.tsx:70-84`). That reason
code comes from a downstream 401 — it does **not** imply the NyxID row is
unhealthy. A GitHub token that 401s for a scope reason lives on a
`status: "active"` key. User clicks Reconnect → popup opens → GitHub consent
screen → user clicks **Cancel**, or closes the popup, or the state expires. The
key is now `pending_auth` with both tokens nulled. The previously-working
connection is destroyed, permanently, by an action the user cancelled.

This also makes user-facing copy false: `frontend/src/pages/oauth-complete.tsx:51-54`
tells the user on `exchange_failed` that *"Your existing connection was not
replaced."* It was already erased at initiate.

REFUTES the "each chat attempt stamps a server nonce" framing insofar as it
describes a *stamp*: it is a stamp **plus a destructive reset**, and the
destructive half has no legacy precedent.

Fix direction: split the stamp from the reset — stamp `oauth_attempt_nonce`
unconditionally, apply the token-clearing reset only under the same status gate
the legacy path uses, and only on the callback path once an exchange has
actually succeeded.

### F-2 [P1] Provider denial with a reaped state escapes popup routing and renders the dashboard inside the popup

`backend/src/handlers/user_tokens.rs:529-582`.

The denial branch classifies `cc` only from a successful `peek_oauth_state`:

```rust
match user_token_service::peek_oauth_state(&state.db, state_param).await {
    Ok(oauth_state) => { chat_completion = is_chat_connect_state(&oauth_state); ... }
    Err(e) => state_lookup_error = Some(e.to_string()),   // <-- chat_completion stays false
}
```

The two sibling branches — missing-code (`:549-563`) and claim-failed
(`:625-633`) — both fall back to `chat_attempt_nonce_from_state(state_param)`
and route to `/oauth`. This one does not.

Failure scenario: `OAuthState` TTL is 10 minutes
(`user_token_service.rs:745`, `expires_at = now + Duration::minutes(10)`), and
Mongo reaps shortly after. A user opens the popup, gets distracted, comes back
after ~11 minutes and clicks **Deny** on the GitHub consent screen. `peek` returns
`NotFound` → `chat_completion == false` → `redirect_callback` → the popup lands on
`{FRONTEND_URL}/providers/callback?status=error&message=...`.

That route is a child of `dashboardLayout` (`frontend/src/router.tsx:444-465`), so
the 760×820 popup renders the **entire NyxID dashboard shell** with a "Back to
Providers" button — precisely the duplicate-app-window the design forbids. It never
broadcasts, never closes, and the opener's `useKeyAuthorizationStatus` poll
(`frontend/src/hooks/use-keys.ts:70-76`) spins on `pending_auth` forever. Combined
with F-1, the user's credential is already gone at this point.

REFUTES the implementation plan's own routing table, which claims
"Peek failed — unknown or reaped state | valid `1cc_<uuid>` → `state_invalid`
with suffix nonce". That is true of the claim-failed site, not of this one.
No test covers it.

Fix: in the `Err(_)` arm, apply the same `chat_attempt_nonce_from_state`
fallback the other two branches already use.

### F-3 [P1] The generation guard is one-directional — legacy paths mutate chat-guarded keys and never clear the nonce

`backend/src/services/user_api_key_service.rs:493-524` (`write_oauth_tokens_to_key`),
`:1076-1108` (`reset_provider_api_key_state`), `fail_oauth_placeholders`.

All three still filter on `connection_id` alone. None of them reads or clears
`oauth_attempt_nonce`. The same `UserApiKey` / `connection_id` is reachable from
both the chat card (`flow=cc`) and the `/keys` dashboard (no `flow`) — the chat
card resolves `matchingKey` out of the same `useKeys()` list the dashboard renders.

Failure scenario:
1. Chat starts attempt A on connection C → key `pending_auth`, `oauth_attempt_nonce = A`, tokens nulled (F-1).
2. User gives up on the popup, opens `/keys`, and reconnects the same connection from the dashboard (legacy, `flow` absent). `mark_provider_connection_pending` → `reset_provider_api_key_state` fires (status is `pending_auth`) and **leaves `oauth_attempt_nonce = A` in place**.
3. The legacy callback succeeds → `write_oauth_tokens_to_key` writes the new tokens, `status: "active"`. `oauth_attempt_nonce` is *still* `A`.
4. The user returns to the still-open popup from step 1 and completes it. `write_chat_oauth_tokens_to_key` filters `{connection_id: C, oauth_attempt_nonce: A, status: {$nin: ["revoked","failed"]}}` — `active` is not in that list, so it **matches** and overwrites the tokens the dashboard just obtained.

Net effect: the credential the user last consented to (dashboard scope set) is
silently replaced by an older attempt's grant (chat scope set). The reverse also
holds: a legacy `fail_oauth_placeholders` for connection C will fail the row while
a *current* chat attempt B is in flight, with no nonce check.

Directly REFUTES the review question "Can a legacy (non-chat) mutation path still
clobber a chat-guarded key?" — yes, and the guard does not detect it.

Fix: have `reset_provider_api_key_state` and `write_oauth_tokens_to_key`
`$unset: {oauth_attempt_nonce: ""}` so any legacy write invalidates the in-flight
chat generation.

---

## P2 findings

### F-4 [P2] The stamp is atomic per-document but not atomic with the state insert

`backend/src/services/user_token_service.rs:766-801`. `insert_one(&oauth_state)`
and `begin_chat_oauth_attempt` are two separate round-trips.

Two concurrent chat initiates on the same connection (two browser tabs — the
`oauth-popup-store` single-flow lock is per-tab, `frontend/src/stores/oauth-popup-store.ts:19-25`)
can interleave: A inserts, B inserts, B stamps, A stamps. The key now belongs to
attempt A while the user is following B's authorization URL. When B completes,
`write_chat_oauth_tokens_to_key` returns `matched_count == 0` → the handler raises
`BadRequest("OAuth attempt is no longer current")` (`user_token_service.rs:1826-1830`).

The provider access token was already minted and exchanged at that point. It is
discarded without an upstream revocation call, leaving an orphaned live grant at
GitHub/Google, and the key stays destroyed per F-1. Answering the sub-question
explicitly: an **unstamped/`None` nonce does not fall through to an unguarded
filter** — `handle_oauth_callback` errors with `BadRequest("Invalid chat OAuth
attempt")` and `fail_callback_placeholders` returns `Ok(0)`. That part of the
claim holds.

### F-5 [P2] The channel capability is in the `/oauth` URL and is the same value handed to the OAuth provider

`backend/src/handlers/user_tokens.rs:984-1004` appends `nonce=<uuid>` to the
completion redirect. `backend/src/services/user_token_service.rs:742-745` makes the
OAuth `state` parameter literally `1cc_<attempt_nonce>`.

So the value that names the BroadcastChannel (`frontend/src/lib/oauth-popup.ts:25-29`,
`nyxid.oauth.<nonce>`) is:
- sent to GitHub/Google as `state` and recorded in their logs;
- emitted in the backend's `Location` header;
- requested as `GET /oauth?status=…&nonce=…` against nginx, whose default
  `combined` log format records both that request line *and* the same URL as the
  `Referer` of the same-origin bootstrap assets fetched before
  `history.replaceState` runs (`frontend/nginx.conf.template` sets no
  `access_log` override and no `Cache-Control: no-store` for `/oauth`).

Payload hygiene is genuinely correct — the nonce never appears in an
`oauth_result`, `oauth_action`, `oauth_ack`, or `oauth_retry` body, and the next
capability is transferred only over the old channel
(`frontend/src/hooks/use-oauth-popup.ts:73-78`). But the claim that the nonce
"is never leaked into the /oauth URL, the DOM, telemetry, logs" is **false** for
the URL and for logs. The implementation plan does accept the infrastructure-log
half as P2; the URL half is not called out.

Practical exploitability is bounded: `BroadcastChannel` is origin-scoped and
`X-Frame-Options: DENY` blocks framing `/oauth`, so a provider or log reader who
learns the nonce cannot join the channel without same-origin script execution.
The correct characterization is that the nonce is a *correlation identifier*, not
a secret capability — the design doc should say so rather than claim log hygiene.

Note `frontend/src/pages/oauth-complete.test.tsx:52-68` is titled
"broadcasts a nonce-free wakeup and scrubs the URL" but constructs the URL *with*
`?nonce=`. It encodes the leak as the expected contract rather than catching it.

### F-6 [P2] The completion page's success state is fully attacker-assertable

`/oauth` is public (`frontend/src/lib/public-paths.ts:24`) and renders purely from
query params. Anyone who knows a nonce — including the OAuth provider itself,
which received it as `state` and controls where it redirects — can send the popup
to `{FRONTEND_URL}/oauth?status=complete&flow=cc&nonce=<known>` and the page will
display **"Connection complete — The credential is secured in NyxID."** with no
server verification.

The blast radius is correctly limited by the wakeup-only design (the opener just
calls `invalidateQueries`, and the key is still `pending_auth`, so nothing flips
to connected). But the page makes an unverified security assertion to the user.
The `status=error&code=provider_error` variant is worse: it renders a **"Try
again"** button whose click starts a fresh real attempt — which, per F-1, destroys
the credential again.

The same forgery works unauthenticated against the backend:
`GET /api/v1/providers/callback?state=1cc_<any-v4-uuid>` with no `code` hits the
prefix-only branch (`user_tokens.rs:549-563`) and redirects to `/oauth?...&nonce=…`
with zero DB access and zero auth.

### F-7 [P2] No popup-closed watchdog; `isClosed()` is dead code

`frontend/src/lib/oauth-popup.ts:105-111` defines `isClosed()`. Nothing in
`frontend/src` calls it — grep returns only the declaration and the interface
signature. There is no test for it.

The most common cancel gesture is closing the popup window. Nothing detects it:
`useKeyAuthorizationStatus` polls `/keys/:id` every 2 s and only stops on
`active`/`failed` (`frontend/src/hooks/use-keys.ts:70-76`), so the opener's OAuth
step shows "Waiting for GitHub…" indefinitely with the Connect button hidden. The
single-flow store lock is held for as long as `OAuthStep` stays mounted, so no
further chat OAuth can start in that tab. The user's only escape is closing the
dialog — at which point, per F-1, their credential is already gone.

### F-8 [P2] Retry silently becomes impossible when the provider origin is not in sessionStorage

`frontend/src/pages/oauth-complete.tsx:131-141` requires
`expectedProviderOrigin !== null` before it will act on an `oauth_retry`. That
value comes from `sessionStorage` written by the interstitial
(`frontend/src/pages/oauth-launching.tsx:47-50`).

Whenever the completion document is not in the same top-level browsing context
that ran the interstitial — popup-blocked fallback via the "Open GitHub" anchor
(`add-key-dialog.tsx:1732-1739`), a storage-partitioned or ITP-cleared context,
the user reopening the completion URL — the key is absent. The opener still runs
a full real re-initiate (destroying the credential per F-1) and posts a valid
retry, but the popup discards it and falls through to the 10 s
`RETRY_TIMEOUT_MS` "Return to your NyxID tab" state. The user sees a dead button
after a 10-second wait, and a wasted destructive attempt.

The origin pin itself is correct and does **not** misfire on real providers: the
retry authorize URL is regenerated by the backend for the same
`provider_config_id`, so it lands on the same origin. Provider-side redirects
between origins happen *after* the authorize URL is loaded and are irrelevant to
this check.

### F-9 [P2] Success auto-close is likely cancelled by the popup's own focus event

`frontend/src/pages/oauth-complete.tsx:96-112` registers
`window.addEventListener("focus", cancel, { once: true })` alongside
`pointerdown`/`keydown`. In several browsers the popup receives focus as the
completion navigation settles, which can fire `focus` after React mounts and set
`staying = true`, permanently suppressing the 3 s auto-close that this PR's
headline behavior depends on. `oauth-complete.test.tsx:88-95` exercises the
`keyDown` path only; nothing covers `focus`. Needs a manual check in Chrome,
Firefox, and Safari before this is called shipped.

### F-10 [P2] Branch is behind `origin/main` and one existing test is red at its base

`src/pages/assistant.test.tsx:558` fails on this branch. I verified it is **not**
caused by this PR: it also fails at the merge-base (`3a392e38`) in a clean
worktree, and passes when `origin/main`'s `frontend/src` is substituted.
`origin/main` is at `bf484f39`. Rebase before merge, or CI will read as red for a
reason unrelated to this work. Everything else is green: clippy clean across the
workspace, 2580 frontend tests pass, and the Mongo-gated backend tests will
actually execute in CI (`.github/workflows/ci.yml:190`, `:408` provide `mongo:8.0`).

### F-11 [P2] Destructive reset is scoped only by `connection_id`

`begin_chat_oauth_attempt` filters `{connection_id, credential_type != node_managed}`
with no `user_id`. The caller does verify ownership first —
`resolve_api_key_for_auth_flow` (`handlers/user_tokens.rs:343-363`) resolves
through `get_user_service`/`get_api_key` scoped to `effective_owner` — so this is
not exploitable today. But the legacy destructive equivalent
(`reset_provider_api_key_state`) *is* `{_id, user_id}`-scoped, so the new
destructive write is strictly weaker than the one it parallels. Add `user_id` for
defense in depth.

---

## Verified as claimed (no finding)

**P1-5, legacy BSON identity — genuinely fixed.** All three fields carry
`serde(default, skip_serializing_if = "Option::is_none")`
(`models/oauth_state.rs:44-49`, `models/user_api_key.rs:43-45`). The round-trip
tests assert absence explicitly on reserialize
(`oauth_state.rs:186-192`, `user_api_key.rs:199-202`) and pass locally. I grepped
every write site: the only `$set` that ever names `oauth_attempt_nonce` is
`begin_chat_oauth_attempt` (chat-only), and `write_chat_oauth_tokens_to_key`
`$unset`s it. `db.rs` migrations set the struct field to `None`, which is skipped.
No `replace_one` exists on either collection. Nothing writes null for a legacy flow.

**P1-2, opener severance — genuinely fixed.** `oauth-launching.tsx:26-63`
captures the opener in a `useState` initializer, installs the validated listener,
sets `window.opener = null`, and only then posts `oauth_launch_ready`. The parent
(`oauth-popup.ts:54-89`) refuses to send `oauth_launch_navigate` before that ack,
and the ack is validated on `event.origin === window.location.origin`,
`event.source === popup`, and an exact `launchId` match — `event.source` is
browser-supplied and unforgeable, so the ack cannot be spoofed by another window.
The parent never assigns `popup.location`; navigation happens inside the popup
after validation. There is no window in which the provider document holds a live
opener.

**P1-7, state prefix — the missing-code branch is genuinely DB-free.**
`chat_attempt_nonce_from_state` (`user_token_service.rs:73-83`) is pure. The
legacy missing-code path performs zero MongoDB access, exactly as before. No
collision is possible: index 3 of a bare UUID is a hex digit, never `_`. Length is
40 chars, charset alnum + `-` + `_`, URL-encoded at emit — acceptable everywhere.
Every `find_one`/`delete_one` uses the full prefixed `_id`
(`:1131`, `:1562`, `:1610`, `:1621`, `:1862`, `:1950`); nothing anywhere parses the
UUID out for a lookup. The canonicality check
(`get_version() == Random && to_string() == nonce`) correctly rejects uppercase,
braced, and non-v4 forms. Docked to *partial* only because the denial branch does
not use it (F-2).

**P1-1, public routes — genuinely fixed, no auth bypass widened.**
`isPublicPath` uses exact equality for both new paths
(`frontend/src/lib/public-paths.ts:24-25`), `/oauth/authorize` and `/oauth/token`
return false (asserted in `public-paths.test.ts`), and the pre-existing
`startsWith("/oauth-consent")` is untouched. The gate is a render-before-auth
concession only; neither page issues an authenticated request, so nothing is
exposed. `frontend/nginx.conf.template:35` uses `location /oauth/` with a trailing
slash, so bare `/oauth` correctly falls through to the SPA while the IdP subpaths
still proxy — the layers agree. The `vite.config.ts` narrowing from
`^/oauth(?:/.*)?$` to `^/oauth/` is the dev-server mirror of that and is correct.

**Incidental backend files are mechanical.** I checked each of
`handlers/proxy.rs`, `mw/auth.rs`, `services/gcp_sa_service.rs`,
`agent_binding_service.rs`, `handlers/agent_bindings.rs`,
`services/user_credentials_service.rs`, `handlers/keys.rs`,
`handlers/user_api_keys_external.rs`, `services/unified_key_service.rs`,
`services/proxy_service.rs`, `db.rs`: every hunk is a single
`oauth_attempt_nonce: None,` line in a struct literal, all but three inside
`mod tests`. The only non-literal change is
`handlers/admin_sa_providers.rs:278-307`, which unwraps the new
`OAuthInitiateResult` and passes `None` for `flow_kind` — behaviorally identical.
**No behavior change in any of them.**

**Wizard bundle rebuild is minimal and legitimate.** A byte-level diff of
`cli/src/wizard/assets/index.html` shows exactly one 30-byte insertion:
`.text-\[17px\]{font-size:17px}` — the Tailwind utility newly emitted because
`oauth-complete.tsx:250` uses `text-[17px]`. No unrelated churn; `index.hash`
matches.

**CLAUDE.md compliance is clean.** Layering (`handlers → services → models`) is
respected — the handler's new `fail_callback_placeholders` only dispatches between
two service calls. No `#[serde(skip_serializing)]` on any model field (the three
new ones use `skip_serializing_if`, which is the required pattern). No new
`DateTime` fields, so the chrono BSON helper rule does not apply. Responses use the
dedicated `OAuthInitiateResponse` struct, not a serialized model. No `console.log`.
No raw `useForm`. `models/oauth_flow_kind.rs` is a value type with no
`COLLECTION_NAME`, consistent with existing `nullable_field.rs` / `bson_bytes.rs`.

---

## Test quality

**Non-vacuous — these fail if the fix is reverted:**
- `user_api_key_service::chat_attempt_generation_rejects_stale_success_and_failure` — asserts `matched_count`/`modified_count` outcomes for attempt A after B stamps; drop the nonce from either filter and it fails. Mongo-gated, but CI supplies Mongo.
- `oauth_state.rs` / `user_api_key.rs` reserialize-absence assertions — fail if `skip_serializing_if` is removed.
- `chat_state_discriminator_requires_canonical_uuid_v4` — covers bare UUID, non-v4, uppercase prefix.
- `oauth-launching.test.tsx` "severs its opener before acknowledging readiness" — the assertion lives *inside* the `postMessage` spy, so it genuinely pins the ordering rather than the end state.
- `add-key-dialog.test.tsx` "keeps the legacy caller free of popup and flow parameters" — the load-bearing guard for the whole safety property.
- `generic_oauth_callback_denial_...` asserts equality against a pre-computed `expected_legacy_location`, which is the right shape for a byte-identity claim.

**Encodes the bug rather than catching it:**
- `oauth-complete.test.tsx:52-68`, titled "broadcasts a nonce-free wakeup", builds the URL as `/oauth?status=complete&flow=cc&nonce=${NONCE}` and asserts only that the *message* lacks a nonce. It ratifies F-5's URL exposure as the contract.

**Coverage gaps for real failure modes:**
1. Popup closed by the user (F-7) — no test, no code.
2. Denial with a reaped state (F-2) — the exact bug, no test.
3. Legacy and chat attempts interleaved on one `connection_id` (F-3) — no test.
4. Concurrent chat initiates on one connection (F-4) — no test.
5. `expectedProviderOrigin === null` retry (F-8) — no test.
6. `focus`-triggered auto-close cancellation (F-9) — only `keyDown` covered.
7. `add-key-dialog.test.tsx:97-99` mocks `useOAuthPopupReceiver` to a no-op, so the wiring between `OAuthStep`'s `retryPopup`/`handlePopupViewResult`/`handlePopupDismiss` and the channel has **zero integration coverage** — every handler is only unit-tested against a mock counterpart.

---

## Merge verdict

**Request changes — 3 P1 (F-1 credential destruction at initiate, F-2 denial escapes popup routing, F-3 one-directional generation guard) and 8 P2; the popup protocol itself is sound but the server-side attempt lifecycle is not safe to ship.**
