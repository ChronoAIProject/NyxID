# OAuth Popup Completion Fix Plan — v2 (post-adversarial-review)

Planner output v2, 2026-08-06, against `fix/oauth-popup-completion-state` @
`21297220` (= `main` tip, contains PR #1349). Supersedes v1 in place after the
adversarial review (`docs/OAUTH_POPUP_COMPLETION_FIX_SOL_REVIEW.md`, verdict
REWORK). This document is self-contained: an implementer needs neither v1 nor
the review to execute it. Background: `docs/OAUTH_CHAT_POPUP_STATUS.md`.

**Review disposition summary** (details in §7 — every finding has an explicit
disposition there; nothing was silently absorbed):

| Finding | Severity | Disposition |
|---|---|---|
| 1. Nonce-gated context doesn't authenticate the outcome | P1 blocker | **ACCEPTED** — success copy stays outcome-neutral; signed receipt **DEFERRED** to the universal design |
| 2. Watcher falsely completes reconnects, then can't render them connected | P1 blocker | **ACCEPTED** — baseline-aware watch + attempt-scoped local verdict |
| 3. Key-id-only settlement lets a stale watch settle a fresh retry | P1 blocker | **ACCEPTED** — attempt-id in the query identity + retry fires the lifecycle callback |
| 4. Pending fires pre-handoff; dialog close has no outcome semantics | should-fix | **ACCEPTED** — pending/abort callback pair; dialog close = explicit cancel (ownership transfer rejected for the pilot) |
| 5. Deploy-skew analysis false; smoke test passes pre-fix | P1 blocker | **ACCEPTED** — v1 claim retracted; frontend-first rollout + exact `/oauth` alias + HTML no-cache + executable smoke that fails today |
| 6. Enriched behavior not gated to `cc` | should-fix | **ACCEPTED** — `flow === "cc"` gate everywhere + regression test |
| 7. Test plan misses the real boundaries | should-fix | **ACCEPTED** — real-QueryClient generation tests, contract-test updates, lifecycle cases added |
| 8. Wizard freshness deterministically fails | nit | **ACCEPTED** — rebuild is unconditional (manifest verified) |

No finding was rejected outright: every piece of the review's evidence I
re-verified independently checked out (§1b).

---

## 0. Implementer prerequisites

1. **Restore the frontend build gate before starting.** The reviewer could not
   complete `npm --prefix frontend run build`: `tsc -b` failed with repeated
   `TS2307: Cannot find module 'lucide-react'` because the installed
   `lucide-react@0.563.0` was missing its declared `dist/esm/lucide-react.js`
   / `dist/cjs/lucide-react.js` entry files — a broken local install
   artifact, not a source failure. Fix by clean reinstall:
   `rm -rf frontend/node_modules && npm --prefix frontend ci`, then confirm
   `npm --prefix frontend run build` passes on the **unmodified** branch
   before any edit. Editing `frontend/package.json` or the lockfile to work
   around it is **not acceptable** (and would drag in the Wizard Bundle
   Freshness closure for no reason). Implementation is not done until this
   gate genuinely passes.
2. MongoDB up on 27018 (`docker compose up -d`) so the Mongo-gated backend
   callback tests actually run rather than skip.
3. Shared worktree discipline: no `git add -A`; stage files explicitly.

---

## 1. Verification log

### 1a. Original findings (v1 pass, re-confirmed by the reviewer's independent probes)

**P1 — completion page unreachable in prod: CONFIRMED.**

```
$ curl -sS -o /dev/null -D - "https://nyx.chrono-ai.fun/oauth?status=complete&flow=cc&nonce=test123"
HTTP/2 301
location: /oauth/?status=complete&flow=cc&nonce=test123

$ curl -sS -o /dev/null -D - "https://nyx.chrono-ai.fun/oauth/?status=complete&flow=cc&nonce=test123"
HTTP/2 404
content-length: 0
vary: origin, access-control-request-method, access-control-request-headers
x-content-type-options: nosniff          ← doubled
x-frame-options: DENY                    ← doubled
content-security-policy: default-src 'none'; frame-ancestors 'none'
```

The 404's headers are byte-identical to `GET /api/v1/<nonexistent>` (tower-http
`vary` triple, doubled security headers, `default-src 'none'` CSP from
`backend/src/mw/security_headers.rs:41`): the 404 is the **Rust backend**, not
the SPA. Control group in the same session: `/oauth-launching`,
`/oauth-consent`, `/login` all 200 with the 8.1 KB SPA shell.

**Correction to the original report:** the edge config **is** in this repo.
`frontend/nginx.conf.template:35` has `location /oauth/ { proxy_pass ... }`,
and nginx's documented special case (slashless request to a proxied
slash-terminated prefix location → automatic 301 to the slashed form) produces
exactly the observed `/oauth → /oauth/` redirect (relative `Location` because
of `absolute_redirect off`, line 6; Cloudflare merely fronts it). The
implementation plan (`docs/OAUTH_CHAT_POPUP_IMPL_PLAN.md`, Correction C2)
predicted the opposite — "bare `/oauth` … falls through to the SPA
`try_files`" — and that wrong prediction is why the completion page was left
at `/oauth` while the interstitial was moved to `/oauth-launching`. Dev never
caught it because vite's `^/oauth/` proxy regex (`frontend/vite.config.ts:51`)
has no auto-redirect, so bare `/oauth` works in dev. Dev/prod parity gap.

