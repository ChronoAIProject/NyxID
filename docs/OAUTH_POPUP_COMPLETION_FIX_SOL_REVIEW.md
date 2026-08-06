# Adversarial Review: OAuth Popup Completion Fix Plan

Reviewed `docs/OAUTH_POPUP_COMPLETION_FIX_PLAN.md` against branch
`fix/oauth-popup-completion-state` at `21297220`. Phase 1 only: no source
implementation was changed.

## Findings

### 1. P1 blocker: the nonce-gated launch context does not authenticate the completion result

**Evidence.** The shipped invariant says the attempt nonce is a correlation
identifier, not a capability (`docs/OAUTH_CHAT_POPUP_STATUS.md`, section 4.2).
The provider is deliberately sent that nonce as OAuth `state`:

- `frontend/src/schemas/oauth-popup.ts:76-91` accepts an authorization URL only
  when `state == 1cc_<nonce>`.
- `frontend/src/components/dashboard/add-key-dialog.tsx:1723-1745` obtains the
  nonce and navigates the popup to that provider URL.
- The plan's proposed `trustedContext` is only
  `context.nonce === search.nonce` (plan section 3.3(b)).

That permits this sequence without any NyxID callback:

```text
trusted interstitial stores nonce N in popup sessionStorage
  -> provider receives state=1cc_N in its own URL
  -> provider navigates the same popup to
     /oauth-complete?status=complete&flow=cc&nonce=N
  -> stored N matches query N
  -> page says "<service> authorized"
```

The stored record proves that this tab started a launch. It does not prove that
the backend exchanged a code or saved a credential. A malicious or compromised
authorization endpoint knows `N` and controls the popup's next navigation. The
same problem is amplified on retry because an `oauth_retry` message updates the
stored nonce before navigation. This quietly turns the publicly exposed nonce
into the display capability that section 4.2 explicitly says it is not. The
card still settles from server truth, but P3 itself would lie.

**Correction.** Do not render "authorized" from query status plus a nonce
match. Either keep the current neutral success copy, or add a backend-authenticated
completion receipt. A suitable receipt is short-lived and signed, covers at
least `flow`, nonce, status/code, provider identity, and expiry, and contains no
OAuth token or credential material. The page may combine a verified receipt
with the tab-local service label. An opaque, single-use result handle resolved
by a public metadata-only endpoint is another valid design. Nonce matching can
remain correlation and replay narrowing, but cannot be the proof of outcome.

### 2. P1 blocker: the proposed watcher falsely completes reconnects and then cannot render them connected

**Evidence.** Reauthorization intentionally preserves a working active row.
`backend/src/services/user_api_key_service.rs:771-825` resets unhealthy rows
but leaves an active row's status and credential intact while stamping the new
attempt nonce. The dialog therefore records `previousAuthorizationAt` and only
accepts active after `last_authorized_at` advances
(`add-key-dialog.tsx:1522-1539`).

The proposed card watch has no such baseline. `useKeyAuthorizationWatch`
stops at any `active` status (`frontend/src/hooks/use-keys.ts:178-203`), so a
reconnect is reported successful on its first read, before the user authorizes.
The plan's stated follow-on also cannot occur: `ConnectCard.connectedNow`
explicitly requires `!needsReauthorization`
(`connect-card.tsx:78-92`). Invalidating the list therefore does not make a
`NYXID_UNAUTHORIZED` reconnect card connected, contrary to plan section 3.2.

**Correction.** Carry an attempt record containing at least `{attemptId,
keyId, previousAuthorizationAt, startedAt}`. Treat active as terminal only when
`last_authorized_at` differs from the captured baseline (undefined remains the
fresh-add case). Store an attempt-scoped local success verdict, or change the
authoritative block/list predicate so a completed reconnect visibly outranks
the stale `reason_code`. Add a reconnect test that begins with an active key,
proves it remains Authorizing while the timestamp is unchanged, and becomes
Connected only after the timestamp advances.

