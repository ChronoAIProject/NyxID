# NyxID OAuth — Universal Popup Flow (Plan v3)

Status: draft, not implemented. Supersedes v1/v2. v2 was written for a
cross-origin external host; the host page is NyxID itself, so the cross-origin
machinery is gone. Codex's adversarial findings that still apply are folded in
(full text: `docs/OAUTH_POPUP_FLOW_CODEX_REVIEW.md`).

Chat-only pilot implementation note (2026-08): the implemented `cc` flow is
governed by `docs/OAUTH_CHAT_POPUP_IMPL_PLAN.md`, including accepted
corrections C1-C9 and the attempt-generation/state-prefix hardening added by
the SOL review. The remaining surfaces in this universal design are still
unimplemented.

## 1. What we're building

**Every OAuth flow in NyxID runs in a popup window and completes on one shared
route.** The page underneath never navigates away. The popup shows a
flow-appropriate message and closes itself.

```
NyxID app (any surface: /keys, chat connect card, admin grid, wizard, login)
   │  click
   ↓
popup window (top-level, small, ~760×820)  ← provider consent screen
   │  provider redirects → backend callback → tokens stored
   ↓
/oauth?status=complete&flow=kc   ← shared completion route, NyxID's own page
   │  broadcast → opener updates instantly
   ↓
auto-closes
```

The popup is a **separate small window, not a tab** — that requires the
`popup,width=,height=` feature string; `window.open` without it (and
`target="_blank"`, which is what ships today) yields a tab. Chrome/Firefox/Edge
desktop honor it. Android Chrome and iOS Safari have no windowing and always
produce a tab; the flow still works, it just isn't visually contained (§8).

**The provider's consent page can never be iframed** (`github.com` sends
`X-Frame-Options: DENY`, as does every serious IdP). A popup is a top-level
browsing context, so XFO does not apply to it — this is the standard pattern
(Auth0 `loginWithPopup`, Firebase `signInWithPopup`, Supabase).

## 2. Surfaces to migrate

| Surface | Today | File |
|---|---|---|
| AddKeyDialog (`/keys` + assistant chat connect card) | fetch URL → second click on `<a target="_blank">` → tab; poll every 2s | `components/dashboard/add-key-dialog.tsx:1565`, `hooks/use-keys.ts:54-91` |
| Admin providers grid | `hardRedirect` — destroys the SPA | `components/dashboard/provider-grid.tsx:81` |
| Service-account providers | `hardRedirect`; separate POST handler | `components/dashboard/sa-connected-providers.tsx:99`, `handlers/admin_sa_providers.rs:210` |
| CLI wizard auth-flows | `window.open(url,"_blank")` from an async effect | `components/cli-wizard/auth-flows.tsx:991` |
| Social login (Sign in with GitHub/Google) | full-page `openExternal` | `components/auth/auth-flow.tsx:296,344` |

## 3. The flow-kind enum

A NyxID-owned enum identifying *which of our flows* is running, so the
completion page renders the right copy and the backend can route without
guessing. Defined once in Rust, mirrored in TS, and asserted equal by a test.

| Variant | Wire token | Surface | Completion copy |
|---|---|---|---|
| `KeyConnect` | `kc` | AddKeyDialog / chat connect card | "Connected. Your credential is ready to use." |
| `ProviderConnect` | `pc` | admin providers grid | "Provider connected." |
| `ServiceAccountConnect` | `sa` | admin SA providers | "Connected for this service account." |
| `WizardConnect` | `wz` | CLI wizard | "Connected. Return to your terminal." |
| `SocialLogin` | `sl` | Sign in with GitHub/Google | "Signed in. Returning to NyxID." |

Unknown/absent token → generic copy. The token is **never rendered**; it only
selects from a fixed client-side table (Codex finding 20: no provider- or
attacker-controlled free text in the completion URL).

`provider` (catalog slug) may accompany it for a nicer label, validated against
the known catalog map client-side with a generic fallback.

### 3.1 Two different URLs — only one of them is constrained

This distinction drives the whole design, so state it plainly:

| | **Callback URL (`redirect_uri`)** | **Success URL** |
|---|---|---|
| Value | `{BASE_URL}/api/v1/providers/callback` | `{FRONTEND_URL}/oauth?status=…&flow=…` |
| Who sees it | registered at GitHub/Google; the provider redirects here | **nobody but us** — our backend 302s the browser here after the token exchange |
| Query params | constrained (below) | **completely unconstrained** |