**P2 — connect card doesn't learn the outcome: CONFIRMED.**
`useOAuthPopupReceiver` handles `oauth_result` by invalidating `["keys"]` /
`["keys", keyId]` only (`frontend/src/hooks/use-oauth-popup.ts:44-51`) — this
is *deliberate* (wakeup-only invariant, status doc §4.4) and stays. The real
gap: `ConnectCard` mounts `AddKeyDialog` with `launch="popup" flow="cc"`
(`connect-card.tsx:248-264`) but never passes `onAuthorizationPending`, so no
card-scoped server-truth watch ever starts. The machinery exists and is wired
for action cards (`action-card.tsx:203-228, 513-516`;
`useKeyAuthorizationWatch` in `hooks/use-keys.ts:133-204`) — but see §1b: the
review proved naive adoption is insufficient (reconnects, retries, aborts).

**P3 — completion copy: CONFIRMED**, error-code audit (emitted by the `cc`
branch of `backend/src/handlers/user_tokens.rs`):

| Code | Emitted? | Where |
|---|---|---|
| `access_denied` | yes | `user_tokens.rs:577-588` |
| `provider_error` | yes | `:577-588`, `:601-607` (missing code) |
| `state_invalid` | yes | `:633-640` |
| `session_mismatch` | yes | `:710-718` |
| `exchange_failed` | yes | `:811-818`, `:881-895` |
| `state_expired` | yes | `:881-895` |
| `state_replayed` | yes | `:881-895` |
| `session_required` | **never** | — |
| `server_error` | **never** | (`:1376` is the audit-log normalizer, not the redirect) |

Nothing emitted is unmapped; two mapped codes are dead on the wire
(disposition §3.3d).

### 1b. v2 re-verification of the review's evidence

Each review claim I depended on was independently re-checked before accepting:

- **`/oauth-complete` already returns 200 today** — confirmed by my own probe
  and by inspection: nginx `location / { try_files $uri /index.html; }` serves
  the shell for any unknown path, and the SPA renders
  `defaultNotFoundComponent: AppNotFound` (`frontend/src/router.tsx:935`).
  A header-only smoke is therefore a false positive **before** the fix. My v1
  deploy-skew claim ("stale FE bundle works via SPA fallback") was **wrong**
  at the app-router level: the old bundle has no `/oauth-complete` route and
  renders a dead not-found page — no channel, no broadcast. Retracted.
- **Frontend and backend are separately deployed images** —
  `docker-compose.prod.yml` builds them as separate services; source-atomic
  merge ≠ runtime-atomic rollout. Confirmed.
- **`frontend/src/lib/telemetry.ts` and `frontend/src/hooks/use-keys.ts` are
  both in `cli/src/wizard/bundle-meta/index.manifest`** (lines 76 and 67) —
  confirmed by grep. The wizard rebuild is deterministic, not conditional.
- **Device-code fires `onAuthorizationPending` before initiate** — confirmed:
  `add-key-dialog.tsx` DeviceCodeStep calls `onAuthorizationPending?.(key.id)`
  before `initiateMutation.mutateAsync(...)`, and its catch deletes the fresh
  placeholder without notifying the card. The OAuth step fires it after
  initiate but before nonce validation / `popup.navigate`, and its catch
  (`:1756-1768`) likewise cleans up silently.
- **Reconnect preserves an active row and the card cannot render reconnect
  success** — confirmed: `useKeyAuthorizationWatch` treats bare `active` as
  terminal with no `last_authorized_at` baseline (`use-keys.ts:178-203`), and
  `connectedNow` requires `!needsReauthorization` (`connect-card.tsx:86-92`),
  so a `NYXID_UNAUTHORIZED` card can never flip from the list alone.
- **Stale terminal cache under `["keys", keyId]`** — confirmed: the watch
  query is keyed by keyId only; OAuth retry reuses `pendingKeyId`
  (`add-key-dialog.tsx:1555-1577`) and `retryPopup` never notifies the card;
  cached `failed` from attempt A is synchronously visible to attempt B.
- **`frontend/src/lib/oauth-popup.test.ts:39-80`** asserts the exact
  `oauth_launch_navigate` payload object — confirmed; it must change with the
  message contract.
