# OAuth Chat Popup Pilot — Current Status (Authoritative)

**This document supersedes the five earlier popup docs.** It was produced by an
independent realignment pass on 2026-08-05 that re-verified every claim below
against `git diff origin/main...HEAD` and re-ran every gate — dispositions here
are verified, not inherited. A fresh agent should need to read nothing else;
the superseded docs remain only as the review trail:

- `OAUTH_POPUP_FLOW_PLAN.md` — universal design v3 (mostly unimplemented)
- `OAUTH_POPUP_FLOW_CODEX_REVIEW.md` — 24 findings against the design
- `OAUTH_CHAT_POPUP_IMPL_PLAN.md` — chat-pilot implementation plan
- `OAUTH_CHAT_POPUP_SOL_REVIEW.md` — 7 P1s against the plan
- `OAUTH_CHAT_POPUP_OPUS_REVIEW.md` — 3 P1 / 8 P2 against the implementation

Branch: `feature/oauth-chat-popup-flow`, 15 commits, rebased onto
`origin/main` (merge-base == main tip `682189ad` at verification time), open
(un-drafted) PR #1349 against `main`. Updated 2026-08-05 for `403f6fc3`
(CodeQL-driven refactor of `validateAuthorizationUrl`; see §5 CodeQL entry
and §8).

Completion-state repair: `fix/oauth-popup-completion-state` moves the shipped
completion surface out of the backend-owned `/oauth/` namespace, makes chat
cards settle from attempt-aware authenticated reads, and keeps the completion
page outcome-neutral. Design and adversarial review:
`OAUTH_POPUP_COMPLETION_FIX_PLAN.md` v2 and
`OAUTH_POPUP_COMPLETION_FIX_SOL_REVIEW.md`.

---

## 1. What shipped and why

The chat assistant's ConnectCard now runs OAuth in a small isolated popup
window instead of navigating the chat tab away. **Provider consent pages can
never be iframed** (`github.com`, `accounts.google.com`, and every serious IdP
send `X-Frame-Options: DENY` / framing-hostile CSP), so a top-level popup
window is the only way to keep the chat conversation alive underneath while
the user authorizes.

This is the **chat-only pilot** (`flow=cc`) of the universal popup design.
Everything else in that design — providers grid, SA providers, CLI wizard,
social login, Phase-2 embed — is deliberately unimplemented (§6).

### Runtime flow

```
ConnectCard (chat) — click "Connect"
  │ window.open('/oauth-launching', <client launchId>, 'popup,…')  ← synchronous, keeps user activation
  │ POST-equivalent GET /api/v1/providers/{id}/oauth/initiate?flow=cc&key_id=…
  ▼
/oauth-launching (interstitial, public route, same origin)
  │ installs validated postMessage listener → sets window.opener = null
  │ → posts {type:"oauth_launch_ready", launchId} to the captured opener
  │ opener replies {type:"oauth_launch_navigate", launchId, nonce, url, serviceName?}
  │ interstitial validates, stores tab-local display context, and self-navigates
  ▼
Provider consent → GET /api/v1/providers/callback?code&state=1cc_<nonce>
  │ nonce-guarded token exchange & write
  ▼
302 → /oauth-complete?status=…&flow=cc&code=…&nonce=…   (public completion route)
  │ history.replaceState scrubs the query on mount
  │ BroadcastChannel "nyxid.oauth.<nonce>" → wakeup only
  ▼
opener invalidates key queries; UI settles from authenticated server reads
```

Retry never relaunches from the opener (no user activation there): the opener
re-runs initiate and transfers `{nextNonce, url}` over the *old* attempt's
channel; the popup validates and self-navigates (§4 invariant 3).

---

## 2. Wire contracts

### Flow-kind enum

`backend/src/models/oauth_flow_kind.rs`, mirrored by
`frontend/src/types/oauth-popup.ts` (`OAUTH_FLOW_TOKENS`):

| Wire token | Variant | Status |
|---|---|---|
| `cc` | ChatConnect | **implemented (this pilot)** |
| `kc` | KeyConnect | reserved, unimplemented |
| `pc` | ProviderConnect | reserved, unimplemented |
| `sa` | ServiceAccountConnect | reserved, unimplemented |
| `wz` | WizardConnect | reserved, unimplemented |
| `sl` | SocialLogin | reserved, unimplemented |