**Query params on the success URL: yes, they work, no caveats.** No provider is
involved — it's a plain redirect from our own server to our own page. That is
already the design (`/oauth?status=complete&flow=kc&nonce=…`).

**Query params on the `redirect_uri`: don't rely on them.** RFC 6749 §3.1.2
permits a query component, but implementations diverge:

- **GitHub** ignores query params when matching the registered callback and
  its docs point you at the state parameter instead; path matching requires a
  *subdirectory* of the registered URL, host and port exact.
- **Google** requires an exact match and the redirect URL cannot be customized
  per request, so a per-flow query param is not expressible.
- **GitLab** has long-standing breakage authenticating when the redirect URL
  carries query parameters.

Since the catalog spans many providers and BYO users register their own OAuth
apps, per-flow callback URLs (or query params on them) are not viable. `state`
is the spec's round-trip channel and GitHub's own documented recommendation.

### 3.2 Where flow-kind lives: the OAuthState row

Because **every** OAuth flow becomes a popup flow (§1), there is no mode to
discriminate — `/oauth` is the unconditional completion route for all of them.
That removes the need for both a `display_mode` flag and the state-prefix
envelope earlier drafts proposed.

So: `flow_kind` and `attempt_nonce` are stored on `OAuthState` at initiate, and
the backend reads them when building the success URL. Nothing is smuggled into
the state string; state stays an opaque uuid.

**The one degraded case, and why it's acceptable.** If the state row is gone at
callback time (missing state, unknown state, or a TTL-reaped row after the
10-minute window), the backend cannot attribute the flow and redirects to
`/oauth?status=error&code=state_expired` with **generic copy and no nonce**.
The retry CTA still works, because the retry is executed by the *opener*, and
the single-flow lock (§5) means the opener knows exactly which attempt of its
own to restart. We lose flow-specific wording on that one error path; we do not
lose the remedy. (If that wording turns out to matter, a short prefix on the
state string recovers it later without changing anything else.)

### 3.3 Error-code enum (also NyxID-only, fixed set)

`access_denied`, `provider_error`, `state_invalid`, `state_expired`,
`state_replayed`, `session_required`, `session_mismatch`, `exchange_failed`,
`server_error`. Copy lives client-side; unknown code → generic. Replaces
today's free-text `message` query param, which can carry masked emails.

## 4. The completion route — `/oauth`

**Contract**

```
/oauth?status=complete&flow=kc&nonce=<attempt_nonce>[&provider=github]
/oauth?status=error&flow=kc&code=state_expired&nonce=<attempt_nonce>
```

- **Root-level route**, direct child of `rootRoute`, no auth guard, minimal
  boot. (Codex finding 24: `/providers/callback` is *not* a session-free
  precedent — it nests under `dashboardLayout`, whose `beforeLoad` redirects
  unauthenticated users to `/login`, `router.tsx:318`.) `/oauth` is free;
  `/oauth-consent` (`router.tsx:165`) is NyxID-as-IdP and unrelated.
- **`nonce`, not `key_id`** (Codex finding 4): the inbound `key_id` may be a
  `UserService` or `UserApiKey` id, is resolved to `connection_id` and
  discarded, and SA/legacy flows have no key id at all — it cannot be populated
  consistently. Initiate mints a high-entropy `attempt_nonce`, stores it on
  `OAuthState`, and only that appears in the URL. Receivers match it against
  the attempt they started; unknown nonce is ignored (this is also what stops
  any same-origin page from faking a completion broadcast, Codex finding 11).

**Behavior on mount**

1. Broadcast `{type:"oauth_result", flow, status, code, nonce}` on
   `BroadcastChannel("nyxid.oauth")` — *first*, before any delay, so the opener
   updates instantly.
2. `history.replaceState` to strip the query string (history/log/referrer
   hygiene, Codex finding 20). `Referrer-Policy: no-referrer`, telemetry
   suppressed on this route.
3. Render the flow-specific message + CTAs (§4.1) with
   `role="status" aria-live="polite"`; focus the primary CTA for keyboard users.