- **`docs/DEPLOYMENT.md` cache strategy** ("HTML files: no-cache … JS/CSS with
  content hashes: cache forever") — confirmed at the "Cache Strategy" section;
  `frontend/nginx.conf.template` currently implements the immutable-assets
  half (`:70-77`) but **not** the HTML no-cache half.
- **Finding 1's attack** — confirmed by construction, no probe needed: the
  provider receives `state=1cc_<nonce>` in its own authorization URL
  (`schemas/oauth-popup.ts:76-98` requires it; `add-key-dialog.tsx:1723-1745`
  navigates the popup there), and the provider controls the popup's next
  navigation. It can navigate to
  `/oauth-complete?status=complete&flow=cc&nonce=<N>` without any NyxID
  callback having run. v1's nonce-match gate would have rendered
  "{Service} authorized" for an exchange that never happened — worse, the
  page's retry handler updates the stored nonce, keeping the gate satisfied
  across retries. The v1 design is withdrawn (§3.3).

---

## 2. Root causes

| # | Root cause | Anchors |
|---|---|---|
| P1 | SPA completion route placed at bare `/oauth`, inside a namespace whose slashed prefix is proxied to the backend IdP; nginx auto-301s the slashless form into the proxy → backend 404. Impl-plan C2 mispredicted the nginx behavior. | `frontend/src/router.tsx:190-194`, `frontend/nginx.conf.template:35`, `backend/src/routes.rs` (`.nest("/oauth", …)`), `vite.config.ts:51` |
| P2 | `ConnectCard` never receives the placeholder key id (`onAuthorizationPending` not passed), so no card-scoped authoritative watch exists; and the existing watch hook is not reconnect- or attempt-aware, so naive adoption would mis-settle. | `connect-card.tsx:248-264`, `add-key-dialog.tsx:1721`, `use-keys.ts:133-204` |
| P3 | Success copy was neutralized (F-6) because `status=complete` is query-asserted and no trusted display context existed; the page names no service and gives no next step. Any "connected" claim on this page is unprovable client-side for the pilot (Finding 1), so indicativeness must come from honest content, not an outcome assertion. | `oauth-complete.tsx:177-190`, `oauth-launching.tsx:51-55`, status doc §4.2/§5 F-6 |

---

## 3. Change plan

One PR on this branch. Internally ordered so each commit leaves the tree
green; the **deploy** order (frontend image first, backend second) is a
release-process requirement recorded in §3.1d and the PR body.

### 3.1 P1 — move the completion route to `/oauth-complete`, with a compatibility bridge

**Steady state:** the completion page lives at `/oauth-complete`, a root-level
hyphenated sibling like `/oauth-launching` (proven reachable in prod), outside
nginx `location /oauth/`, vite `^/oauth/`, and every backend route.

**Rejected alternatives** (unchanged from v1): fixing nginx *instead* of
moving (leaves the SPA squatting on the backend IdP namespace — the footgun
that already fired; and deployment provenance of the template is unproven);
vite-only fix (dev-only); hash-based route (breaks the server redirect model).

#### 3.1a Frontend route move

1. `frontend/src/router.tsx:190-194` — `oauthCompleteRoute.path` `"/oauth"` →
   `"/oauth-complete"`.
2. `frontend/src/pages/oauth-complete.tsx:90` — query scrub becomes
   `window.history.replaceState(null, "", "/oauth-complete")`.
3. `frontend/src/lib/public-paths.ts:24` — `path === "/oauth"` →
   `path === "/oauth-complete"`; keep the "never broaden to `/oauth/*`"
   comment.
4. `frontend/src/lib/telemetry.ts:125` — sensitive-pageview drop
   `/^\/oauth$/` → `/^\/oauth-complete$/` (keep `/\/oauth\/callback/`).
5. Mechanical test updates: `public-paths.test.ts:6`,
   `telemetry.test.ts:289`, every `/oauth` URL in `oauth-complete.test.tsx`.

#### 3.1b nginx template: compat alias + HTML cache policy

`frontend/nginx.conf.template` — two edits:

1. **Exact-match compat alias**, placed above `location /oauth/`:

   ```nginx
   # TEMPORARY (remove after one release cycle — see PR #<this>): pre-rename
   # backends redirect OAuth completions to /oauth; forward them, query intact.
   location = /oauth {
       return 302 /oauth-complete$is_args$args;
   }
   ```

   `302`, not `301`: this alias is temporary and must not be durably cached.
   The exact-match location wins over the `/oauth/` prefix, so the backend
   IdP surface under `/oauth/*` is untouched. With the alias, an
   **old backend** redirecting to `/oauth?...` lands on the new page the
   moment the frontend image deploys — the user-visible fix ships with the
   frontend; the backend change in §3.1c is then namespace cleanup, not a
   prerequisite.

2. **SPA HTML no-cache** (required by `docs/DEPLOYMENT.md` "Cache Strategy",
   currently unimplemented — this is what makes "old bundle after deploy"
   windows short instead of indefinite):

   ```nginx
   location / {
       add_header Cache-Control "no-cache" always;
       # add_header at location level DISCARDS server-level headers —
       # re-declare the security set, same as the static-assets location does.
       add_header X-Frame-Options "DENY" always;
       add_header X-Content-Type-Options "nosniff" always;
       add_header Referrer-Policy "strict-origin-when-cross-origin" always;
       try_files $uri /index.html;
   }
   ```

   The nginx `add_header` inheritance-clearing gotcha is why the security
   headers are repeated; the existing assets location (`:70-77`) already
   models this. Hashed assets keep their immutable-year policy unchanged.

**Alias removal:** tracked as a follow-up line in
`docs/OAUTH_CHAT_POPUP_STATUS.md` §6 — remove `location = /oauth` after one
full release cycle in which no pre-rename backend can still be issuing
callbacks (OAuth state TTL is minutes; the real bound is deploy overlap).
Removal is deliberate, not drive-by.

#### 3.1c Backend redirect target

6. `backend/src/handlers/user_tokens.rs:1000` —
   `format!("{frontend_url}/oauth")` →
   `format!("{frontend_url}/oauth-complete")` in
   `redirect_to_oauth_completion`. Only backend production change;
   `redirect_callback` / `redirect_to_path` (legacy flows) untouched — all
   `redirect_to_oauth_completion` call sites are inside chat-classified
   branches (verified `user_tokens.rs:576-895`), preserving invariant §4.6.
7. `user_tokens.rs:2242` and `:2331` — prefix asserts → `/oauth-complete?`
   (grep confirms these are the only two).

#### 3.1d Rollout order and the executable smoke test

**Deploy frontend image first, backend second.** Rationale (v1's claim
retracted): the old JS bundle renders `AppNotFound` for `/oauth-complete`, so
a new backend against an old bundle is a broken window; the reverse order has
no broken window because of the alias.

**Smoke test — must fail on today's code.** A header-only `curl` is a false
positive: `/oauth-complete` already returns a 200 SPA shell today (§1b). The
smoke must execute the page. Using the repo's headless-browser tooling (or
Playwright), against the target origin, with a fresh UUID nonce each run:

1. Navigate to
   `/oauth-complete?status=error&flow=cc&code=access_denied&nonce=<uuid>`;
   assert the rendered text **"Authorization declined"** appears and
   `window.location.search === ""` (the scrub executed — proves the real
   completion component ran, not the shell). *Today: fails (AppNotFound).*
2. Navigate to `/oauth?status=error&flow=cc&code=access_denied&nonce=<uuid>`;
   assert the same two facts (exercises alias → page end-to-end).
   *Today: fails (blank backend 404).*