The initiate endpoint (`handlers/user_tokens.rs:384-390`) accepts any of the
six via `?flow=`; unknown tokens are a 400 `ValidationError`. Only `cc`
engages the attempt-nonce lifecycle; other tokens keep byte-identical legacy
behavior. `OAuthState.flow_kind` and `attempt_nonce`, and
`UserApiKey.oauth_attempt_nonce`, are `Option` with
`serde(default, skip_serializing_if = "Option::is_none")` — absent on every
legacy document, before and after round-trip (tested).

The chat OAuth `state` parameter is `1cc_<attempt_nonce>` — self-discriminating
(`CHAT_CONNECT_STATE_PREFIX`), so callbacks classify chat-vs-legacy without a
DB read. `attempt_nonce` is a canonical lowercase UUID v4; the parser rejects
uppercase/braced/non-v4 forms.

### `/oauth-complete` completion query contract

`GET /oauth-complete?status=…&flow=…&code=…&nonce=…` — all params optional, validated
by `frontend/src/schemas/oauth-popup.ts` (`oauthCompletionSearchSchema`,
zod `.catch(undefined)` — bad values degrade to generic copy, never throw):

- `status`: `complete` | `error`
- `flow`: one of the six wire tokens (pilot emits `cc` only)
- `code` (errors): `access_denied`, `provider_error`, `state_expired`,
  `state_replayed`, `state_invalid`, `session_mismatch`, `session_required`,
  `exchange_failed`, `server_error`
- `nonce`: canonical UUID v4; names the BroadcastChannel `nyxid.oauth.<nonce>`

Broadcast messages (`frontend/src/types/oauth-popup.ts`): `oauth_result`
(status/flow/code — wakeup only), `oauth_action` (view_result | retry |
dismiss), `oauth_ack`, `oauth_retry` ({nextNonce, url}); plus the launch
handshake pair `oauth_launch_ready` / `oauth_launch_navigate` over
postMessage. The navigate message may carry a length-capped `serviceName` used
only for display. The interstitial stores `{providerOrigin, nonce,
serviceName?}` under `nyxid.oauth.launch-context` in the popup's
sessionStorage. **No message or launch-context record ever carries tokens,
key material, or credential material.**

---

## 3. Backend attempt lifecycle (the part two review rounds got wrong)

All in `backend/src/services/user_api_key_service.rs`:

- `begin_chat_oauth_attempt` (initiate): filter
  `{user_id, connection_id, credential_type != node_managed}`; pipeline update
  that **always** stamps `oauth_attempt_nonce` but resets
  credential/status/error fields **only** when status ∈
  `{failed, refresh_failed, expired, pending_auth}` — the same gate the legacy
  reconnect path uses. An active credential survives initiating, cancelling,
  or abandoning a reconnect.
- `write_chat_oauth_tokens_to_key` (success): filter
  `{connection_id, oauth_attempt_nonce, status ∉ {revoked, failed}}`; writes
  tokens, stamps `last_authorized_at`, `$unset`s the nonce. `matched_count == 0`
  → handler raises "OAuth attempt is no longer current".
- `fail_chat_oauth_placeholder` (denial/failure): matches by nonce; pipeline
  conditionally fails **only** `pending_auth` rows; any other status keeps all
  credential data and merely loses the matching nonce (a late callback from
  that generation can then never land).
- **Bidirectional invalidation**: every legacy mutation path —
  `write_oauth_tokens_to_key`, `fail_pending_placeholders_for_provider`,
  `fail_connection_placeholder`, `promote_node_managed_api_key`,
  `reset_provider_api_key_state` — `$unset`s `oauth_attempt_nonce`, so a
  dashboard reconnect invalidates any in-flight chat attempt and vice versa.
- Frontend reconnect polling (`useKeyAuthorizationStatus`) treats `active` as
  terminal **only when `last_authorized_at` advanced** past its pre-attempt
  value, so a preserved active credential is never mistaken for completion.

---

## 4. Security invariants (must hold in any future change)

1. **Opener severance before external navigation.** The interstitial clears
   `window.opener` *before* posting `oauth_launch_ready`; the parent refuses
   to send the provider URL before that ack (validated on `event.origin`,
   `event.source === popup`, exact `launchId`); the parent never assigns
   `popup.location`. No provider document ever holds a live opener.