4. **Success only:** after ~3s (visible countdown), `window.close()`. Any
   pointer or keyboard interaction with the page cancels the countdown — the
   user reaching for a CTA must never have it yanked away. Honors
   `prefers-reduced-motion` (no animated counter, same timing).
   **Errors never auto-close** (§4.3).
5. If the document is still alive ~300ms after a close attempt — close refused,
   or we're a tab on mobile, or the page was opened directly — swap to "You can
   close this window" plus a manual close button and, when `window.opener` is
   null, a "Back to NyxID" link.

### 4.1 CTAs — driving the original tab from the popup

**Constraint:** by the time we reach `/oauth`, `window.opener` is null. GitHub
serves COOP, which switches the browsing-context group and severs the opener
permanently — including after we navigate back to our own origin. So the popup
cannot `opener.focus()` or `opener.location = …`.

**What works:** the popup asks the original tab to act, over the same-origin
broadcast channel, then closes itself. Closing a popup returns OS focus to the
window behind it, so the user lands back on a tab that has already navigated.

```
popup → opener   {type:"oauth_action", action, nonce}
opener → popup   {type:"oauth_ack", nonce}        ← proves a live opener handled it
popup            ack within 400ms → window.close()
                 no ack           → navigate THIS window to the destination
                                    (original tab is gone; don't strand the user)
```

**The opener resolves the destination, not the popup.** The completion URL
carries no key id by design (§4, Codex finding 4). It doesn't need one: the tab
that started the attempt already knows which key it minted, keyed by `nonce`.
So `action:"view_result"` + `nonce` is enough for the opener to route itself to
`/keys/{id}`. This keeps identifiers out of the URL and out of browser history.

**Two rules that decide every CTA**

1. **The popup never becomes a destination.** It is a transient surface: a
   chrome-less ~760×820 window with no tab strip and no back button. Navigating
   it to the dashboard would leave the user with two live NyxID instances
   (two query caches, and in chat's case potentially a second assistant stream),
   an ambiguous "which window is the app", and an orphaned original tab. The
   popup completes, confirms, and hands attention back. Always.
2. **The opener decides what "view result" means** — the popup only broadcasts
   the intent. That is what makes the CTA surface-appropriate rather than
   one-size-fits-all, and it needs no change to the protocol.

**Success CTAs**

| Flow | Primary CTA | What the opener does |
|---|---|---|
| `cc` ChatConnect | "View connection" | opens `ManageConnectionModal` **in place** over the transcript — no navigation |
| `kc` KeyConnect (`/keys`) | "View your AI services" | navigates → `/keys/{id}` (resolved from nonce) |
| `pc` ProviderConnect | "Back to providers" | navigates → `/providers` |
| `sa` ServiceAccountConnect | "Back to service account" | navigates → the SA detail route it came from |
| `wz` WizardConnect | "Continue setup" | advances the wizard step in place |
| `sl` SocialLogin | "Continue to NyxID" | `checkAuth()` → `return_to` or `/` |

**Chat is deliberately the exception.** Navigating the chat tab to `/keys`
would destroy the conversation the user was mid-way through — the exact failure
this whole design exists to prevent. Chat already has the right surface:
`ManageConnectionModal` (`components/assistant/manage-connection-modal.tsx`),
which the connect card can already open via its "Manage" affordance
(`blocks/connect-card.tsx:112`). So the chat CTA opens that modal over the
transcript and the conversation stays put.

Because the card in chat has already flipped to "Connected — send your request
again" by the time the user reads the popup, this CTA is a convenience, not the
only path. Users who ignore it lose nothing.

Secondary CTA on every success: "Stay here" (cancels the countdown), since the
window closing on its own is the default.

### 4.2 Error CTAs

Every error gets a meaningful next step, not just a message.

| Code | Copy | Primary CTA | Secondary |
|---|---|---|---|
| `access_denied` | "You declined access at {provider}." | "Try again" | "Cancel" |
| `state_expired` | "This took longer than 10 minutes and expired." | "Start over" | "Cancel" |
| `state_invalid` / `state_replayed` | "This authorization link is no longer valid." | "Start over" | — |
| `session_required` | "Sign in to NyxID first, then reconnect." | "Sign in" (opener → `/login`) | — |
| `session_mismatch` | "You're signed into NyxID as a different account in this browser." | "Switch account" (opener → `/login`) | "Cancel" |
| `provider_error` / `exchange_failed` | "{provider} couldn't complete the connection." | "Try again" | "Get help" (docs) |
| `server_error` | "Something went wrong on our side." | "Try again" | "Get help" |

"Try again" / "Start over" broadcast `action:"retry"`, which runs the §9 retry
transaction in the opener (fresh state, prior attempt invalidated, placeholder
reset) and relaunches — then the popup closes. "Cancel" broadcasts
`action:"cancel"` (consume state, reset placeholder) and closes.

### 4.3 Deviation from the original brief — errors do not auto-close

The brief said errors should also auto-close. Recommendation: they shouldn't.
An error window that closes itself takes its own remedy with it, and the user
is left on the original tab with a failed card and no explanation of what to do
next. Errors stay open until the user picks a CTA; that CTA is the exit. Easy
to reverse if you'd rather have the symmetry — it's one flag on the route.

Same-origin popup → same storage partition → the broadcast reaches the opener
directly. (The partitioning problem Codex flagged as fatal only existed for the
cross-origin-host design, which is now gone.)

**The broadcast is a wakeup, not a verdict.** Receivers invalidate/refetch
`["keys"]` (or call `checkAuth()` for `sl`); the DB row stays the source of
truth (FI-004), and the existing 2s poll remains the fallback for blocked
popups, lost messages, and mobile tabs.

## 5. Popup launch

`frontend/src/lib/oauth-popup.ts` (new).

**Open synchronously, then navigate** (Codex finding 5). A single-click
"Continue with GitHub" would call `window.open` after two awaits
(`ensureKey()` + initiate) and Safari blocks that as lacking user activation:

1. On click, synchronously `window.open("/oauth/launching", name, features)` —
   a same-origin interstitial, inside the gesture.
2. Await key creation + initiate in the parent.
3. `popup.location.href = authorizationUrl`. On failure, navigate the
   interstitial to an error state; on abort, close it.

The interstitial is also the **opener-severance point** (Codex finding 6):
`noopener` can't be passed at open time (we'd lose the handle and
blocked-detection), and not every catalog provider is GitHub or serves COOP, so
a malicious provider could otherwise `window.opener.location = <phishing>`.
The interstitial hops to the provider in a context whose opener is already
cleared.