Local pre-merge equivalent: `npm --prefix frontend run build && npm --prefix
frontend run preview`, then check 1 against `localhost:3000` (vite preview has
no nginx alias, so check 2 is deploy-time / frontend-image-container only —
runnable locally via `docker compose -f docker-compose.prod.yml up frontend`
if desired). Record both checks in the PR body as the deploy gate.

### 3.2 P2 — attempt-aware, reconnect-aware, retry-aware card settlement

**Decision:** the card still adopts the ActionCard shape (server-truth watch,
presence gate, deadline), but the review proved three concrete correctness
holes in naive adoption. The watch and the dialog↔card lifecycle contract are
upgraded first; the connect card consumes the upgraded contract.

#### 3.2a Lifecycle contract: `onAuthorizationPending` / `onAuthorizationAborted`

`frontend/src/components/dashboard/add-key-dialog.tsx`:

- **New attempt record.** `onAuthorizationPending` changes signature to
  `(attempt: { keyId: string; attemptId: string; previousAuthorizationAt: string | null | undefined }) => void`.
  - `attemptId`: dialog-minted `crypto.randomUUID()`, one per initiation —
    including popup retries.
  - `previousAuthorizationAt`: `reconnectMode ? (key.last_authorized_at ?? null) : undefined`
    — the exact baseline the dialog already computes for its own poll
    (`add-key-dialog.tsx:1715-1717`); `undefined` remains the fresh-add case.
- **Fire only after a handoff exists.**
  - OAuth step: keep the call after `initiateOAuthAsync` resolves, but move it
    **after** the popup-path nonce/URL validation succeeds (or immediately for
    the non-popup link path) so a validation throw never leaves the card
    watching a dead attempt.
  - Device-code step: move the call from before `initiateMutation.mutateAsync`
    to after it resolves (the placeholder key exists either way; the *journey*
    only exists after initiate succeeds).
- **New optional callback** `onAuthorizationAborted?: (attemptId: string) => void`,
  fired whenever an announced attempt dies without a server verdict:
  - OAuth step catch path (placeholder deleted / reconnect row preserved,
    `:1756-1768`);
  - device-code step catch path;
  - **`OAuthStep` unmount while the attempt is non-terminal** — this defines
    dialog close as **cancellation** (Open Question 2 of v1, now resolved).
    Guard: do *not* fire when the step's own dialog-scoped status is already
    terminal (`authorized || authorizationFailed`), so closing after an
    in-dialog "Connected" never retro-cancels.
  - `retryPopup` failure after a new attempt was announced.
- **Popup retry announces the new generation**: `retryPopup` success path
  calls `onAuthorizationPending` with the same `keyId`, a fresh `attemptId`,
  and the unchanged baseline — closing the "card never learns a retry
  started" hole.
- Popup-ownership transfer out of the dialog (the review's alternative arm)
  is **rejected for the pilot**: it is a structural refactor of popup/store
  lifetime for marginal UX, and "dialog close cancels the attempt" is
  coherent with the popup's own dismiss button (which already closes via
  `handleOpenChange(false)`). Recorded as a candidate for the universal
  design.

#### 3.2b Watch hook: generation- and baseline-aware

`frontend/src/hooks/use-keys.ts` — `useKeyAuthorizationWatch` options gain
`attemptId: string` and `previousAuthorizationAt?: string | null`:

- **Query identity includes the generation**:
  `["keys", keyId, "authorization", attemptId]`. Attempt B can never
  synchronously observe attempt A's cached terminal row — the review's race
  (A caches `failed`; B settles from it before its first fresh GET) is
  structurally impossible, not merely guarded. The internal `expiredFor`
  deadline record is keyed by `attemptId` as well.
- **Reconnect-correct terminal predicate**: `active` is terminal only when
  `previousAuthorizationAt === undefined` (fresh add) **or**
  `last_authorized_at` is non-null and differs from the baseline — the same
  predicate `useKeyAuthorizationStatus` already implements (`use-keys.ts:72-85`).
  A reconnect against a preserved active row stays "authorizing" until the
  server stamps a new authorization.
- On terminal, invalidate the exact `["keys"]` list (existing behavior) **and**
  `["keys", keyId]`, so the shared detail entry other surfaces read is warmed
  despite the watch's private query key.
- **Update the existing call site**: `action-card.tsx:222-228` passes the new
  fields from its (extended) `pendingAuth` record — it has the same latent
  retry/reconnect bug class and gets the fix mechanically.

#### 3.2c Connect card

`frontend/src/components/assistant/blocks/connect-card.tsx`:

- State: `pendingAuth: { keyId, attemptId, previousAuthorizationAt, startedAt } | null`,
  a `settledAttemptRef`, and `localVerdict: { attemptId: string; kind: "authorized" | "failed" | "timed_out" | "cancelled"; message?: string } | null`.
- Wire `AddKeyDialog` with both callbacks:
  - `onAuthorizationPending(attempt)` → reset settled ref, set `pendingAuth`
    (with `startedAt: Date.now()`), clear `localVerdict`.
  - `onAuthorizationAborted(attemptId)` → if it matches the live
    `pendingAuth` and no verdict is recorded: clear `pendingAuth`, set
    `localVerdict = { attemptId, kind: "cancelled" }`, and fire one
    invalidation of `["keys"]` + `["keys", keyId]` (catches an
    authorization that landed in the same instant the dialog closed).
- Watch: `useKeyAuthorizationWatch(pendingAuth?.keyId ?? null, { attemptId,
  previousAuthorizationAt, enabled: pendingAuth !== null && visible,
  deadlineAt: connectWatchDeadline(pendingAuth.startedAt, lastActivityAt) })`
  with `useChatPresence()` supplying `visible` / `lastActivityAt`.
