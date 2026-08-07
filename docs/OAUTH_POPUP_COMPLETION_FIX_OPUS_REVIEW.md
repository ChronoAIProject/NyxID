# Adversarial Review: OAuth Popup Completion Fix (implementation)

> **Round 2 (2026-08-07): APPROVE.** All six findings verified fixed at the
> root against independently re-run revert-proofs; three new P3 observations,
> no blockers. See [Round 2](#round-2-verification-2026-08-07) at the end. The
> round-1 body below is preserved unchanged as the record of what was found.

## Round 1

Reviewed `git diff 21297220..HEAD` on `fix/oauth-popup-completion-state`
(`14490fc2`, `5c7a0f58`, `3216a7b9`) against
`docs/OAUTH_POPUP_COMPLETION_FIX_PLAN.md` v2,
`docs/OAUTH_POPUP_COMPLETION_FIX_SOL_REVIEW.md`, and
`docs/OAUTH_CHAT_POPUP_STATUS.md`. No source was changed; every temporary edit
made to reproduce a finding was reverted and the tree is clean.

**Verdict: REWORK.** One P1 defect is confirmed by a reproduction: after the
user explicitly cancels a fresh connect, the chat card renders **Connected**
with the placeholder key still `pending_auth`. The same one-line root cause
already makes a second reachable connect-card path claim Connected before any
authorization. The security boundary the plan was built around **held** under
direct attack, the nginx work is correct and drops no security header, the
route move is complete, and the "red gate" is genuinely host contention.

---

## Gate results (independently re-run, 2026-08-06/07)

| Gate | Result |
|---|---|
| `npm --prefix frontend run build` | **pass** (exit 0) |
| `npm --prefix frontend run lint` | **pass** — 0 errors, 23 pre-existing warnings (matches the claim) |
| `npm --prefix frontend test` (default concurrency, run 1) | 29 failed / 2625 passed |
| `npm --prefix frontend test` (default concurrency, run 2) | 19 failed / 2635 passed — **different identities** |
| Same 11 files (all touched + all run-2 failures) in isolation | **171/171 passed in 21.95s** |
| `cargo test -p nyxid` | **pass** (exit 0) |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | **pass** — `wizard_bundle_is_fresh ... ok` |
| `nginx -t` on the rendered template | **pass** — "syntax is ok / test is successful" |
| Live nginx probe of every location | **pass** — see F-8 |
| Live prod probe (`https://nyx.chrono-ai.fun`) | reproduces the P1 this branch fixes — see F-7 |

---

## Findings

### F-1 — P1 blocker: cancelling a fresh connect renders the card **Connected**

**File:** `frontend/src/components/assistant/blocks/connect-card.tsx:123-136`

`cancelledAuthorizationAdvanced` accepts `matchingKey.is_active` as evidence
that an authorization landed, and on the fresh-connect branch
(`previousAuthorizationAt === undefined`) that is the *only* evidence it
requires:

```ts
const cancelledAuthorizationAdvanced =
  localVerdict?.kind === "cancelled" &&
  matchingKey !== undefined &&
  matchingKey.id === localVerdict.keyId &&
  matchingKey.is_active &&                                  // <-- always true
  (localVerdict.previousAuthorizationAt === undefined ||    // <-- fresh connect: short-circuits
    (matchingKey.last_authorized_at != null && ...));
const connected = connectedNow || block.state === "connected" ||
  localVerdict?.kind === "authorized" || cancelledAuthorizationAdvanced;
```

`is_active` is not an authorization signal. It is the **`UserService` enabled
flag**, and `GET /keys` filters on it, so it is a tautology in the frontend:

- `backend/src/services/user_service_service.rs:663` — `create_user_service`
  hardcodes `is_active: true`, including for the OAuth placeholder.
- `backend/src/services/unified_key_service.rs:3743-3758` — `build_key_view`
  maps `is_active: svc.is_active` (service flag) and `status: ak.status`
  (`"pending_auth"` for the placeholder — `unified_key_service.rs:938-943`).
- `backend/src/services/unified_key_service.rs:2013` → `user_service_service.rs:241`
  → `:259` → `list_user_services` at `:190-196`, whose query is
  `doc! { "user_id": user_id, "is_active": true }`.

**Therefore every `KeyInfo` returned by `GET /keys` has `is_active === true`.**
The pending state lives only on `status`, and `reconcile_pending_oauth_placeholder`
(`user_api_key_service.rs:891-947`) only ever writes `UserApiKey.status` — it
never touches `UserService.is_active`.

**Failure scenario (reproduced).** Chat connect card, `reason_code =
"NYXID_UNAUTHORIZED"`, no existing key → fresh connect → `reconnectMode`
false → `previousAuthorizationAt: undefined`. The dialog mints the placeholder
and announces the attempt. The user closes the dialog without authorizing;
`OAuthStep`'s unmount fires `abortAnnouncedAuthorization()` →
`onAuthorizationAborted` → `localVerdict = cancelled` + `invalidateQueries(["keys"])`.
The refetched list now contains the placeholder (`is_active: true`,
`status: "pending_auth"`), `matchingKey.id === localVerdict.keyId`, and the
`undefined` branch short-circuits.

Note the dialog does **not** delete the placeholder on close — `handleOpenChange`
only calls `resetWizard()` (`add-key-dialog.tsx:2882-2900`); `cleanupPendingAuthKey`
runs only from `handleConnect`'s catch and the device-code step. So the row
persists and the wrong state is durable, not a flash.

**Evidence — reproduction I wrote and ran** (temporary file, since removed;
harness copied verbatim from `connect-card.test.tsx`, only the fixture's
`is_active` corrected to server truth):

```
BADGE/GUIDANCE dump after act(() => aborted?.("attempt-fresh")):
  <div class="... border-success/30 bg-success/10 text-success ...">Connected</div>
  <p ...>Connected — send your request again.</p>

FAIL src/components/assistant/blocks/__repro-connect.test.tsx
  > fresh connect + cancel must not render Connected
  expected document not to contain element, found <div ...>Connected</div>
```

**Correction.** `is_active` must not appear in this predicate at all. Require an
authorization status on both branches, mirroring what the dialog's own
`useKeyAuthorizationStatus` already does (`add-key-dialog.tsx:1546-1552` ANDs
`authorizationAdvanced` with `status === KEY_AUTH_ACTIVE`):

```ts
const cancelledAuthorizationAdvanced =
  localVerdict?.kind === "cancelled" &&
  matchingKey?.id === localVerdict.keyId &&
  matchingKey.status === KEY_AUTH_ACTIVE &&
  (localVerdict.previousAuthorizationAt === undefined ||
    (matchingKey.last_authorized_at != null &&
     matchingKey.last_authorized_at !== localVerdict.previousAuthorizationAt));
```

This also closes F-6 below: with the status check in place, a fresh connect the
user completed *after* closing the dialog correctly flips to Connected, instead
of being stuck at "Connection cancelled" forever.

---

### F-2 — P1 blocker: a connect card claims **Connected** the moment the placeholder is minted

**File:** `frontend/src/components/assistant/blocks/connect-card.tsx:117-122`
(pre-existing at `21297220`, but it defeats this branch's P2 deliverable and
shares F-1's root cause and fix)

```ts
const connectedNow =
  !needsReauthorization &&
  block.catalog_slug !== "custom" &&
  (keys ?? []).some((key) => key.is_active && key.catalog_service_slug === block.catalog_slug);
```

Given F-1's proof that `is_active` is always `true`, this reduces to *"a key row
exists for this slug"*. `reason_code` is optional and its other legal value is
`"NYXID_SERVICE_NOT_CONNECTED"` (`frontend/src/types/assistant.ts:61`) — used
throughout `connect-card.test.tsx` — and on that path `needsReauthorization`
is false.

**Failure scenario (reproduced).** `reason_code =
"NYXID_SERVICE_NOT_CONNECTED"`, one placeholder row (`is_active: true`,
`status: "pending_auth"`):

```
BADGE/GUIDANCE: api-github GitHub Connected  Connected — send your request again.
FAIL > NYXID_SERVICE_NOT_CONNECTED + live pending placeholder
```

Because `authorizing`, `failed` and `timedOut` are all gated on `!connected`
(`connect-card.tsx:140-151`), **the entire state machine this branch was written
to deliver is unreachable on that path** — no spinner, no "Waiting for GitHub…",
no Failed/Timed out/Cancelled, and the Connect button is hidden
(`:291-294`). The P2 owner requirement is not met there.

**Correction.** Same root cause, same fix: gate on the credential status, not
the service flag — e.g. `key.status === KEY_AUTH_ACTIVE` (or at minimum
`key.status !== "pending_auth" && key.status !== KEY_AUTH_FAILED`). Consider
removing `is_active` from `KeyInfo` consumers entirely, or documenting on the
type that it is invariantly `true` for anything `GET /keys` returns.

---

### F-3 — P2: the connect-card test fixtures encode a server shape that cannot occur

**File:** `frontend/src/components/assistant/blocks/connect-card.test.tsx:442-451,
554-563, 611-620`

Every fixture that pairs `status: "pending_auth"` with `is_active: false` is
impossible: `list_user_services` filters `is_active: true` (F-1). This single
untrue fixture value is what hides F-1 and F-2 — with the correct value, the
"renders a matching abort as neutral cancellation" test and the "streams
authorization progress while the placeholder key is pending" test both assert
the wrong thing.

**Correction.** Set `is_active: true` on every `pending_auth` fixture, then fix
the production predicates until those tests pass again. Add one explicit
regression test named for the invariant, e.g. *"a pending placeholder is never
rendered as Connected"*.

---

### F-4 — P2: self-directed deviation #1 (dialog query generation isolation) has zero test coverage

**File:** `frontend/src/hooks/use-keys.ts:65-67`

The status doc calls this out as a deliberate addition beyond plan v2
(`OAUTH_CHAT_POPUP_STATUS.md`: *"Implementation also generation-isolates the
dialog-scoped status query, closing the same-key retry cache hole on both the
card and dialog surfaces."*). Nothing exercises it.

**Evidence — revert experiment R2.** I replaced the conditional key with the
old unconditional `queryKey: ["keys", keyId]` and re-ran every plausibly
relevant suite:

```
$ npx vitest run add-key-dialog.test.tsx use-key-authorization-watch.test.tsx connect-card.test.tsx
 Test Files  3 passed (3)
      Tests  57 passed (57)
```

The reason is structural: `add-key-dialog.test.tsx:54-79` mocks
`@/hooks/use-keys` wholesale, including `useKeyAuthorizationStatus`, so the
real hook never runs in the only suite that mounts `OAuthStep`. And
`use-key-authorization-watch.test.tsx` only covers `useKeyAuthorizationWatch`.

**Correction.** Add the twin of the existing watch test — *"does not expose a
stale failed/active result to a new attempt on the same key"* — against
`useKeyAuthorizationStatus`, in the same `renderHook` + `QueryClientProvider`
harness the watch test already established.

---

### F-5 — P2: the route move itself is untested

**File:** `frontend/src/router.tsx:190-194`

`path: "/oauth-complete"` is the one line that makes the P1 route move real, and
no test observes it.

**Evidence — revert experiment R1.** I set it back to `path: "/oauth"`:

```
$ npx vitest run oauth-complete.test.tsx public-paths.test.ts telemetry.test.ts
 Test Files  3 passed (3)
      Tests  50 passed (50)
```

No test in the repo loads the real route table — `grep -rl "routeTree\|createRouter" src`
over test files returns only `src/lib/assistant/search.test.ts`, which builds
its own `rootRoute.addChildren([assistantRoute])`. `oauthCompleteRoute` is
referenced only at its definition and in the children array.

By contrast every other leg of the move *is* covered — reverting the backend
target fails two tests (R5: `generic_oauth_callback_denial_chat_is_guarded_and_popup_routed`,
`..._with_reaped_state_stays_popup_routed`), and reverting `public-paths.ts`
fails one (R6). The route registration is the gap.

**Correction.** Either assert the registered path directly (import
`routeTree`/the route object and assert `.path`), or make the deploy-gate smoke
of plan §3.1d a checked-in Playwright spec rather than a prose instruction.
Cheapest useful version: a unit test asserting
`oauthCompleteRoute.options.path === "/oauth-complete"` alongside the existing
`isPublicPath` assertion, so the two can never drift.

---

### F-6 — P2: popup handoff failure now destroys the attempt instead of degrading to the manual link

**File:** `frontend/src/components/dashboard/add-key-dialog.tsx:1800-1826`

`handleConnect` was restructured so `await popup.navigate(...)` throws into the
outer `catch`, which now tears everything down *and* deletes the placeholder:

```ts
} catch (err) {
  abortAnnouncedAuthorization();
  popup?.close(); ... setPendingKeyId(null); setAuthorizationUrl(null);
  await cleanupPendingAuthKey(key, { protectExistingKey: reconnectMode });
  setError(err instanceof ApiError ? err.message : "Failed to start OAuth flow");
}
```

The removed code (visible in the diff) closed the popup but **kept**
`authorizationUrl`/`pendingKeyId`, leaving the dialog on the authorizing view
with the "Open `<service>`" anchor and its own poll running — the same fallback
the popup-blocked path relies on (`add-key-dialog.test.tsx` "renders the
manual link when the popup cannot be opened").

`popup.navigate` awaits `ready`, which rejects after **`POPUP_READY_TIMEOUT_MS =
2_000`** (`lib/oauth-popup.ts:13,138-143`). Two seconds is a realistic cold-load
budget for `/oauth-launching` on a slow link — and this branch just added
`Cache-Control: no-cache` to that HTML, so the first popup after a deploy is
exactly the slow case. Result: the user gets "Failed to start OAuth flow", the
placeholder is deleted, and there is no manual fallback.

The same restructure turned a missing `attempt_nonce` from a graceful degrade
(old: close popup, keep the link) into `throw new Error("OAuth provider returned
an invalid attempt nonce")`. `attempt_nonce` shipped in `21297220`, so this
only bites if the deployed backend predates it — but plan §3.1d mandates
frontend-first rollout, which is precisely that window.

Neither change is in plan v2, and the new test ("aborts an announced attempt
when popup navigation fails") ratifies the new behaviour without weighing the
lost fallback.

Related: the outer `catch` has no `generationRef` guard, unlike the inner one it
replaced (`if (generationRef.current !== generation) return;`), so a rejection
that lands after unmount still issues the `DELETE /keys/{id}?only_if_pending=true`.

**Correction.** Keep the abort/announce bookkeeping, but restore the degrade:
catch `popup.navigate` separately, and on failure close the popup + end the
store entry while leaving `authorizationUrl`, `pendingKeyId` and the announced
attempt intact so the anchor path still works. Re-add the generation guard
before the destructive cleanup.

---

### F-7 — P2: a real authorization landing after cancel is never reflected

**File:** `frontend/src/components/assistant/blocks/connect-card.tsx:192-199`

`enabled: pendingAuth !== null && visible` — once a verdict is recorded,
`pendingAuth` is cleared and the watch stops. On the chat path
(`needsReauthorization === true`) `connectedNow` is permanently false, so after a
`cancelled` verdict the only remaining route back to Connected is
`cancelledAuthorizationAdvanced` — the buggy predicate from F-1. Today it
"works" for the wrong reason (it fires immediately on any placeholder); once
F-1 is fixed by requiring `status === KEY_AUTH_ACTIVE` it will fire for the
right reason, on both branches.

This is listed separately because it constrains F-1's correction: **do not fix
F-1 by deleting `cancelledAuthorizationAdvanced`** — that removes the only
server-truth escape hatch after a cancel and leaves a genuinely-connected
service showing "Connection cancelled — you can start again", inviting the user
to mint a second placeholder.

---

### F-8 — P3 nits

1. **`validServiceName` is length-only** (`lib/oauth-popup.ts:44-50`). No trim,
   no control-character/bidi filter, and the label is concatenated into the H1
   as `` `${serviceName}: ${title}` `` (`oauth-complete.tsx:199`). Rendered
   output I captured:

   ```
   H1: GitHub — connected successfully, no action needed: Authorization declined
   ```

   The source is `catalogEntry.name` (admin-managed `DownstreamService`), and
   the value can only reach the popup through a same-origin `postMessage` whose
   `origin` and `source` are both checked (`oauth-launching.tsx:36-47`), so this
   is not attacker-reachable — hence P3. But the plan describes it as "a
   validated ≤64-char label"; the validation is length only. Suggest trimming,
   rejecting `\p{C}` / bidi controls, and rendering the label in its own
   element rather than string-concatenating it into the outcome title.

2. **"View connection"** (`oauth-complete.tsx:230`) is the primary button on a
   page whose entire contract is that it must not assert an outcome. It reads
   as "a connection exists". The destination is server-read
   (`ManageConnectionModal` → `useKey`), so nothing lies downstream — but
   "Back to NyxID" or "Check status" would carry no implication.

3. **`Cache-Control: no-cache` in `location /`** also lands on the real files
   that block serves — `/docs/*.md`, `/docs/search-index.json` — which lose
   heuristic caching. Live probe: `GET /docs/readme.md` → `Cache-Control:
   no-cache`. Scope it to `text/html` (e.g. a nested `location = /index.html`
   or `map $sent_http_content_type`) if doc-asset caching matters.

4. **Duplicate `Cache-Control` on hashed assets** (pre-existing): `expires 1y`
   and `add_header Cache-Control "public, immutable"` both emit, so the response
   carries two `Cache-Control` headers (`max-age=31536000` + `public, immutable`).
   Combining semantics make it harmless; worth folding into one directive while
   the file is open.

5. **Redundant badge branch**: `connect-card.tsx:271-275`'s
   `timedOut ? STATE_LABEL.timed_out : stateLabel` duplicates the
   `localVerdict?.kind === "timed_out"` arm already inside `stateLabel`
   (`:176-184`). The only case it adds is `block.state === "timed_out"` while
   `needsReauthorization` — express that in `stateLabel` and render `stateLabel`
   alone.

6. **The temporary nginx alias has no expiry mechanism** — only a prose comment
   and a bullet in `OAUTH_CHAT_POPUP_STATUS.md` §6. Add a dated tracking issue,
   or the block will outlive the release cycle it was scoped to.

7. **Historical docs still name the old route.** `OAUTH_CHAT_POPUP_STATUS.md`
   was updated, but `docs/OAUTH_CHAT_POPUP_IMPL_PLAN.md:375,490,539,711-712,790-803`
   and `docs/OAUTH_POPUP_FLOW_PLAN.md:29,83,89,120,139-140` still document
   `/oauth` as the completion route. They are historical records, but a reader
   who opens the impl plan gets the wrong route. One "superseded by
   `/oauth-complete`" line at the top of each would settle it.

---

## Attacks that did not stick

**The security boundary held.** I constructed the malicious-provider sequence
the brief asked for and rendered the shipped page against it.

### A. Forged `status=complete` with a matching nonce and no NyxID callback

The provider receives `state=1cc_<N>` and controls the popup's next navigation,
so it can send the popup straight to
`/oauth-complete?status=complete&flow=cc&nonce=<N>` with the trusted launch
context still in sessionStorage. Rendered output I captured:

```
TEXT:      Authorization response received
           Return to your NyxID chat — the Evil Provider connection's status
           appears there and updates automatically.
           View connection   Stay here
BROADCAST: [{"type":"oauth_result","status":"complete","flow":"cc"}]
CHANNEL:   nyxid.oauth.8e1fcf2a-e679-4da2-9f54-2d90cd5f0085
```

No claim of connected or authorized; icon is `"neutral"`, not the success check
(`oauth-complete.tsx:198`, and `oauth-complete.test.tsx` asserts
`document.querySelector(".lucide-circle-check-big")` is null). The copy is a
*directive to go look at the authoritative surface*, which is the correct
construction. The broadcast is a wakeup only: `useOAuthPopupReceiver` handles
`isOAuthResultMessage` by calling `invalidateQueries` and returning
(`hooks/use-oauth-popup.ts:44-52`) — it flips no state. The card's `connected`
never reads the query string, and its `authorized` verdict comes only from
`useKeyAuthorizationWatch`, an authenticated `GET /keys/{id}`. **F-6 of the
prior review is genuinely closed for outcome claims.** (The residual F-1/F-2
defects above let the card lie, but from a *server* read of the wrong field —
not from anything the provider controls.)

### B. Forged `status=error`

A hostile provider can also forge a *failure* display
(`"Evil Provider: Connection could not be saved / Try again. Your existing
connection was not replaced."`) and induce a real re-initiate via "Try again".
This is within the disposition already recorded in `OAUTH_CHAT_POPUP_STATUS.md`
("A provider can forge neutral completion/error display, but cannot cause an
outcome claim"), and F-1 of the prior round made retry non-destructive. Noted,
not raised.

### C. Retry-path nonce update

`writeOAuthLaunchContext({...displayContext, providerOrigin, nonce: nextNonce})`
(`oauth-complete.tsx:148-152`) reopens nothing. The retry message arrives on a
`BroadcastChannel`, which is same-origin-only, so the provider cannot post it;
`isOAuthRetryMessage(data, expectedProviderOrigin)` and
`validateAuthorizationUrl` triple-bind the URL to http(s), no embedded
credentials, `state === 1cc_<nextNonce>`, and `origin === expectedProviderOrigin`
(`schemas/oauth-popup.ts:58-98`). The origin written back is
`authorizationUrl.origin`, which that check already pinned to the recorded
origin. `displayContext` is non-null on every path that reaches this line
(`canRetryHere` requires `expectedProviderOrigin !== null`), and spreading it
cannot introduce a new origin.

### D. Service-label injection

Not reachable cross-origin: the label enters only via
`oauth_launch_navigate`, whose handler checks
`event.origin === window.location.origin && event.source === opener`
(`oauth-launching.tsx:38-47`), and sessionStorage is per-popup (the popup gets
its own copy at `window.open` time). Cross-attempt label confusion is blocked by
the `flow === "cc"` + `launchContext.nonce === search.nonce` gate
(`oauth-complete.tsx:69-76`), which the tests cover in both directions.

### E. nginx — no security header was dropped anywhere

`nginx -t` passed, and I ran the rendered template live on :18080 and probed
every location class:

| Request | Result |
|---|---|
| `GET /oauth?status=complete&flow=cc&nonce=…&code=a%20b%2Bc` | `302` → `Location: /oauth-complete?status=complete&flow=cc&nonce=…&code=a%20b%2Bc` — **query byte-exact, `%20`/`%2B` preserved** — plus all 3 security headers |
| `GET /oauth-complete?status=complete` | `200` + `Cache-Control: no-cache` + all 3 security headers |
| `GET /app.abc123.js` | `200` + `Expires: +1y` + `max-age=31536000` + `public, immutable` + all 3 security headers — **immutable asset caching intact** |
| `GET /oauth/authorize` (proxied) | `502` (backend down) + all 3 security headers |
| `GET /docs/readme.md` | `200` + all 3 security headers (+ the `no-cache` noted in F-8.3) |

`add_header` inheritance is not a problem here: the only two blocks that add
headers (`location /` and the asset regex) both re-declare
`X-Frame-Options`, `X-Content-Type-Options` and `Referrer-Policy`, and every
other location declares none, so it inherits the server-level set. HSTS and CSP
are not in this file — HSTS is added at the edge (observed on prod) and CSP by
`mw/security_headers.rs` — so neither can be dropped by this change.
`location = /oauth` is an exact match, so `/oauth/*` still reaches the backend.

### F. The prod P1 is real, and `/oauth-complete` is already reachable

```
$ curl -D - "https://nyx.chrono-ai.fun/oauth?status=complete&flow=cc&nonce=550e8400-…"
HTTP/2 301
location: /oauth/?status=complete&flow=cc&nonce=550e8400-…
$ curl -L …            → HTTP/2 404, body 0 bytes
$ curl -D - "https://nyx.chrono-ai.fun/oauth-complete?status=complete&flow=cc"
HTTP/2 200
```

The 301 is nginx's `auto_redirect` for the `proxy_pass` location `/oauth/`, as
diagnosed. It reproduces the blank-404 popup exactly. And prod HTML currently
carries **no** `Cache-Control` at all (only `Last-Modified`/`ETag`), so the
`no-cache` addition is a real fix for heuristic staleness, not cargo cult.

**Smoke checks fail on pre-fix code, as claimed.** Check 2 is the curl above.
Check 1 fails because `21297220`'s router registers the completion page at
`/oauth`, so `/oauth-complete` falls to `defaultNotFoundComponent: AppNotFound`
(`router.tsx:935`) — it renders neither "Authorization declined" nor the query
scrub. (Both checks remain prose, not executable — see F-5.)

### G. Non-`cc` flows are untouched

All seven `redirect_to_oauth_completion` call sites
(`user_tokens.rs:582,601,634,711,812,827,889`) pass
`OAuthFlowKind::ChatConnect.as_wire()`; `kc`/`pc`/`sa`/`wz`/`sl` never reach the
completion redirect and still use `redirect_callback` / `redirect_to_path`.
`location = /oauth` shadows only a bare `/oauth`, and the backend's
`.nest("/oauth", oauth_routes)` (`routes.rs:800-822,1430`) defines no root route
— every real endpoint is `/oauth/<something>` and still proxies.

### H. Deviation #4 (`fs.rmSync` reinstall) disturbed nothing

`git diff 21297220..HEAD` touches no `package.json`, no lockfile, and no
`Cargo.lock`; `git status --porcelain` is empty. The wizard bundle rebuild is
legitimate: `index.hash` changed because `use-keys.ts` and `telemetry.ts` are in
`index.manifest` (source-closure hashing), while the compiled
`cli/src/wizard/assets/index.html` is byte-identical because both changed
symbols are tree-shaken out of the wizard entry — the 990 KB bundle contains
zero occurrences of `oauth-complete` and none of the telemetry sensitive-path
patterns. `wizard_bundle_is_fresh` passes.

### I. The red gate is host contention, not a new flake

- 18 of 19 run-2 failures are literally `Test timed out in 5000ms` (one at
  10000ms); the 19th is a `getByRole` cascade *after* a timed-out click chain.
- The failing set changes run to run (29 vs 19) and is dominated by files this
  branch never touches: `date-picker`, `admin-oauth-clients` data-table,
  `node-detail`, `devices-bind`, `credential-push`, `org-developer-apps-tab`,
  `confirm-panels`, `assistant-wire-log-panel`.
- **No changed or added test file failed in either run** —
  `connect-card`, `oauth-complete`, `oauth-launching`,
  `use-key-authorization-watch`, `action-card`, `oauth-popup`, `public-paths`,
  `telemetry` were green throughout. `add-key-dialog.test.tsx`'s 4 failures are
  all in pre-existing "custom endpoint path" / "ConnectVerifyStep integration"
  blocks untouched by this diff.
- All 11 relevant files pass in isolation: **171/171 in 21.95 s**.
- `uptime` during the run: `load averages: 201.66 201.05 132.84` on a 12-core
  machine — ~17× oversubscribed, with unrelated node/Xcode processes competing.
- No dev server is answering: `lsof -i :3000 -i :3001 -sTCP:LISTEN` is empty
  (checked before the first run), so the "live server answering test fetches"
  failure mode from `reference_vitest_contention_flakes` is ruled out.

I could not manufacture a deterministic failure in any changed test. The
`--maxWorkers=2` green run recorded in the status doc is the honest reading.

---

## Verdict

**REWORK.**

### Must fix before merge

1. **F-1 (P1)** — `cancelledAuthorizationAdvanced` must require
   `matchingKey.status === KEY_AUTH_ACTIVE`, not `matchingKey.is_active`.
   Cancelling a fresh connect currently renders **Connected**. Reproduction
   included; keep the predicate (see F-7) rather than deleting it.
2. **F-2 (P1)** — `connectedNow` has the same defect and makes the whole new
   state machine unreachable for `reason_code = "NYXID_SERVICE_NOT_CONNECTED"`.
   Same one-field fix.
3. **F-3 (P2)** — correct every `pending_auth` fixture to `is_active: true` and
   add a named regression test; the false fixture is what hid both P1s.
4. **F-5 (P2)** — cover the route registration. Reverting `router.tsx:191` to
   `/oauth` today leaves the suite green; the headline fix has no test.
5. **F-4 (P2)** — cover deviation #1 (`useKeyAuthorizationStatus` attempt
   isolation), which is currently mocked out of existence.

### Should fix

6. **F-6 (P2)** — restore the manual-link degrade on popup handoff failure and
   re-add the generation guard before the destructive cleanup.

F-8's nits are optional and can ride a follow-up.

---

# Round 2 verification (2026-08-07)

Re-reviewed the whole of `git diff 21297220..HEAD` after four further commits —
`bd471112`, `0ae48268`, `604172fd`, `f9e6ce95`. Every revert-proof below I ran
myself rather than reading from the round-2 transcript; every temporary edit was
restored and `git status --porcelain` was empty after each block.

## Gate results (re-run)

| Gate | Result |
|---|---|
| `npm --prefix frontend run build` | **pass** (exit 0) |
| `npm --prefix frontend run lint` | **pass** — 0 errors, 23 pre-existing warnings |
| `npm --prefix frontend test` (default concurrency) | 5 failed / 2655 passed (2660) — all timeout-shaped, all in files this branch never touches |
| Same 12 files (all touched + all failures) in isolation | **180/180 passed** |
| `cargo test -p nyxid` | **4997 passed; 0 failed** (320.69 s) |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | **pass** — `wizard_bundle_is_fresh ... ok` |
| `nginx -t` + live probes | **pass** (config byte-identical to round 1) |

## Per-finding outcome

| # | Round-1 severity | Outcome |
|---|---|---|
| F-1 | P1 | **Verified fixed** |
| F-2 | P1 | **Verified fixed** |
| F-3 | P2 | **Verified fixed** |
| F-4 | P2 | **Verified fixed** |
| F-5 | P2 | **Verified fixed** |
| F-6 | P2 | **Verified fixed**, and better than the pre-round-1 behaviour |
| F-7 | P2 (constraint) | **Verified resolved** as a consequence of F-1's correction |
| F-8 | P3 nits | 2 of 7 addressed incidentally; the rest remain open by choice |

### F-1 / F-2 — fixed at the root

`connect-card.tsx:118-134` now reads `key.status === KEY_AUTH_ACTIVE` in both
predicates; `is_active` no longer appears anywhere in the card. I re-ran my two
round-1 reproductions **verbatim** (same harness, same server-truth fixture)
plus a new third case, against round-2 code:

```
 Test Files  1 passed (1)
      Tests  3 passed (3)
   ✓ fresh connect + cancel must not render Connected
   ✓ NYXID_SERVICE_NOT_CONNECTED + live pending placeholder is Authorizing, not Connected
   ✓ a late real success after cancel does flip to Connected (F-7)
```

Both fixes are load-bearing under independent mutation:

```
RV1  connectedNow: status === KEY_AUTH_ACTIVE  ->  key.is_active
       × renders a matching abort as neutral cancellation and ignores stale aborts
       × never renders a real pending placeholder as Connected
       Tests  2 failed | 18 passed (20)

RV2  cancelledAuthorizationAdvanced: status === KEY_AUTH_ACTIVE  ->  matchingKey.is_active
       × renders a matching abort as neutral cancellation and ignores stale aborts
       Tests  1 failed | 19 passed (20)
```

**Every branch of the state machine now agrees on the same field.** I
enumerated all four ways `connected` can become true:

1. `connectedNow` — `status === "active"` on a slug-matching row from
   `GET /keys`. Authenticated read. ✓
2. `localVerdict?.kind === "authorized"` — `watch.authorized`, i.e.
   `status === KEY_AUTH_ACTIVE && authorizationAdvanced` from an authenticated
   `GET /keys/{id}`. ✓
3. `cancelledAuthorizationAdvanced` — `status === KEY_AUTH_ACTIVE` + key-id
   match + (reconnect only) `last_authorized_at` advancement. ✓ Note this is
   *exactly* branch 2's rule: `useKeyAuthorizationWatch` also short-circuits the
   timestamp when `previousAuthorizationAt === undefined`, so the two branches
   are consistent rather than merely both-safe.
4. `block.state === "connected"` — the one remaining path not derived from a
   NyxID key read. It is the upstream chat block's own assertion, arriving over
   the authenticated assistant transport; a provider cannot emit chat blocks, so
   it is outside the popup threat model. Pre-existing and unchanged; recorded
   here so the boundary is explicit rather than assumed.

I also checked the negative directions that the tests do not: a reconnect whose
`last_authorized_at` does **not** advance stays Cancelled even though the row is
`active` (correct — the preserved old credential is not a new authorization),
and a reconnect with `last_authorized_at: null` on both sides stays Cancelled
until a real stamp lands.

`status === "active"` is sound as an authorization proxy because
`pending_oauth` (`unified_key_service.rs:886-888`) covers exactly the
credential-free OAuth/device-code mint the connect card produces, so the
placeholder is always `pending_auth`. One residual (P3, below): `build_key_view`
defaults `status` to `"active"` when a `UserService` has no `UserApiKey` at all.

### F-3 — fixed

Fixtures at `connect-card.test.tsx:444-451, 558, 610-626` now use
`is_active: true` + `status: "pending_auth"` with a comment explaining the
server invariant, and the renamed test *"never renders a real pending
placeholder as Connected"* asserts `queryByText("Connected")` is absent. RV1
shows that assertion is what catches the regression.

### F-4 — fixed

`use-key-authorization-watch.test.tsx:101-120` now exercises the **real**
`useKeyAuthorizationStatus` through a real `QueryClientProvider`.

```
RV3  queryKey: attemptId ? ["keys", keyId, "authorization", attemptId] : …  ->  ["keys", keyId]
       × does not expose a stale terminal result to a retried dialog attempt
       Tests  1 failed | 2 passed (3)
```

### F-5 — fixed

`router.tsx:190` exports `oauthCompleteRoute`; `router.test.ts` asserts its
initialized `fullPath`. Asserting `fullPath` rather than `options.path` is the
stronger choice — it observes the value after the route tree is built.

```
RV4  path: "/oauth-complete"  ->  "/oauth"
       AssertionError: expected '/oauth' to be '/oauth-complete'
       Tests  1 failed (1)
```

### F-6 — fixed, with all four sub-behaviours independently pinned

`handleConnect` now (a) validates every popup-path URL through
`validateHttpAuthorizationUrl`, (b) keeps the full nonce/state binding when a
nonce is present, (c) releases the popup to the manual link — preserving the
announced attempt, the placeholder, `authorizationUrl` and the dialog poll — on
both missing-nonce and navigate-failure, and (d) generation-guards the
destructive catch.

```
RV5a  remove the OUTER catch guard (add-key-dialog.tsx:1849)
        × does not clean up a fresh placeholder when initiation rejects post-unmount
        AssertionError: expected "vi.fn()" to not be called at all, but actually been called 1 times

RV5b  (control) remove the INNER guard after ensureKey (:1752) instead
        Tests  1 passed | 36 skipped (37)

RV6   remove the try/catch around popup.navigate
        × falls back to the manual link when popup handoff fails

RV7   missing-nonce fallback -> throw (pre-round-2 behaviour)
        × falls back to a safe manual link when an older backend omits the popup nonce
        TestingLibraryElementError: Unable to find role="link" and name `/Open GitHub/i`

RV8   POPUP_READY_TIMEOUT_MS 5_000 -> 2_000
        × allows five seconds for a cold interstitial before falling back

RV9a  remove the protocol/credential check from validateHttpAuthorizationUrl
        × accepts real cross-origin providers only with exact state binding
        × accepts only credential-free HTTP URLs for a manual fallback
```

**RV5a/RV5b settle the concern the brief raised about the redesigned
post-unmount test.** The test's `await waitFor(() => expect(popup.close)...)` is
satisfied by the unmount cleanup alone, so on its own it would not prove the
catch ever ran — but removing *only* the outer guard makes it fail with a real
`DELETE`, and removing *only* the inner guard leaves it passing. The test is
therefore specific to the outer guard, and the catch demonstrably executes. The
redesign is correct.

### F-7 — resolved

Requiring `status === KEY_AUTH_ACTIVE` restored the escape hatch's meaning
instead of removing it: my third reproduction shows a cancelled fresh connect
whose authorization later lands does flip to Connected. The predicate was kept,
as recommended.

## New behaviour introduced in round 2, reviewed as a first-time deviation

### N-1 — the `javascript:` claim reproduced, and extended

I did not take their three-case test on trust. I ran my own hostile corpus
directly against `validateHttpAuthorizationUrl`:

```
HOSTILE (want BLOCKED):
  BLOCKED  "javascript:alert(1)"
  BLOCKED  "JaVaScRiPt:alert(1)"
  BLOCKED  "   javascript:alert(1)"
  BLOCKED  "java\tscript:alert(1)"
  BLOCKED  "java\nscript:alert(1)"
  BLOCKED  " javascript:alert(1)"
  BLOCKED  "data:text/html,<script>alert(1)</script>"
  BLOCKED  "vbscript:msgbox(1)"      BLOCKED  "blob:https://evil.example/abc"
  BLOCKED  "file:///etc/passwd"      BLOCKED  "about:blank"
  BLOCKED  "chrome://settings"       BLOCKED  "ws://evil.example/"
  BLOCKED  "https://user:pass@github.com/oauth"
  BLOCKED  "https://:pass@github.com/oauth"
  BLOCKED  "https://user@github.com/oauth"
nonce-bound validator, javascript: url carrying a matching state:  null
```

Every scheme-escape and credential form is blocked, including the case-,
whitespace-, tab-, newline- and NUL-prefixed `javascript:` variants that defeat
naive prefix checks (the WHATWG URL parser strips leading C0/space and
lowercases the scheme before the check sees it). The refactor also preserved
`validateAuthorizationUrl` semantics exactly — same protocol/credential/origin/
`state` conjunction, just factored — and RV9a proves the state-binding test
still depends on it.

### N-2 — P3: the fallback validator base-resolves junk to same-origin

Same corpus, informational rows:

```
  ALLOWED! ""          -> http://localhost:3000/
  ALLOWED! "not a url" -> http://localhost:3000/not%20a%20url
  allowed  "//evil.example/protocol-relative" -> http://evil.example/protocol-relative
  allowed  "/relative/path" -> http://localhost:3000/relative/path
```

`new URL(rawUrl, window.location.origin)` is inherited from the original
validator, so this is not new — but the no-nonce fallback is a *new consumer* of
it, and it is the one place where the result is handed to the user as a link
without a `state` check to catch nonsense. A broken/empty `authorization_url`
would render a manual "Open GitHub" link pointing at NyxID's own origin instead
of surfacing an error. Not a security escape (the source is NyxID's own backend,
and no hostile scheme or unexpected cross-origin survives), so P3. Correction if
touched: require an absolute URL, or reject `url.origin === window.location.origin`
on the fallback path so the link is provably external.

### N-3 — P3: `attempt_nonce: ""` and `attempt_nonce: undefined` diverge

`add-key-dialog.tsx:1788-1801` validates under `if (nonce !== undefined)` but
falls back under `if (!nonce)`. An empty-string nonce hits the first branch and
**throws** into the destructive catch; an absent nonce degrades gracefully. The
distinction (malformed vs. absent) is defensible, but it is implicit. One
comment, or aligning both on `undefined`, would remove the trap.

### N-4 — P3: the anchor href is normalized on the popup path only

`authorizationHref = validatedAuthorizationUrl?.href ?? response.authorization_url`
means the legacy (`launch !== "popup"`) path still renders the raw, unvalidated
backend string — unchanged from before, but the new code reads as though
authorization URLs are validated generally. Worth a comment so a future reader
does not assume the legacy anchor is covered.

### N-5 — the 5 s ready timeout is low-risk

`setAuthorizationUrl(...)` runs **before** `await popup.navigate(...)`, so the
dialog is already showing the manual link while the 5 s budget elapses; the
timeout only delays `releasePopupToManualLink()`. Raising it strictly widens the
window in which a cold interstitial still succeeds, and expiry is now
non-destructive. No objection.

### N-6 — P3 observation carried forward

`build_key_view` (`unified_key_service.rs:3743-3746`) defaults `status` to
`"active"` when a `UserService` has no `UserApiKey`. That is semantically right
for `auth_method: "none"` services, but it means `status === "active"` also
reads as connected for a row whose api key row is missing. Not reachable through
the OAuth connect path (which always mints a `pending_auth` key), so noted, not
raised.

Separately, the card trusts `status === "active"` for Connected but never reads
`status === "failed"` for Failed — a server-reconciled abandoned flow with no
local verdict falls through to the block's default copy. It errs toward
not-connected, which is the safe direction.

## Round-1 items that did NOT regress

Each was re-checked, not assumed. `git diff 3216a7b9..HEAD` shows these files
**byte-identical**: `frontend/nginx.conf.template`,
`backend/src/handlers/user_tokens.rs`, `frontend/src/pages/oauth-complete.tsx`,
`frontend/src/pages/oauth-launching.tsx`, `frontend/src/lib/public-paths.ts`,
`frontend/src/lib/telemetry.ts`, `frontend/src/hooks/use-keys.ts`,
`frontend/src/hooks/use-oauth-popup.ts`,
`frontend/src/components/assistant/blocks/action-card.tsx`,
`cli/src/wizard/**`, `frontend/package.json`, `frontend/package-lock.json`,
`Cargo.lock`. On top of that:

- **Security boundary / neutral copy** — re-rendered the forged-provider
  completion against round-2 code:
  `"Authorization response received / Return to your NyxID chat — the Evil
  Provider connection's status appears there and updates automatically."`,
  broadcast `[{"type":"oauth_result","status":"complete","flow":"cc"}]`,
  success check-icon present: **false**. Unchanged, still outcome-neutral.
- **nginx** — `nginx -t` passes; live probes reconfirm the alias
  (`302 → /oauth-complete?status=complete&flow=cc&nonce=…&code=a%20b%2Bc`,
  **query byte-exact including `%20`/`%2B`**), `Cache-Control: no-cache` on SPA
  HTML, `Expires`+`max-age=31536000`+`public, immutable` on hashed assets, and
  all three security headers on every location class.
- **Non-`cc` flows** — `user_tokens.rs` untouched; all seven completion call
  sites remain `ChatConnect`.
- **Lockfiles / `package.json` / `Cargo.lock`** — unchanged from `21297220`.
- **Wizard bundle** — none of the round-2 changed files
  (`router.tsx`, `lib/oauth-popup.ts`, `schemas/oauth-popup.ts`,
  `connect-card.tsx`, `add-key-dialog.tsx`) are in `index.manifest`, so no
  rebuild was required and the `3216a7b9` hash is still correct;
  `wizard_bundle_is_fresh` passes.

## Flakiness

No new instability. The 5 failures in my unconstrained run are all
`Test timed out in 5000ms` and all in `admin-oauth-clients.test.tsx` and the
`AddKeyDialog — custom endpoint path` / `catalog template path` blocks — the
latter last touched in `9939b908`, i.e. before this branch. No test added or
modified by this branch failed in any run. All 12 relevant files pass in
isolation (**180/180**), and `uptime` during the run showed
`load averages: 193.36 94.61 55.16` on a 12-core machine with no dev server on
:3000/:3001. The new tests are structurally safe: the 5 s-timeout test uses fake
timers with `vi.useRealTimers()` in `afterEach`, and the two hook tests drive a
real `QueryClient` with `shouldAdvanceTime`. The `cargo` suite passed
**4997/4997** outright on my run, with no elapsed-time failure.

---

# Final verdict: APPROVE

All six findings are fixed at the root, not papered over, and every fix is
pinned by a test that I independently proved fails when the production line is
reverted. The one test whose provenance the brief flagged as suspect — the
post-unmount outer-guard test — passes a specificity check (RV5a fails, RV5b
control passes). The security boundary, nginx behaviour, non-`cc` flows,
lockfiles and wizard bundle are all unregressed, verified rather than assumed.
Three new P3 observations (N-2, N-3, N-4) are non-blocking and can ride a
follow-up, as can the round-1 F-8 nits that remain open.

This approves the **code**. It does not discharge the deploy gates the status
doc already owns: frontend-image-first rollout, the §3.1d executable smoke
(still prose, not a checked-in spec), and the §7 manual browser matrix.

## Are the two owner requirements met?

**(a) "I need card to know the state when auth is connected." — Met.**
The card reaches a visible Connected state with no user click, driven by an
authenticated `GET /keys/{id}` watch that is presence-gated, deadline-bounded,
refetches on window focus, and survives the dialog being closed; plus the
`["keys"]` list for non-reauthorization cards. Failed, Timed out and Cancelled
are now genuinely distinct states with their own copy, badge and recovery
action, and — the thing that was actually broken — Authorizing is now reachable
at all, because a pending placeholder no longer masquerades as Connected.

One scoped limit the owner should know: on a `NYXID_UNAUTHORIZED` card,
automatic settlement requires that an attempt was announced through that card's
dialog (or that a cancelled attempt's key later reaches `active`). If a user
reconnects entirely elsewhere — say from `/keys/{id}` in another tab — without
ever opening this card's dialog, the card keeps showing "Reauthorization
required" until a new turn re-renders the block. That follows from the static
block contract, not from this change, but it is the boundary of "the card knows
the state".

**(b) "I need redirect page to be indicative of what has happened." — Met, as
re-scoped by the accepted security decision, and the owner should see the
re-scoping stated plainly.**
The page now names the service, maps all eight backend error codes to specific
copy, distinguishes "nothing to complete" from a real attempt, offers Try again
/ Close, and scrubs the query. It is indicative of everything **except a success
claim**: because NyxID hands the provider the attempt nonce as OAuth `state`,
and the provider controls where the popup navigates next, a success page can be
forged, so success stays outcome-neutral ("Authorization response received")
and points the user at the authoritative chat card. That is the right trade —
being *wrong* about a connection is worse than being *quiet* about it — and it
is exactly the boundary I attacked and could not break. If the owner wants the
popup itself to say "connected", that needs the backend-signed completion
receipt already specced as a follow-up in `OAUTH_POPUP_FLOW_PLAN.md` §10.1; it
cannot be done with the nonce alone.

---

# Round 3 — rebase resolution review (2026-08-07)

The round-2 APPROVE was given at base `21297220`. The branch has since been
rebased onto `047dc0b4` (~60 commits of `main`), where PR #1384 had relocated
`ActionCard`'s `pendingAuth` into `frontend/src/stores/pending-connect-store.ts`
— a semantic collision with the lifecycle work I approved. PR #1391.

**Scope: the rebase resolution and post-approval changes only.** Round 2 stands
for everything that survived unchanged, and I established exactly what that is
before reviewing anything.

## Scope isolation — what actually changed

I compared the branch's *own contribution* before and after the rebase, per
file, rather than diffing the trees (which `main`'s 60 commits would swamp):
`git diff 21297220..f9e6ce95 -- <f>` vs `git diff 047dc0b4..HEAD -- <f>`.
Of 26 source files the branch touches, **19 contributions are byte-identical**
and 7 differ.

Stronger still, these 14 files' **final content** is byte-identical to the tip
I approved (`git rev-parse f9e6ce95:<f>` == `HEAD:<f>`):

```
frontend/nginx.conf.template                      frontend/src/lib/oauth-popup.ts
frontend/src/pages/oauth-complete.tsx             frontend/src/lib/public-paths.ts
frontend/src/pages/oauth-launching.tsx            frontend/src/lib/telemetry.ts
frontend/src/components/assistant/blocks/connect-card.tsx    (+ its .test.tsx)
frontend/src/hooks/use-keys.ts                    frontend/src/hooks/use-oauth-popup.ts
frontend/src/types/oauth-popup.ts                 frontend/src/router.test.ts
frontend/src/components/dashboard/add-key-dialog.test.tsx
frontend/src/hooks/use-key-authorization-watch.test.tsx
```

`frontend/package.json`, `frontend/package-lock.json` and `Cargo.lock` are
untouched relative to the new base.

The 7 that differ:

| File | Nature of the difference |
|---|---|
| `backend/src/handlers/user_tokens.rs` | context drift only — **semantically identical** (same one production line, same two test asserts) |
| `frontend/src/router.tsx` | context drift only — **semantically identical** (`export` + `/oauth-complete`) |
| `frontend/src/components/dashboard/add-key-dialog.tsx` | **comments only** — 6 added lines, zero code, addressing my round-2 N-3 and N-4 |
| `frontend/src/stores/pending-connect-store.ts` | the store extension (main's file) |
| `frontend/src/components/assistant/blocks/action-card.tsx` + `.test.tsx` | the resolution |
| `frontend/src/schemas/oauth-popup.ts` + `.test.ts` | the post-approval N-2 fix |

The dialog delta, verified strictly (every `+`/`-` line in the post-approval
diff):

```
+        // Present-but-malformed is a broken contract and throws; absent means
+        // a pre-nonce backend and degrades to the manual link below. An empty
+        // string counts as malformed on purpose — a backend that sends the
+        // field must send a valid one.
+      // Normalized only on the popup path; the legacy (non-popup) anchor still
+      // renders the backend's raw string, unvalidated, as it always has.
```

Both round-2 nits answered in place, no behaviour touched.

## 1. Does the merged lifetime preserve both invariants at once?

**Yes.** The two card types do not share a lifecycle surface, and I verified the
premises rather than assuming them.

- **Only `ActionCard` consumes the store.** `grep -rn usePendingConnectStore
  frontend/src` (non-test) returns the store itself and four lines in
  `action-card.tsx`. `ConnectCard` still holds `pendingAuth` /
  `pendingAuthRef` / `localVerdict` in component state — its file is
  byte-identical to the approved tip.
- **The store key cannot collide.** Records are keyed by `block.block_id`, and
  action-card block ids come from the transport's
  `newId(prefix) => `${prefix}-${crypto.randomUUID()}``
  (`aevatar-transport.ts:808-810`). A surviving record can only ever rejoin the
  exact card that created it — including across the conversation switches and
  history refetches #1384 exists for, and it is dropped on reload together with
  the local block it clears.
- **Watch generations cannot collide either.** The cache key is
  `["keys", keyId, "authorization", attemptId]`, and `attemptId` is minted per
  initiation inside `AddKeyDialog.announceAuthorization` with
  `crypto.randomUUID()`. Two cards each own an `AddKeyDialog` instance, so two
  simultaneous attempts on the *same key* still occupy different cache entries
  and different settle guards.
- **The divergent abort wiring is inert, not conflicting.** With no
  `onAuthorizationAborted` prop, `abortAnnouncedAuthorization()` clears the
  dialog's internal `announcedAttemptIdRef` and calls
  `authorizationAbortedRef.current?.(…)` — `undefined` — so the store is never
  touched. Main's "dialog close ≠ abandonment" survives literally.
- **No orphan record can point at a deleted placeholder.** After the round-2
  restructure, nothing between `announceAuthorization` and the end of
  `handleConnect` can throw to the outer catch: the no-nonce case `return`s
  through `releasePopupToManualLink()`, and `popup.navigate` has its own
  `catch`. The outer catch — the only caller of `cleanupPendingAuthKey` — can
  therefore only fire *before* a store record exists.
- **The one genuinely shared singleton is already guarded.**
  `useOAuthPopupStore.begin` refuses a second concurrent attempt
  (`if (get().attempt !== null) return false`), so an ActionCard launch cannot
  clobber a live ConnectCard launch. Unchanged by the rebase; noted because it
  is the only cross-card mutable state that exists.
- **`reconciledRef`'s stranded-card rollback still reads a self-contained
  record** (keyId + attemptId + previousAuthorizationAt + startedAt) with no
  external registry to fall out of sync with.

### Finding R-1 — P2: the deliberate `onAuthorizationAborted` omission is neither tested nor commented

This is the load-bearing claim of the whole resolution, and it is the one thing
holding it up that a future contributor cannot see.

```
RV-A  add to ActionCard's <AddKeyDialog>:
        onAuthorizationAborted={() => { endPendingAuth(block.block_id); }}
      npx vitest run action-card.test.tsx
        Test Files  1 passed (1)
              Tests  22 passed (22)          <-- green
```

Deliberately breaking main's invariant is invisible to the suite. The reason is
structural: `action-card.test.tsx` mocks `AddKeyDialog`, and the mock only fires
the callbacks its own buttons are wired to — `onAuthorizationAborted` is never
among them. Main's natural guard,
*"keeps the card busy when the dialog is dismissed mid-authorization"*
(`action-card.test.tsx:697`), drives that mock and so cannot observe the prop.

Contrast the other card, where the same mutation is caught immediately:

```
RV-B  remove onAuthorizationAborted from ConnectCard
        × renders a matching abort as neutral cancellation and ignores stale aborts
        × lets an advanced reconnect authorization override a simultaneous cancellation
        Tests  2 failed | 18 passed (20)
```

And the rationale is not written down either: `grep -in abort
action-card.tsx` returns **nothing**. Every other non-obvious decision in that
same JSX block is commented — why `launch="popup"`, why `onPopupViewResult`
closes the dialog — but the one introduced by a conflict resolution, and the one
a reader is most likely to "fix" for consistency with `ConnectCard`, is silent.

**Failure scenario.** Someone notices the asymmetry, adds
`onAuthorizationAborted` to `ActionCard` "for consistency", CI stays green, and
#1384's stranded-`Connecting` card is back in production.

**Correction.** Two small things in files already being touched: (a) have the
mocked dialog's dismiss button call `onAuthorizationAborted`, and assert the
card stays busy and `usePendingConnectStore.getState().attempts["action-card-1"]`
survives — that turns RV-A red; (b) one comment on the absent prop stating the
invariant and pointing at #1384.

This is a coverage gap, not a defect — the shipped behaviour is correct.

### Finding R-2 — P3: `if (!attemptId) return;` turns a broken record into a stranded card

`action-card.tsx:253-262`. If a store record ever lacked `attemptId`, the settle
effect returns early — and so does the guard below it, since
`watchSettledRef.current === attemptId` is `undefined === undefined`. The card
would mount `in_progress`, skip the stranded-card rollback (because
`pendingAuth !== null`), and never settle: precisely the class #1384 fixed.
Only TypeScript prevents it today — one call site, spreading a typed
`AuthorizationAttempt`, and the store has no `persist` middleware so no legacy
shapes can be rehydrated. Low probability, bad failure mode; `?? "legacy"` or
simply dropping the redundant guard (a record always has a `keyId`) would be
more robust.

## 2. Are the re-keyed settle guards still stale-safe?

**Yes — and `ActionCard` got strictly safer without changing behaviour.**

- `ActionCard` never reads `is_active` (`grep` returns nothing). The only input
  that settles it to `completed` is `watch.authorized`, i.e.
  `status === KEY_AUTH_ACTIVE && authorizationAdvanced` from an authenticated
  `GET /keys/{id}`. The F-1/F-2 class has no foothold here.
- `previousAuthorizationAt` threads dialog → store → watch. `ActionCard` passes
  no `reconnectKey`, so it is always `undefined` there, which makes
  `watch.authorized ≡ watch.status === KEY_AUTH_ACTIVE` — **exactly main's
  pre-rebase semantics**. The resolution neither loosens nor tightens
  `ActionCard` today, and the reconnect gate is correctly wired if one is ever
  added. That is what a rebase resolution should look like.
- `attemptId` isolation survived the move: it is in the store record, the query
  key, the settle guard (`watchSettledRef.current === attemptId`), and the
  manual-outcome guard (`watchSettledRef.current = pendingAuth.attemptId`).
  Because it now lives in the store rather than being re-minted per mount, a
  remount rejoins the *same* cache generation — better than a component-state
  version would have been.
- `ConnectCard`'s four `connected` branches are byte-identical to the file I
  verified in round 2; nothing needs re-deriving.

Focused re-run across all 11 affected files: **147/147 passed**.

## 3. Did anything previously verified regress?

- **nginx, completion copy, launching interstitial, ConnectCard, `use-keys`,
  `use-oauth-popup`, public paths, telemetry, types** — byte-identical final
  content to the approved tip (table above). Round-2 evidence stands verbatim;
  no re-run needed and none would be meaningful.
- **Non-`cc` flows** — re-audited against `main`'s 168 changed lines in
  `user_tokens.rs`: still exactly **seven** `redirect_to_oauth_completion` call
  sites, **all seven** `OAuthFlowKind::ChatConnect.as_wire()`, target still
  `{frontend_url}/oauth-complete`. `main` introduced no new completion redirect
  and no non-`cc` one.
- **Query preservation / headers** — `nginx.conf.template` is byte-identical, so
  the round-2 live probes (302 with `%20`/`%2B` intact, `no-cache` on SPA HTML,
  `public, immutable` on hashed assets, all three security headers on every
  location class) transfer unchanged.
- **Wizard bundle** — rebuilt post-rebase (`index.html` + `index.hash`);
  `wizard_bundle_is_fresh ... ok`; the bundle contains **zero** `oauth-complete`
  strings and only the same 19 pre-existing admin `oauth-client*` occurrences,
  i.e. no stale completion route baked in.
- **Lockfiles** — untouched.
- **Build / lint** — `npm run build` exit 0 (incl. the mock-footprint
  assertion); `npm run lint` 0 errors, the same 23 pre-existing warnings.

## 4. The post-approval N-2 fix

`validateHttpAuthorizationUrl` now parses with `new URL(rawUrl)` — no base — so
`""`, `not a url`, `/relative/path` and `//protocol-relative` are rejected
instead of silently resolving into same-origin NyxID links on the nonce-free
fallback path. It is pinned:

```
RV-C  new URL(rawUrl)  ->  new URL(rawUrl, window.location.origin)
        × rejects non-absolute URLs instead of base-resolving them to NyxID
        Tests  1 failed | 5 passed (6)
```

**It does not narrow anything legitimate.** I re-ran my hostile corpus, extended
with the four junk shapes and eight real provider URL shapes:

```
HOSTILE / JUNK — 20/20 BLOCKED, now including:
  BLOCKED  ""            BLOCKED  "not a url"
  BLOCKED  "/relative/path"        BLOCKED  "//evil.example/protocol-relative"
  (plus every javascript:/data:/vbscript:/blob:/file:/about:/chrome:/ws: form
   and all three embedded-credential forms, unchanged from round 2)

LEGITIMATE PROVIDER URLS — 8/8 allowed:
  https://github.com/login/oauth/authorize
  …?client_id=x&scope=repo%20user&state=1cc_<nonce>
  http://localhost:3001/oauth/authorize
  HTTPS://GitHub.com/...            -> https://github.com/...   (scheme/host normalized)
  https://login.microsoftonline.com/common/oauth2/v2.0/authorize?a=1
  https://accounts.google.com:443/o/oauth2/v2/auth  -> default port stripped
  https://xn--80ak6aa92e.com/auth   (punycode IDN)
  https://provider.example/auth?redirect_uri=https%3A%2F%2Fnyx%2Ffoo

nonce-bound validator still binds:  origin-pinned match -> URL, wrong origin -> null
```

Note the fix also reaches `validateAuthorizationUrl`, which delegates — so the
*nonce-bound* path is absolute-only too. That is correct (an OAuth authorization
endpoint is always absolute) and the new test asserts it on both functions.

## Gates I ran

| Gate | Result |
|---|---|
| `npm --prefix frontend run build` | **pass** (exit 0) |
| `npm --prefix frontend run lint` | **pass** — 0 errors, 23 pre-existing warnings |
| Focused suite, 11 affected files | **147/147 passed** |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | **pass** |
| Revert-proofs RV-A / RV-B / RV-C | run; results above |
| Hostile + legitimate URL corpus | run; results above |

Per the brief I did not re-run the full suites. The reported 2757/2757 and
5115/5115 are consistent with my focused runs, with round 2's contention
pattern, and with build/lint being green.

---

# Round 3 verdict: APPROVE WITH CHANGES

The rebase resolution is **correct**. Main's store-based lifetime is preserved
intact, this branch's cancellation semantics are preserved intact, and the two
live on structurally separate lifecycles — separate state owners, UUID-keyed
records, UUID-keyed watch generations, and the only shared singleton already
refusing concurrent use. I could not construct an ordering where one card
settles, mutates, or strands the other. `ActionCard`'s port is behaviour-
preserving on main's semantics while correctly threading the reconnect gate,
and no path can still reach a connected/completed outcome without an
authenticated read of `UserApiKey.status`. Everything I verified in rounds 1–2
that matters is byte-identical, and the post-approval N-2 fix is right, pinned,
and costs no legitimate provider URL.

**The change I want before this lands: R-1.** The resolution's central decision
— `ActionCard` deliberately not wiring `onAuthorizationAborted` — is currently
held up by nothing a future contributor can see: no comment, and a mutation that
breaks it leaves the suite 22/22 green. That is the same class of gap as
round-1 F-4 and F-5, and here the regression it invites is main's own #1384. It
is ~10 lines in two files that are already being edited.

Not a blocker on correctness: if the owner prefers to merge #1391 now and land
R-1 as an immediate follow-up, that is a defensible call — nothing shipped is
wrong today. R-2 is a P3 and can ride whenever.

The deploy gates from round 2 are unchanged and still outstanding:
frontend-image-first rollout, the §3.1d executable smoke (still prose), and the
§7 manual browser matrix. Both owner requirements remain met — the ConnectCard
state machine and the completion page are byte-identical to the versions I
assessed against them in round 2.