**Per-attempt window names** — `nyxid_oauth_{nonce}`, never a fixed name
(Codex findings 9, 10): a shared name lets a second connect navigate an
in-flight popup away mid-consent, and lets any page that can resolve the name
retain a handle or navigate it somewhere hostile. Concurrency is instead
managed by an app-level single-flow lock (shared store, not component state)
offering "a connection is already in progress — focus / cancel".

**`win.closed` is a soft hint only** (Codex finding 7): after a COOP
browsing-context-group switch the retained handle can report closed while the
provider window is visibly open. It may offer a retry affordance; it must never
auto-fail a placeholder.

Blocked popup (`null` return) → fall back to today's `<a target="_blank">`
anchor; polling completes the flow either way.

## 6. Backend changes

- `models/oauth_state.rs` — add `flow_kind` and `attempt_nonce`
  (serde-defaulted). No `display_mode`: every flow is a popup flow, so there is
  nothing to switch on. Note: new fields require updating **every**
  `OAuthState` constructor including device-code paths and tests — serde
  defaults cover old BSON, not Rust compilation (Codex finding 19).
- New `models/oauth_flow_kind.rs` — the enum + wire-token parse/serialize, one
  place, unit-tested against the TS mirror.
- `handlers/user_tokens.rs` — accept `flow` at initiate, mint the nonce; route
  **every terminal branch** (success, denial, missing code, missing state,
  expired, mismatch, exchange failure) to `/oauth`. The success URL is
  server-built from the row; `redirect_path` is no longer used for browser
  flows, so there is no client-steered redirect and no open-redirect surface.
  Unattributable branches emit generic `status=error&code=…` with no flow.
- `handlers/admin_sa_providers.rs` — SA initiate is a separate POST handler
  calling the service directly with a fixed redirect path; it must be updated
  or SA flows silently stay legacy (Codex finding 19).
- `handlers/social_auth.rs` — popup variant of the success/error redirects
  (§7 sequencing note).
- New cancel endpoint (consume state, reset placeholder) for §9.

## 7. Pre-existing defects to fix first