- Settlement effect (attempt-scoped, one-shot via `settledAttemptRef`):
  - terminal-active (baseline-aware) → `localVerdict.kind = "authorized"`,
    clear `pendingAuth`;
  - `KEY_AUTH_FAILED` → `kind = "failed"` with `watch.errorMessage`;
  - `watch.timedOut` → `kind = "timed_out"`.
- Display precedence (first match wins):
  1. `connected` — now
     `connectedNow || block.state === "connected" || localVerdict?.kind === "authorized"`.
     The local verdict is what lets a **reconnect** card render Connected:
     `connectedNow` requires `!needsReauthorization` and the transcript
     block's `reason_code` is static, so without it a completed
     reauthorization could never display (review finding 2, second half).
     While `localVerdict.kind === "authorized"`, the reauthorization badge
     and Reconnect affordance are suppressed.
  2. `localVerdict.kind === "failed"` → destructive "Failed" badge, guidance
     from the server-sanitized message (generic fallback), Connect
     re-enabled ("Try again" path — safe: `begin_chat_oauth_attempt`'s
     status gate makes re-initiating a failed row non-destructive, status
     doc §3).
  3. `localVerdict.kind === "timed_out"` → existing `STATE_LABEL.timed_out`
     copy, Connect re-enabled.
  4. `localVerdict.kind === "cancelled"` → **neutral**, not destructive:
     baseline card state with guidance "Connection cancelled — you can start
     again." Connect enabled. (A destructive Failed badge for an attempt the
     user's own dialog-close aborted was the review's finding-4 UX defect;
     note the server's later lazy reconciliation of the abandoned
     placeholder to `failed` does not repaint the card — the cancelled
     verdict is attempt-scoped and the card is no longer watching.)
  5. `pendingAuth !== null` → the existing "Authorizing" spinner row;
     `authorizing` becomes `pendingAuth !== null || matchingKey?.status === "pending_auth"`
     so the card flips the instant the handoff happens instead of after the
     next list refetch.
- Untouched: `useOAuthPopupReceiver` (wakeup-only, invariant §4.4), the popup
  store's `begin()` single-attempt guard, and the nonce/launchId generation
  guards in `use-oauth-popup.ts:67-83` / `oauth-popup-store.ts` — they guard
  the broadcast/retry channel; the watch's generation safety now lives in its
  own query identity and does not lean on them.

The wakeup path composes for free: `oauth_result` → receiver invalidates
`["keys", keyId]` → list/detail refresh; the watch's private query refetches
on its cadence and on window focus, so the card settles within seconds of the
callback landing even with the popup closed, blocked, or `BroadcastChannel`
absent — server truth is the only verdict path.

### 3.3 P3 — indicative completion page without an unprovable outcome claim

**Decision on finding 1 (the arm chosen and why).** The review proved the
nonce cannot gate an outcome claim: the provider itself holds
`state=1cc_<nonce>` and controls the popup's next navigation, so it can drive
the popup to a nonce-matching `status=complete` URL with no NyxID callback
ever occurring. The alternatives were (a) a backend-authenticated completion
receipt (short-lived signed token covering flow/nonce/status/code/expiry,
verified by the public page via JWKS or a metadata-only resolve endpoint), or
(b) keeping success copy **outcome-neutral** and making the page indicative
through honest content. For the `cc` pilot, (b) is chosen:

- The popup is a ~3-second surface whose authoritative sibling — the chat
  card — now renders the real verdict from authenticated reads (§3.2). A
  signed receipt would add a new public token type, client-side signature
  verification, expiry/replay handling, and backend changes on the
  unauthenticated callback redirect, to upgrade three seconds of display in a
  pilot. Cost/blast-radius is out of proportion; the receipt is **deferred**
  to the universal popup design where all six flows would share it (tracked:
  status doc §6 gains a line; sketch preserved in
  `OAUTH_POPUP_FLOW_PLAN.md`'s follow-up list).
- **Residual, stated plainly:** a malicious or compromised provider can
  navigate the popup to a benign-looking neutral completion page (or a forged
  error page) without any NyxID callback. It can never elicit a
  "connected/authorized" claim from NyxID UI — no such claim exists on this
  page — and it gains nothing it could not already do by simply denying or
  succeeding the real flow. The only NyxID surface asserting connection
  outcome is the chat card, settled from authenticated server reads.

#### 3.3a Trusted launch context (display-only, `cc`-gated)

Purpose narrowed from v1: the context **selects display strings**; it proves
nothing about outcome and is never treated as proof.

- `frontend/src/types/oauth-popup.ts` — `OAuthLaunchNavigateMessage` gains
  `readonly serviceName?: string` (display label, ≤64 chars).
- `frontend/src/lib/oauth-popup.ts` — `navigate(url, nonce)` gains
  `serviceName`; `AddKeyDialog` passes `catalogEntry.name`. Replace
  `OAUTH_PROVIDER_ORIGIN_KEY` with one JSON record under
  `nyxid.oauth.launch-context`: `{ providerOrigin, nonce, serviceName }`, plus
  a strict `readLaunchContext()` (origin re-parsed via `new URL`, nonce via
  `oauthAttemptNonceSchema`, name type/length-checked). One key, one writer,
  one schema (FI-007 — the old key is removed, not kept alongside).
- `frontend/src/pages/oauth-launching.tsx:45-55` — after
  `validateAuthorizationUrl` passes, write the record (origin from the parsed
  URL object per the CQ-1 taint rule; invalid `serviceName` dropped without
  blocking navigation).
- `frontend/src/pages/oauth-complete.tsx` — `expectedProviderOrigin` for the
  retry binding now reads `context.providerOrigin`; the validated-retry path
  updates the stored `nonce` to `nextNonce` before `assign` (keeps
  *correlation* across retries — explicitly not an outcome gate).
- **`flow === "cc"` gate (finding 6):**
  `displayContext = search.flow === "cc" && context !== null && context.nonce === search.nonce ? context : null`.
  Context reads, enriched copy, and retry-context mutation all require it;
  the five reserved flow tokens keep today's neutral/degraded behavior
  byte-for-byte.

Why sessionStorage (unchanged from v1, survived review): query params are
attacker-reflected content — rejected; an authenticated fetch breaks
invariant §4.7 (public page, no authenticated requests) — rejected.
sessionStorage is tab-scoped, survives the interstitial → provider →
completion chain in the same tab (the mechanism the shipped retry-origin
binding already relies on; the review's independent MDN check confirmed the
partitioning model), and its writer is the validated interstitial handshake.

#### 3.3b Success copy — outcome-neutral, now indicative

`status=complete`:

- With `displayContext`: title stays **"Authorization response received"**
  (asserts only what the page can know: a completion navigation arrived);
  body becomes concrete and truthful in every case including the forged one:
  *"Return to your NyxID chat — the {serviceName} connection's status appears
  there and updates automatically."* Icon stays **neutral** (no green check:
  green would assert an outcome; the authored-but-unused `icon="success"`
  variant in `OAuthResultShell` stays unused).