### 3. P1 blocker: key-id-only settlement lets a stale watch settle a fresh retry

**Evidence.** The plan claims replacing `{keyId, startedAt}` and resetting
`watchSettledRef` is a generation guarantee. It is not:

- `useKeyAuthorizationWatch` caches terminal data under `['keys', keyId]`
  (`use-keys.ts:172-203`).
- OAuth retry reuses `pendingKeyId` (`add-key-dialog.tsx:1555-1577`).
- `retryPopup` does not call `onAuthorizationPending`, so the card is not told
  that a new nonce/generation started.
- `useInitiateOAuth` performs only the GET and does not invalidate or remove the
  exact key query (`frontend/src/hooks/use-providers.ts:111-186`).
- The in-dialog Try Again also reuses the outer `authKey`
  (`add-key-dialog.tsx:1845-1855,3036-3046`).

After attempt A caches `failed`, attempt B on the same key immediately renders
that cached failure before its first fresh GET can return `pending_auth`. The
effect can settle B as failed. The existing popup-store launchId/nonce checks
guard retry mutation and channel transfer (`use-oauth-popup.ts:67-83`); they do
not guard this independent card query, so citing them does not close the race.

**Correction.** Make attempt generation part of the watcher contract and
query identity, for example `['keys', keyId, 'authorization', attemptId]`.
Cancel/reset any older exact-key fetch before enabling the new generation, tie
all local terminal state to `attemptId`, and invoke the lifecycle callback on
popup retry as well as initial initiation. Test the real QueryClient behavior
with the same key id: A returns failed, B starts and returns pending, then B
returns active. A must never settle B.

### 4. P2 should-fix: `onAuthorizationPending` fires before a handoff exists, and dialog close has no explicit outcome

**Evidence.** The plan describes the callback as firing at provider handoff,
but OAuth invokes it before nonce validation and before `popup.navigate`
(`add-key-dialog.tsx:1719-1745`). Device code invokes it before the initiate
request succeeds (`:2188-2201`). The catch paths then delete a fresh placeholder
or preserve a reconnect row without notifying the card (`:1756-1768` and
`:2224-2235`). The adopted watcher can consequently show Authorizing until its
deadline even though no usable provider journey started.

Dialog close is similarly ambiguous. `OAuthStep` unmount always closes the
popup (`add-key-dialog.tsx:1619-1625`), while the card retains `pendingAuth`.
The subsequent GET reconciliation eventually writes status `failed` with the
message "Authorization timed out or was cancelled"
(`backend/src/services/user_api_key_service.rs:947-1035`). The proposed card
then renders a destructive Failed badge for an attempt its own dialog close
aborted. This is not merely pre-existing once the new card state turns it into
a visible terminal verdict.

**Correction.** Resolve Open Question 2 before implementation. Either transfer
popup ownership out of the dialog so closing the dialog leaves the live popup
and card watch intact, or define close as cancellation and send an
attempt-scoped `onAuthorizationAborted` signal that clears the watch, renders a
non-failure Cancelled state, and immediately permits a fresh attempt. Move the
pending callback after successful initiation/validation, or pair every early
callback with an abort signal. Cover OAuth initiate failure, device-code
initiate failure, dialog X, and successful close-after-callback.

### 5. P1 blocker: the deploy-skew analysis is false and the proposed smoke test already passes before the fix

**Evidence.** Independent production probes on 2026-08-06 reproduced P1:

```text
$ curl -D - -o /dev/null 'https://nyx.chrono-ai.fun/oauth?status=complete&flow=cc&nonce=<uuid>'
HTTP/2 301
location: /oauth/?status=complete&flow=cc&nonce=<uuid>
cf-cache-status: DYNAMIC

$ curl -D - -o /dev/null 'https://nyx.chrono-ai.fun/oauth/?status=complete&flow=cc&nonce=<uuid>'
HTTP/2 404
content-length: 0
content-security-policy: default-src 'none'; frame-ancestors 'none'
```