Found by the review, present in `main` today, verified in the code. Each is its
own PR, landed before the popup work.

**D-1 — account-linking login-CSRF.** `ensure_callback_user_matches_state`
returns `Ok(())` when `auth_user` is `None`; it rejects only a *present but
different* session. A callback with a valid `state` and no session cookie
succeeds, so an attacker can initiate a connect in their account, send the
authorization URL to a victim, and capture the victim's GitHub token under the
attacker's account. Fix: browser flows (`mode=p`) require the initiating
session at callback → `session_required`; session-less completion stays
confined to device/CLI pairing paths that have their own confirmation.

**D-2 — `state` is not single-use on the denial path.** Only
`handle_oauth_callback` atomically claims the row
(`user_token_service.rs:1550-1557`). The `query.error` branch peeks and returns
without consuming or deleting it, so `?error=access_denied&state=…` can be
replayed to permanently fail a connection while leaving the state live. Fix:
atomic claim/delete on every terminal branch.

**D-3 — expiry marks a live flow terminally failed.** State TTL is 10 min from
initiation (`user_token_service.rs:709`) and lazy reconciliation treats expired
as absent (`user_api_key_service.rs:726`), flipping the placeholder to
`failed`. A user slowed by GitHub MFA or org approval crosses it and can't be
rescued by the later callback. Fixed together with §9.

## 8. Failure modes

| Failure | Behavior |
|---|---|
| Popup blocked | `null` return → `target="_blank"` anchor fallback; polling completes |
| Initiate fails after sync-open | interstitial shows error; popup closed |
| `closed` fires spuriously (COOP) | soft hint only — offer retry, never auto-fail |
| `window.close()` refused | "You can close this window" + manual button |
| Android Chrome / iOS Safari | popup becomes a tab; flow works, containment doesn't. Completion page gets an explicit "Return to NyxID" affordance instead of pretending to auto-close |
| Installed PWA (iOS) | Safari owns the provider flow, app may be suspended; polling resumes on return |
| `/oauth` opened directly by a user | no opener, no auto-close; message + "Back to NyxID" |
| CTA clicked but original tab was closed | no `oauth_ack` within 400ms → the popup navigates itself to the destination and becomes the app |
| CTA clicked while opener is mid-navigation | ack is nonce-scoped; a stale opener that doesn't recognize the nonce stays silent, so the popup takes the self-navigate path rather than driving the wrong tab |
| State expired mid-flow | retry transaction mints fresh state and resets the placeholder (§9) |
| Two flows started concurrently | single-flow lock offers focus-or-cancel; per-attempt names prevent clobbering |
| Broadcast lost | 2s poll flips the UI exactly as today |

## 9. Retry / cancel transaction

"Retry?" is not a UI affordance alone (Codex finding 18). A retry must mint a
**fresh** state, atomically invalidate the prior attempt, and reset-or-replace
the correct placeholder — a `failed` key must be recoverable, not terminal.
Cancel does the same minus the new attempt. Both go through one backend
transaction so the placeholder-before-state reconciliation race stays closed.

## 10. Security invariants

1. `state` single-use on every terminal branch (D-2).
2. Browser popup flows require the initiating session at callback (D-1).
3. No token material, no free text, no account identifiers in the completion
   URL — opaque nonce + fixed enums only; URL scrubbed on arrival.
4. Broadcast is a wakeup; the DB row is the source of truth.
5. Popup-mode redirect target is server-fixed; client input never steers it.
6. The provider window's opener is severed by a same-origin interstitial before
   provider navigation — never by trusting provider COOP.
7. Per-attempt window names; app-level single-flow lock.
8. Framing posture unchanged: `X-Frame-Options: DENY` and
   `frame-ancestors 'none'` stay as they are (`mw/security_headers.rs:34,38`).
   Nothing in this plan needs a framing carve-out.

## 11. Scope option: chat-only pilot (recommended first cut)

The popup flow can be confined to the assistant chat connect card, leaving
every other surface byte-identical. The flow-kind enum is the seam.

**Add one variant** — `ChatConnect` / `cc` — distinct from `kc`, because
`AddKeyDialog` is shared: it serves both `/keys` and the chat card
(`components/assistant/blocks/connect-card.tsx:246-255` mounts the same
dialog). The dialog takes a `launch?: "popup" | "tab"` prop; the chat card
passes `"popup"` and `flow: "cc"`, `/keys` passes nothing and keeps today's
behavior.