- Without `displayContext` (forged link, contextless tab, non-`cc` flow):
  today's copy verbatim.
- Auto-close (3s), interaction-cancel, ack machinery, `replaceState` scrub:
  untouched. No token, code, or credential material is rendered; the only new
  string is the length-capped catalog display name from the trusted channel,
  rendered as React-escaped text and never used as an action input.

#### 3.3c Error copy

Per-code `ERROR_COPY` bodies are already specific and action-oriented — kept.
With `displayContext`, the service name is prefixed into the title
(`"GitHub: Authorization declined"`). This adds no capability to a malicious
provider: forging an error is behaviorally equivalent to denying the real
flow, which it can always do.

#### 3.3d Dead-end and reserved codes

- No-peer case (`validCompletion === false`): title "Nothing to complete",
  body *"This window isn't part of an active connection attempt. If you were
  connecting a service, its status appears in your NyxID chat — this window
  can be closed."* Keep "Back to NyxID". (Truthful because of §3.2.)
- `session_required` / `server_error`: **keep, annotated as reserved** for the
  unimplemented flows (comment in `types/oauth-popup.ts` + status-doc §2 table
  note), mirroring the flow-token table's "reserved, unimplemented" pattern.
  zod's `.catch(undefined)` already degrades unknown codes to generic copy, so
  keeping them is free; deletion is a 10-line amendment if the owner prefers
  strict FI-007.

Design-system note: everything renders in the existing `OAuthResultShell`
with existing tokens; no new primitives. Where `DESIGN.md` disagrees with the
live app, `frontend/src/app.css` + Mona Sans win; nothing here is novel enough
to engage that conflict.

### 3.4 Docs

`docs/OAUTH_CHAT_POPUP_STATUS.md`:

- Route rename throughout (§1 diagram, §2 contract, §4.2/4.7, F-5/F-6).
- §2: `oauth_launch_navigate` optional `serviceName` (display-only, validated,
  never an action input); launch-context record documented.
- §4: add invariant — *the completion page never renders an outcome claim;
  query `status` + launch context select copy only; outcome claims live
  exclusively on authenticated surfaces.* F-6 disposition updated to match.
- §6: two new tracked follow-ups — signed completion receipt (universal
  design) and removal of the temporary `location = /oauth` alias.
- New ledger rows for this fix round citing this plan and the review doc.

### 3.5 CI / packaging — unconditional obligations

- **No `frontend/package.json` / lockfile change is planned or permitted**
  (including as a lucide-react workaround, §0).
- **Wizard bundle rebuild is unconditional**, not contingent:
  `frontend/src/lib/telemetry.ts` (edited for the route rename) and
  `frontend/src/hooks/use-keys.ts` (edited for the watch upgrade) are both in
  `cli/src/wizard/bundle-meta/index.manifest`, and the freshness test hashes
  every listed source. Sequence: rebase (if any) → all source edits →
  `npm --prefix frontend run build:wizard` → commit `cli/src/wizard/` as the
  **last** commit → `cargo test -p nyxid-cli --test wizard_bundle_freshness`
  green. If wizard-bundle CodeQL false positives appear on the PR, dismiss
  via API per the standing note.
- Gates: `npm --prefix frontend run build` (the real type gate — `tsc -b`
  with `noUncheckedIndexedAccess`; `tsc --noEmit` is not sufficient),
  `npm --prefix frontend run lint`, `npm --prefix frontend test`,
  `cargo test` (MongoDB up so the callback tests run), wizard freshness, and
  the §3.1d smoke checks at deploy time.

---

## 4. Test plan

### Existing specs that change

- `frontend/src/pages/oauth-complete.test.tsx` — path updates
  (`/oauth` → `/oauth-complete`); the "neutral, unverified success" test
  becomes the **no-context** case and keeps its assertions as the forged-URL
  guarantee (now also asserting the neutral icon).
- `frontend/src/pages/oauth-launching.test.tsx` — navigate-message test
  extended: launch-context record written (parsed origin, nonce,
  serviceName); invalid `serviceName` dropped without blocking navigation.