The nginx cause is confirmed at `frontend/nginx.conf.template:34-41`. Moving
the route to `/oauth-complete` avoids that location. However, the plan's claim
that "new backend + stale frontend bundle still works because the SPA fallback
serves index.html" is wrong. The old route table contains only `/oauth`
(`frontend/src/router.tsx:190-194`) and renders `AppNotFound` for unknown paths
(`router.tsx:932-939`, `components/shared/app-not-found.tsx:6-24`). In fact,
this command already returns 200 today, before the route exists:

```text
$ curl -I 'https://nyx.chrono-ai.fun/oauth-complete?status=complete&flow=cc&nonce=<uuid>'
HTTP/2 200
content-type: text/html
```

Therefore the proposed post-deploy `curl -I` smoke cannot detect an old bundle
or a missing client route. Backend and frontend are separate images/services
(`docker-compose.prod.yml:26-60`), and the documented Kubernetes topology uses
separate deployments, so atomic source merge is not atomic runtime rollout.
The nginx template also omits explicit no-cache headers on SPA HTML despite
`docs/DEPLOYMENT.md:648-652` requiring them, leaving the stale-index premise
especially unsafe.

**Correction.** Add a temporary exact compatibility location before the
`/oauth/` prefix, preserving the query, such as `/oauth` -> `/oauth-complete`;
ship the frontend route and alias first, verify it, then switch backend
callbacks. Keep the alias for at least the old OAuth-state/deploy overlap
window, then remove deliberately. Make SPA HTML explicitly `no-cache` while
keeping hashed assets immutable. The smoke must execute the page (Playwright
or equivalent) and assert completion copy/query scrubbing or a BroadcastChannel
wakeup; HTTP 200 alone is insufficient.

The separate cached-301 attack did **not** stick: the live response is dynamic,
the redirect cache key includes the full query URI, and each real attempt uses
a fresh nonce. A cached 301 can affect a repeat visit to that same callback URL,
but it does not poison future unique attempts. That does not solve mixed-version
deployment.

### 6. P2 should-fix: enriched behavior is not gated to the `cc` pilot

**Evidence.** `oauthCompletionSearchSchema` accepts all six reserved flow
tokens (`frontend/src/schemas/oauth-popup.ts:16-21`). The plan defines trusted
context solely as a context/nonce match and then enriches success/error copy;
it never requires `search.flow === 'cc'`. Thus a same-tab URL with `flow=kc`,
`pc`, `sa`, `wz`, or `sl` receives new behavior even though those flows are
explicitly out of scope.

**Correction.** Require `flow === 'cc'` for launch-context use, enriched copy,
retry-context mutation, and any new tests. Reserved flows must retain their
current neutral/degraded behavior until implemented. Add a nonce-matched but
non-`cc` regression case.

### 7. P2 should-fix: the test plan can pass without exercising the generation bugs and omits changed contracts

**Evidence.** `connect-card.test.tsx:24-31` mocks the entire `use-keys` module.
A component test that manually swaps mocked watch status cannot reproduce the
same-key TanStack cache race in finding 3. The plan also changes
`OAuthPopupHandle.navigate` and the exact `oauth_launch_navigate` payload, but
does not list `frontend/src/lib/oauth-popup.test.ts:39-80`, whose current call
and exact object assertion cover that contract. It does not require tests for
reconnect timestamp advancement, early initiate failure, retry callback
propagation, dialog close, or non-`cc` gating.

**Correction.** Keep the proposed copy/path tests, but add a real-hook
QueryClient test for same-key generations; update the popup-manager exact
message test; and add the reconnect, retry, abort, dialog-close, and non-`cc`
cases identified above. These tests should fail against the naive plan, not
merely prove that a mocked enum maps to a badge.

### 8. P3 nit: Wizard Bundle Freshness is deterministically affected, not conditional