2. **`attempt_nonce` is a correlation identifier and channel selector, NOT a
   secret capability or authorization boundary.** It transits the provider as
   `state` and appears in the `/oauth-complete` query (and hence provider/edge logs) by
   design. Everything that mutates server state is independently
   authenticated and nonce-generation-guarded; broadcast payloads carry no
   secrets.
3. **Retry URLs are triple-bound** (`validateAuthorizationUrl`): http(s), no
   embedded credentials, `state == 1cc_<nextNonce>`, and origin equal to the
   provider origin recorded in the parsed launch-context record by the trusted
   interstitial. Context reads, display enrichment, and retry updates require
   `flow=cc` plus a matching nonce; that match is correlation, never outcome
   proof.
   Retry is not offered at all when that recorded origin is absent.
   Since `403f6fc3` the validator returns the parsed `URL` object (`URL |
   null`) instead of a boolean, and every navigation sink (`window.location.
   assign`, `postMessage` url fields, the sessionStorage origin write) uses
   the returned object's `.href` / `.origin` — never the raw input string —
   so the sanitisation sits on the static-analyser-visible taint path. Keep
   it that way: new call sites must check `=== null` and navigate with the
   returned object.
4. **`oauth_result` is a wakeup only.** The opener never trusts message or
   query content for state; it invalidates queries and settles from
   authenticated `/keys/:id` reads (+ `last_authorized_at` advancement for
   reconnects). The completion page renders only fixed copy selected by
   enum-validated tokens; a forged `status=complete` shows neutral
   "Authorization response received" copy and flips nothing. Even with a
   matching tab-local service label, the page only directs the user back to
   the chat where authenticated reads show the result; it never says connected
   or authorized.
5. **Initiate is non-destructive** (§3). Credential-clearing writes happen
   only under the legacy unhealthy-status gate or after a nonce-guarded
   exchange/denial on a `pending_auth` row.
6. **Legacy flows stay byte-identical**: BSON shapes (skip_serializing_if),
   redirects, and the DB-free missing-code fast path are pinned by tests.
   Malformed/legacy state strings keep the legacy dashboard redirect; only
   canonical `1cc_<uuid-v4>` states route to `/oauth-complete`.
7. **`/oauth-complete` and `/oauth-launching` are exact-match public routes**
   (`lib/public-paths.ts`) that render before auth and issue no authenticated
   requests. Never broaden to `/oauth/*` — those are the backend IdP surface.
8. **Popup-closed detection is client-only.** The 1s `isClosed()` watchdog
   shows a hint and a "Start again" reset; it never fails/deletes server
   state, because COOP can make a live popup appear closed.
9. New GET routes in this area must respect the delegated-read deny rules in
   `mw/auth.rs` (CLAUDE.md Critical Rule 5) — the popup routes added none.

---

## 5. Finding ledger — verified dispositions (2026-08-05)

### Sol plan review (7 P1) — all fixed in implementation; re-verified at HEAD, no regressions

| # | Finding | Disposition |
|---|---|---|
| S-1 | `isPublicPath` missing popup routes | **Fixed, holds** — exact entries + tests (`public-paths.ts:24-25`) |
| S-2 | No guaranteed opener severance before external nav | **Fixed, holds** — ready/navigate handshake (§4.1) |
| S-3 | Same-origin retry rule breaks real providers | **Fixed, holds** — http(s) + state-binding + recorded-provider-origin (§4.3) |
| S-4 | Stale denial can poison a retried placeholder (D-2 interplay) | **Fixed, holds** — attempt-generation guard; made bidirectional in round 4 (F-3) |
| S-5 | `Option::None` would serialize as BSON null | **Fixed, holds** — `skip_serializing_if` + reserialize-absence tests |
| S-6 | Nonce mistaken for authentication on the channel | **Disposition revised** — nonce reclassified as correlation-only (§4.2); mutations authenticated server-side; per-attempt channels + retry-channel capability transfer retained. Accepted, not a secret-channel redesign. |
| S-7 | `peek_oauth_state` added to the shared legacy missing-code path | **Fixed, holds** — `1cc_` prefix classifies without DB; extended to the denial branch in round 4 (F-2) |

### Opus implementation review (3 P1 + 8 P2) — fixed in `15c61a4d` / `a7bf4d31`, dispositions verified against code