- `frontend/src/lib/oauth-popup.test.ts:39-80` — the exact
  `oauth_launch_navigate` payload assertion gains `serviceName` (finding 7:
  this contract test was missing from v1's list).
- `frontend/src/lib/public-paths.test.ts:6`, `telemetry.test.ts:289` — rename
  in place.
- `backend/src/handlers/user_tokens.rs:2242, :2331` — prefix asserts →
  `/oauth-complete?`. No new backend tests: the sole backend change is one
  literal in an already-pinned constructor.

### New coverage (each maps to a review finding or a constraint — no padding)

Completion page (`oauth-complete.test.tsx`):
1. `cc` + context + nonce match, `status=complete` → body names the service,
   title/icon stay neutral (no "authorized"/success icon anywhere) — the
   finding-1 property under the chosen arm.
2. Context present, **nonce mismatch** → fully neutral.
3. Context + nonce match but **`flow=kc`** → fully neutral (finding 6
   regression case).
4. Validated retry updates the stored context nonce before navigating.
5. No-peer copy.

Watch hook (`use-keys` — **real QueryClient, real hook, mocked fetch only**;
finding 7 explicitly bars settling these via a mocked `use-keys` module):
6. Same `keyId`, attempt A returns `failed`; attempt B (new `attemptId`)
   starts and its first read returns `pending_auth`, then `active` — B must
   never settle from A's cached terminal (finding 3's race, exact shape).
7. Reconnect baseline: key starts `active` with unchanged
   `last_authorized_at` → watch stays non-terminal; becomes terminal only
   after the timestamp advances (finding 2).

Dialog lifecycle (`add-key-dialog.test.tsx`):
8. OAuth initiate failure after popup open → `onAuthorizationAborted` fires
   with the announced `attemptId`; device-code initiate failure → no
   `onAuthorizationPending` at all (reordered call).
9. Popup retry success → `onAuthorizationPending` fires again with the same
   `keyId` and a fresh `attemptId`.
10. Dialog close mid-attempt → abort fires; dialog close after in-dialog
    terminal (`authorized`) → abort does **not** fire.