**Evidence.** The plan changes `frontend/src/lib/telemetry.ts`. That exact file
is listed in `cli/src/wizard/bundle-meta/index.manifest`, and
`cli/tests/wizard_bundle_freshness.rs:62-76` hashes every listed source file.
The freshness test will therefore fail after the telemetry route rename even
though `package.json` and the lockfile do not change.

**Correction.** State unconditionally that `npm --prefix frontend run
build:wizard` and the resulting `cli/src/wizard/` update are required after all
source edits (and after the final rebase), then run the Rust freshness test.
No `frontend/package.json` or lockfile edit is needed or planned.

## Attacks That Did Not Stick

1. **P1 root cause and destination survived.** The production 301/404 is real,
   the headers identify the Rust 404, and `/oauth-complete` is outside both
   nginx `location /oauth/` and Vite `^/oauth/`. The route move itself is the
   right steady-state namespace decision.

2. **Popup-vs-opener storage partitioning survived.** MDN's documented model
   says sessionStorage is partitioned by origin and top-level tab; a popup gets
   an initial copy from its opener, subsequent changes are separate, and the
   page session survives reload/restore. Because the interstitial writes inside
   the popup, the completion page in that same top-level context can read the
   record. This supports retry-origin continuity, but finding 1 shows it does
   not authenticate callback outcome. Chrome/Firefox/Safari still need the
   existing real-browser matrix for COOP and popup-as-tab behavior.

3. **Legacy callback isolation survived.** All observed calls to
   `redirect_to_oauth_completion` are inside chat-classified branches in
   `backend/src/handlers/user_tokens.rs:576-895`; `redirect_callback` and
   `redirect_to_path` remain untouched. Changing the helper's path literal
   does not alter non-`cc` callback routing. The planner's emitted error-code
   audit also matches those call sites.

4. **Wakeup-only broadcast precedence survived.** Leaving
   `useOAuthPopupReceiver` status/code-agnostic is correct. It invalidates
   authenticated server queries (`frontend/src/hooks/use-oauth-popup.ts:42-51`)
   and should remain an optimization, never the card verdict.

5. **Packaging claim survived.** `git diff -- frontend/package.json
   frontend/package-lock.json` was empty, and the plan introduces no dependency.
   The issue is only the mandatory generated wizard bundle in finding 8.

## Verification Run

- Branch and base: `HEAD == merge-base(HEAD, main) == 21297220`; only the
  planner's untracked plan existed before this review document.
- Live read-only curl probes reproduced `/oauth` 301 -> `/oauth/` backend 404;
  `/oauth-launching` and `/oauth-complete` both returned the SPA shell with 200,
  demonstrating why a header-only new-route smoke is a false positive.
- Targeted frontend popup/card suite: 6 files, 49 tests passed.
- `npm --prefix frontend run build` was attempted after `npm --prefix frontend
  ci`, but the baseline build could not complete: `tsc -b` reported repeated
  `TS2307: Cannot find module 'lucide-react'`. The installed
  `lucide-react@0.563.0` directory contained declarations but was missing the
  package's declared `dist/esm/lucide-react.js` and
  `dist/cjs/lucide-react.js` entry files. This is an incomplete installed
  dependency artifact, not a source failure; the full build gate therefore
  remains unverified. No package or lockfile was edited to work around it.
- `cargo test -p nyxid-cli --test wizard_bundle_freshness`: 1 passed, 0 failed.
  This proves the current bundle is fresh; after the planned telemetry edit a
  rebuild remains mandatory by construction.

## Verdict

**REWORK**

Implementation must not start from this plan until it:

1. replaces nonce-only P3 trust with a backend-authenticated outcome receipt
   (or keeps success neutral, which does not fully satisfy P3);
2. makes card settlement attempt-aware, reconnect-aware, and retry-aware;
3. resolves pending/abort/dialog-close ownership and terminal UI semantics;
4. adds a frontend-first compatibility rollout with an exact old-path alias,
   explicit HTML cache policy, and an executable smoke test; and
5. gates all enriched behavior to `flow=cc` and expands tests around the real
   query/lifecycle boundaries.