| # | Sev | Finding | Disposition |
|---|---|---|---|
| F-1 | P1 | Initiate destroyed a live credential before user approval | **Fixed** — status-gated pipeline reset; denial preserves active rows; `last_authorized_at` poll guard (§3). Mongo-gated tests cover active-preserved, denial, replacement, and generation staleness. |
| F-2 | P1 | Denial with TTL-reaped state escaped popup routing (dashboard rendered inside popup) | **Fixed** — the `Err` arm of the denial peek falls back to `chat_attempt_nonce_from_state` (`handlers/user_tokens.rs:546-554`); regression test `generic_oauth_callback_denial_chat_with_reaped_state_stays_popup_routed`. |
| F-3 | P1 | Generation guard was one-directional; legacy writes left stale nonces live | **Fixed** — all five legacy mutation paths `$unset` the nonce (§3). Residual: `reconcile_pending_oauth_placeholder` Pass 2 sets `failed` without unsetting — **inert**, every nonce-consuming write excludes `failed` or only unsets. |
| F-4 | P2 | State insert + nonce stamp are two writes; two-tab race can orphan an exchanged upstream grant | **Consciously accepted** (impl plan §0.2.8) — NyxID data stays correct via the nonce guard; upstream revocation of the discarded grant belongs to all-flow retry work. |
| F-5 | P2 | Nonce appears in the `/oauth-complete` URL and provider/edge logs | **Consciously accepted, reframed** — correlation identifier, not a capability (§4.2); edge log/query policy is deployment-owned. |
| F-6 | P2 | Completion success state fully attacker-assertable | **Fixed for outcome claims** — success stays neutral ("Authorization response received"), the optional tab-local service label only points back to the authoritative chat card, and no popup copy says connected/authorized. A provider can forge neutral completion/error display, but cannot cause an outcome claim. Signed outcome receipts remain a universal-design follow-up. |
| F-7 | P2 | No popup-closed watchdog; `isClosed()` dead code | **Fixed** — 1s poll → hint + "Start again" reset; deliberately client-only (§4.8). |
| F-8 | P2 | Retry silently dead when provider origin absent from sessionStorage | **Fixed** — `canRetryHere` gates the button; neutral "return to your NyxID tab" copy instead of a dead CTA; no destructive re-initiate fires. |
| F-9 | P2 | `focus` listener likely cancels the success auto-close | **Fixed** — focus listener removed; pointerdown/keydown remain. Real-browser confirmation stays on the manual matrix (§7). |
| F-10 | P2 | Branch behind main; unrelated red test at old base | **Fixed** — rebased; merge-base equals `origin/main` tip; full suites green at HEAD (§8). |
| F-11 | P2 | Destructive reset not `user_id`-scoped | **Fixed** — `begin_chat_oauth_attempt` filter includes `user_id` (caller passes the effective owner for org flows). |

**Tally: Sol 7/7 fixed-and-holding (one disposition revised, accepted). Opus:
8 fixed, 2 consciously accepted (F-4, F-5), 1 partially fixed with the
remainder accepted by design (F-6). Nothing unaddressed.**

### Completion-state fix review (2026-08-06)

`OAUTH_POPUP_COMPLETION_FIX_SOL_REVIEW.md` found four P1 blockers and four
lower-severity issues in the first repair plan. V2
(`OAUTH_POPUP_COMPLETION_FIX_PLAN.md`) accepted all eight: neutral completion
copy replaced the forgeable outcome claim; watches became attempt- and
reconnect-aware; dialog close became explicit neutral cancellation; rollout
became frontend-first with an exact temporary alias and executable smoke;
enrichment became `cc`-only; real-query lifecycle tests and unconditional
wizard rebuilding were added. Implementation also generation-isolates the
dialog-scoped status query, closing the same-key retry cache hole on both the
card and dialog surfaces.

### CodeQL on PR #1349 (post-realignment) — fixed in `403f6fc3`