Connect card (`connect-card.test.tsx` — component-level; the cache race is
covered at the hook layer by #6):
11. Pending → "Authorizing" immediately (no keys-list dependence); watch
    `active` (fresh add) → Connected; `failed` → Failed + server message,
    Connect re-enabled; `timedOut` → timed-out state.
12. Reconnect flow: card with `reason_code = NYXID_UNAUTHORIZED` and a
    preserved active key stays "Authorizing" while the timestamp is
    unchanged, flips to Connected via the local verdict once it advances
    (finding 2's second half — `connectedNow` alone can never render this).
13. Abort → neutral "cancelled" state (not the destructive Failed badge),
    Connect enabled, one `["keys"]`/`["keys", keyId]` invalidation fired.
14. Stale attempt: verdict for attempt A arriving after attempt B started is
    ignored (attempt-scoped settle ref).

Deploy smoke: §3.1d checks 1–2 (executable, fail-today-by-construction),
recorded in the PR body as the deploy gate — not vitest.

---

## 5. Risk register

| Risk | Assessment / mitigation |
|---|---|
| **OAuth callback path regression** | Backend diff is one URL literal in `redirect_to_oauth_completion`; all call sites chat-classified (verified); legacy constructors untouched; prefix asserts pin both sides. |
| **Deploy skew** | Frontend-first order + exact `/oauth` 302 alias: after FE deploy, old-backend redirects work via the alias; no window in which a served redirect targets a route the served bundle lacks. HTML `no-cache` bounds stale-bundle lifetime. Reverse-order deploy is the failure mode — called out in the PR body as a hard requirement. |
| **Alias lingers forever** | Tracked removal follow-up in status doc §6 with an explicit condition (one release cycle past backend rename). |
| **nginx template not actually deployed** (provenance unproven) | The steady-state fix (route move + backend rename) works without any nginx change once both images deploy; the alias and cache headers only *improve* the transition wherever the template is live. Open question 1 remains for the owner. |
| **Smoke false-positives** | Both smoke checks execute page JS and assert rendered copy + query scrub; both fail on today's code by construction (§1b). |
| **Reconnect mis-settlement / retry race** | Structurally removed: baseline-aware terminal predicate + attempt-id in the query identity; covered by tests 6–7, 12, 14. |
| **Cancelled-vs-failed confusion** | Dialog close is an explicit abort signal rendering a neutral cancelled state; server's later lazy reconciliation of the abandoned placeholder to `failed` does not repaint the card (attempt-scoped verdict, watch stopped). Residual: closing the dialog in the same instant an authorization lands can read as cancelled until the fired invalidation shows the active key (fresh add) — reconnects in that sliver require a re-click. Accepted, stated. |
| **Forged completion page (finding 1 residual)** | A malicious provider can show the user a neutral "response received" or forged-error page; it cannot elicit any NyxID "connected" claim (none exists on the page) and gains nothing beyond its existing power to deny/complete the real flow. Outcome claims live only on authenticated surfaces (chat card). Signed receipt deferred to the universal design. |
| **`onAuthorizationPending` signature change** | Two internal call sites (`OAuthStep`, `DeviceCodeStep`) and two consumers (`action-card`, new `connect-card`) — updated together in one commit; `tsc -b` enforces closure. |
| **Watch behavior change regresses ActionCard** | ActionCard moves to the same attempt-aware contract (mechanical); its existing tests plus #6–7 cover the shared hook. |
| **Poll load** | Unchanged budget: presence-gated, cadence-tiered, ≤30 min hard ceiling. |
| **Wizard freshness CI** | Deterministically triggered (manifest verified); unconditional rebuild-last sequence in §3.5. |
| **jsdom vs real browsers** | The status-doc §7 manual matrix still gates production enablement; add one pass of the new completion/cancel states to it. |

**Explicitly out of scope** (unchanged): all non-`cc` popup surfaces
(`pc`/`kc`/`sa`/`wz`/`sl`), Phase-2 embed, D-1/D-2 general fixes, upstream
token revocation (F-4), edge log policy (F-5), the prod edge CORS wildcard,
popup-ownership transfer out of the dialog, and the signed completion receipt
(both tracked as universal-design follow-ups).

---

## 6. Open questions

1. **Is `frontend/nginx.conf.template` the literal deployed prod config?**
   Behavior matches it exactly, but provenance isn't observable from the
   repo and Cloudflare fronts it. The plan no longer *depends* on the answer
   (steady state works regardless; alias/cache-headers help wherever the
   template is live), but the owner should confirm before trusting the alias
   for the skew window.
2. ~~Dialog close semantics~~ — resolved: close = explicit cancellation with
   an abort signal and a neutral cancelled card state (§3.2a/3.2c).
3. Should the chat card auto-open `ManageConnectionModal` on success? Current
   plan: no — flipping to Connected is sufficient; "View connection" in the
   popup remains the explicit path.
4. `session_required`/`server_error`: kept as annotated reserved codes
   (§3.3d); strict-deletion is a trivial amendment if preferred.

---

## 7. Review dispositions (finding by finding)

**F1 — nonce-gated context doesn't authenticate the outcome (P1 blocker):
ACCEPTED.** Re-derived independently (§1b): the provider holds the nonce as
OAuth `state` and controls the popup's navigation, so v1's
`context.nonce === search.nonce` gate is satisfiable by exactly the party
most motivated to forge, and v1's retry-nonce update would have kept it
satisfied across generations — quietly turning a §4.2 correlation identifier
into a display capability. v1's enriched "authorized" claim is **withdrawn**.
Of the review's two permitted arms, the plan takes *neutral success copy +
honest indicative content* (service name, concrete next step, full failure
taxonomy) and **DEFERS the backend-authenticated receipt** to the universal
popup design (tracked in status doc §6), with the cost/blast-radius reasoning
and the plainly-stated residual in §3.3. The nonce/context match survives
only as a copy-selector, never as proof.

**F2 — reconnect false-completion and unreachable connected state (P1
blocker): ACCEPTED.** Verified both halves (§1b): the watch lacks the
`last_authorized_at` baseline and `connectedNow` requires
`!needsReauthorization`. Plan: baseline-aware terminal predicate in
`useKeyAuthorizationWatch` (§3.2b), baseline threaded through the extended
`onAuthorizationPending` record (§3.2a), attempt-scoped `localVerdict` giving
reconnect cards a renderable Connected state (§3.2c), reconnect tests 7 and
12 (§4).

**F3 — stale watch settles a fresh retry (P1 blocker): ACCEPTED.** Verified
the cache mechanics and that `retryPopup` never notifies the card (§1b).
Plan: `attemptId` becomes part of the watcher contract and the query identity
`["keys", keyId, "authorization", attemptId]`; `retryPopup` announces new
generations; all local terminal state is attempt-scoped; real-QueryClient
test 6 reproduces the exact race (§3.2a–c, §4). The v1 text claiming the
popup-store guards covered this is retracted — they guard the broadcast
channel, not the card query.

**F4 — pending fires pre-handoff; dialog close ambiguous (should-fix):
ACCEPTED.** Verified the device-code pre-initiate call and silent catch
cleanup (§1b). Plan: pending fires only after a real handoff exists; every
announced attempt that dies without a server verdict fires
`onAuthorizationAborted`; dialog close is defined as cancellation with a
neutral (non-destructive) cancelled card state; terminal-guard prevents
retro-cancelling a completed attempt; lifecycle tests 8–10, 13 (§3.2a, §4).
The review's alternative arm — transferring popup ownership out of the
dialog — is **rejected for the pilot** as a structural refactor
disproportionate to the pilot's scope; recorded as a universal-design
candidate. (The review offered either arm, so this is arm selection, not a
finding rejection.)

**F5 — deploy-skew analysis false; smoke passes pre-fix (P1 blocker):
ACCEPTED.** Re-verified: `/oauth-complete` returns a 200 SPA shell **today**
and the old bundle renders `AppNotFound` (`router.tsx:935`), so v1's
"stale bundle works via SPA fallback" claim is **retracted**, and v1's
header-only smoke was a false positive. Plan: frontend-first rollout, exact
`location = /oauth { return 302 /oauth-complete$is_args$args; }` compat alias
(temporary, removal tracked), SPA HTML `Cache-Control: no-cache` per
`docs/DEPLOYMENT.md` (with the `add_header` inheritance re-declaration), and
an executable two-check smoke that fails on today's code by construction
(§3.1b/d). The review's own concession stands: the cached-301 concern does
not apply to fresh-nonce attempts.

**F6 — enriched behavior not `cc`-gated (should-fix): ACCEPTED.** Verified
the schema accepts all six flow tokens. `displayContext` requires
`search.flow === "cc"`; context write/read and retry mutation are gated;
reserved flows keep byte-identical behavior; regression test 3 (§3.3a, §4).

**F7 — tests miss the real boundaries (should-fix): ACCEPTED.** The v1 plan
omitted `frontend/src/lib/oauth-popup.test.ts` (exact navigate-payload
contract — verified at `:39-80`) and would have "covered" the generation
race with a mocked hook. §4 now mandates real-QueryClient hook tests for the
race and the reconnect baseline, the contract-test update, and the
initiate-failure / retry-propagation / dialog-close / non-`cc` cases, each
constructed to fail against the naive v1 design.

**F8 — wizard freshness deterministic (nit): ACCEPTED.** Verified
`frontend/src/lib/telemetry.ts` (line 76) and `frontend/src/hooks/use-keys.ts`
(line 67) in `cli/src/wizard/bundle-meta/index.manifest`. §3.5 states the
rebuild unconditionally, sequenced last, with the freshness test as the check.