**Backend becomes strictly additive:** the callback routes `cc` to `/oauth`;
every other flow (including absent/unknown) falls through to the existing
`redirect_path` / `/providers/callback` logic untouched. All terminal branches
still need handling *for `cc`* — success, denial, missing code, missing state,
expired, mismatch, exchange failure — that part is not reducible.

**What drops out of scope**

- providers grid, SA providers (+ `admin_sa_providers.rs`), CLI wizard (and its
  bundle-rebuild CI dance), social login (PR-D — the highest-blast-radius
  surface in the app)
- enum ships with `cc` live and the rest reserved
- D-3 / the full retry transaction can be deferred: "Start over" mints a fresh
  key and the abandoned placeholder is cleaned up by the dialog's existing
  `cleanupPendingAuthKey`. Known rough edge, acceptable for a pilot.

**What does not drop out**

- D-1 and D-2 are pre-existing security bugs and stay independent of scope
- `oauth-popup.ts`, the `/oauth/launching` interstitial, the `/oauth` route with
  CTAs, the broadcast receiver, and the single-flow lock are all still needed —
  they're the mechanism, not the rollout

**Honest tradeoff.** Chat is the *lowest-risk* surface but also the
*least-broken* one: it already opens a tab and polls, so it never destroys the
page. The acute breakage — `hardRedirect` nuking the SPA — is on the providers
grid and SA pages, which this cut excludes. So chat-only is the right call if
the goal is proving the mechanism safely; it is the wrong call if the goal is
fixing the worst UX first. Either way the later surfaces are additive: flip a
prop and add an enum variant.

## 12. Sequencing

Chat-only pilot (§11), then widen:

1. **D-1, D-2** — independent security PRs on today's code, no dependency on
   any of the below.
2. **PR-A backend** — flow-kind enum + `attempt_nonce` on `OAuthState`;
   initiate accepts `flow`; callback routes `cc` to `/oauth` on every terminal
   branch; all other flows untouched; tests.
3. **PR-B frontend core** — `oauth-popup.ts`, `/oauth/launching` interstitial,
   `/oauth` completion route + CTAs, broadcast receiver hook, single-flow lock.
4. **PR-C chat card** — `launch` prop on `AddKeyDialog`; chat connect card opts
   in. Ship, dogfood, watch.

Then, once the mechanism is proven, each of these is additive — one enum
variant plus one prop:

5. **PR-D** providers grid + SA providers (`admin_sa_providers.rs`) — the
   surfaces with the worst current UX (`hardRedirect`).
6. **PR-E** CLI wizard (**CI:** rebuild the bundle with
   `npm --prefix frontend run build:wizard` and commit `cli/src/wizard/`).
7. **PR-F** D-3 + the full retry/cancel transaction.
8. **PR-G social login** — last, separately. Same mechanism, but its callback
   sets the session cookie and it's the highest-blast-radius surface in the
   app; it should never ride along with credential connects.

## 12. Test plan

- **Vitest** — popup manager (sync-open, blocked → fallback, per-attempt names,
  soft-closed semantics); `/oauth` route (each flow/error enum renders its copy
  and CTAs, unknown token → generic, URL scrubbed, broadcast fires before
  close, interaction cancels the countdown, errors never auto-close, close
  fallback, direct-open path); CTA protocol (ack → close, no-ack → self-navigate,
  stale nonce ignored); opener resolves `view_result` nonce → `/keys/{id}`;
  receiver ignores unknown nonce; dialog state machine.
- **Rust** — flow-kind wire-token round-trip; every terminal branch routes to
  `/oauth` with the right `status`/`code`, and unattributable branches emit
  generic errors with no flow and no nonce; D-1 and D-2 regression tests; SA
  initiate stores flow-kind; TS/Rust enum parity test.
- **Real browser (not mocks)** — COOP behavior and `window.closed` cannot be
  validated by mocking (Codex finding 15/8 methodology point).
- **Manual matrix** — Chrome/Safari/Firefox desktop × {popup allowed, blocked},
  Android Chrome, iOS Safari + installed PWA, GitHub + one PKCE provider
  (Google), chat connect card mid-transcript, admin grid, SA page, CLI wizard,
  two concurrent flows.