| # | Sev | Finding | Disposition |
|---|---|---|---|
| CQ-1 | high | `js/xss` on `oauth-launching.tsx:51` (`window.location.assign(message.url)`) | **Fixed** — analyser blind spot, not a real hole: the sanitiser was a separate boolean-returning function, so CodeQL could not connect it to the sink. `validateAuthorizationUrl` now returns `URL \| null` and all sinks consume the returned object's `.href`/`.origin` (§4.3), putting sanitisation on the taint path. No validation predicate was weakened or removed; `oauth-complete.tsx` additionally gained an explicit re-validation (with `expectedProviderOrigin`) directly before its `assign`. Fixed by making the sanitiser analyser-visible — **not** by dismissing/suppressing the alert. Alert cleared (`code-scanning/alerts?pr=1349&state=open` empty). |
| CQ-2 | medium | `js/client-side-unvalidated-url-redirection`, same line | **Fixed** — same root cause and same change as CQ-1; alert cleared. Regression test added: interstitial rejects `javascript:` URLs and navigates only with the parsed, protocol-approved URL (`oauth-launching.test.tsx`). |

---

## 6. Deliberately out of scope for this branch

- **D-1** (account-linking login-CSRF: callback succeeds with no session) and
  **D-2** (`state` not single-use on the denial path) — pre-existing bugs on
  `main`, tracked in `OAUTH_POPUP_FLOW_PLAN.md` §7 as independent PRs. The
  `cc` attempt-generation guard already neutralizes D-2's retry-poisoning
  consequence *for chat*; the general fixes remain separate.
- All non-`cc` popup surfaces: providers grid (`pc`), keys dashboard (`kc`),
  SA providers (`sa`), CLI wizard (`wz`), social login (`sl`).
- Phase-2 embedded/iframe variant from the universal design.
- Durable server-side cancel: the popup's dismiss is labelled "Close", not
  "Cancel" — OAuth-state cancellation transactions are out of the pilot.
- Upstream token revocation for orphaned grants (F-4 residual).
- Edge/CDN access-log query-string policy (F-5 residual) — deployment-owned.
- Backend-authenticated completion receipts for outcome-specific popup copy —
  deferred to the universal six-flow design; the `cc` pilot stays neutral.
- Remove the temporary nginx exact `/oauth` → `/oauth-complete` compatibility
  alias after one full release cycle with no pre-rename backend callbacks.

## 7. Known residuals — manual browser matrix required before production enablement

jsdom cannot validate real windowing semantics. Before enabling for real
users, manually verify in Chrome, Firefox, and Safari (desktop) plus one
mobile browser:

1. **COOP behavior**: popup handle usability after provider navigation;
   `isClosed()` false-positives (watchdog must stay hint-only).
2. **Refused `window.close()`**: the manual-return copy path when the browser
   refuses script-close (popup opened without script opener rights).
3. **Popup-as-tab** (Android Chrome / iOS Safari always tab): flow completes,
   just not visually contained.
4. **F-9 follow-through**: success auto-close actually fires (no stray focus
   /interaction event cancelling it) and pointer/keyboard still cancels.
5. Popup-blocked fallback anchor (`noopener noreferrer`) end-to-end.
6. Completion/cancellation states: service-labelled neutral success, every
   emitted error-code message, query scrubbing, card auto-settlement, and
   dialog-close rendering neutral Cancelled rather than Failed.

## 8. Gate results

### Completion-state repair — independently re-run 2026-08-06

The frontend dependency tree was restored from the committed lockfile before
any source edit (`frontend/node_modules` removed, then
`npm --prefix frontend ci`: 608 packages, exit 0). The required unmodified
baseline `npm --prefix frontend run build` then passed. Neither
`frontend/package.json` nor a lockfile changed.

| Gate | Result |
|---|---|
| `npm --prefix frontend run build` | passed: `tsc -b`, main Vite build, credential-accept build, and mock-footprint check all completed (one existing lightningcss pseudo-class warning) |
| `npm --prefix frontend run lint` | passed: 0 errors, 23 pre-existing warnings outside the changed files |
| `npm --prefix frontend test -- --maxWorkers=2` | **2654/2654 passed** in 222 files (225.31s) |
| `cargo test -p nyxid -- --test-threads=8` | **4997/4997 passed**, 0 ignored (320.91s) |
| `cargo fmt --all -- --check` | clean |
| `git diff --check` | clean |

The exact unconstrained `npm --prefix frontend test` command was attempted
twice on the shared host. It produced four unrelated timeout failures after
2649 passes on the first run and 19 different unrelated timeout failures
after 2634 passes on the second. The same complete suite passed with two
workers and normal test assertions/timeouts; the changing failures and the
green constrained run identify shared-host scheduling pressure rather than a
deterministic product failure.

The executable smoke failed before the route fix by construction:

- Vite `/oauth-complete?...` rendered `404 / Page not found`, with
  `hasExpectedCopy: false` and the query still present.
- Production `/oauth?...` navigation failed with
  `net::ERR_HTTP_RESPONSE_CODE_FAILURE` after the nginx prefix redirect.

After the fix, a built Vite preview and a container using the production nginx
template both rendered `Authorization declined` at `/oauth-complete`, scrubbed
`window.location.search` to the empty string, and returned 200. The production
template's exact `/oauth?...` alias returned 302 with the query preserved in
`Location: /oauth-complete?...`; the final HTML returned
`Cache-Control: no-cache` plus the expected frame, content-type, and referrer
security headers. `nginx -t` passed. Wizard-bundle build/freshness results are
recorded by the final packaging commit that follows all source edits.

### PR #1349 realignment — independently re-run 2026-08-05

Rust gates ran at `87599c59` (realignment pass); the only commit since,
`403f6fc3`, is frontend-only (verified from its diff: all 8 touched files are
under `frontend/src/`), so the frontend gates plus the wizard-freshness check
were re-run at `403f6fc3` (delta pass) and the backend gates were not.

| Gate | Verified at | Result |
|---|---|---|
| `cargo fmt --all -- --check` | `87599c59` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | `87599c59` | clean |
| `cargo nextest run -p nyxid` | `87599c59` | **4997/4997 passed**, 0 skipped (MongoDB on 27018 was up, so the Mongo-gated lifecycle tests genuinely ran) |
| `cargo test -p nyxid-cli --test wizard_bundle_freshness` | `403f6fc3` | passed (schemas/hooks/pages files touched by `403f6fc3` are outside the wizard source closure) |
| `npm --prefix frontend run lint` | `87599c59` | 0 errors (23 pre-existing warnings) |
| `npm --prefix frontend run test` | `403f6fc3` | **2631/2631 passed** (221 files; +1 = the new interstitial URL-validation regression test) |
| `npm --prefix frontend run build` | `403f6fc3` | passed (tsc -b + vite) |
| GitHub CI on PR #1349 | `403f6fc3` | 22 pass / 5 skipping / 0 fail; open CodeQL alerts on the PR: none |

## 9. CI gotchas for anyone touching this branch

- **Wizard bundle rebuild must be the LAST commit after any rebase.**
  `hooks/use-keys.ts` is in the wizard source closure; the freshness check
  hashes sources, so rebasing *after* rebuilding stales the hash and reddens
  three CLI CI jobs. Sequence: rebase → all source edits →
  `npm --prefix frontend run build:wizard` → commit `cli/src/wizard/` →
  `cargo test -p nyxid-cli --test wizard_bundle_freshness`. (`87599c59` is
  exactly this commit and touches only `cli/src/wizard/`. `403f6fc3` landed
  after it without a rebuild and freshness still passes, because its files
  are outside the wizard source closure — do not rely on that for other
  frontend files; when in doubt run the freshness test.)
- **`npm run build` is the real frontend type gate** (`tsc -b` with
  `noUncheckedIndexedAccess`); `tsc --noEmit` passes code that CI rejects.
- CI (`ci.yml`) gates PRs into `main`/`dev` — this PR targets `main`, so it
  runs. A rollup PR would get no CI.
- Wizard-bundle PRs can trip known CodeQL false positives in the minified
  `index.html`; dismiss via API if they appear.

## 10. Readiness verdict

**Un-drafted and ready for human review.** All 3 Opus P1s are genuinely fixed
in code with regression tests; all 8 P2s are fixed or explicitly
accepted-and-documented; none of Sol's 7 plan-round P1 fixes regressed during
the fix round; the branch is rebased onto current `main` with the wizard
bundle rebuilt last; and every CI-relevant gate passes locally as of this
pass. The one remaining condition is the manual browser matrix (§7): it needs
real browsers, so it is a post-review step, but note the popup path is **not**
feature-flagged — `ConnectCard` passes `launch="popup"` unconditionally, so it
goes live for chat connect cards on the first production deploy containing
this branch. Run the matrix against a staging/preview deploy before that
production deploy; it is not a merge blocker.
