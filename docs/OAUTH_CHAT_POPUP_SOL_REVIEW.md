# Adversarial Review: OAuth Chat Popup Implementation Plan

Reviewed against `a1aa10eb`, the upstream v3 design, the prior 24 findings,
and the current source tree. Corrections C1, C2, C3, C4, and C8 are factually
sound: the connect card does not currently mount `ManageConnectionModal`;
nginx proxies `/oauth/launching` but not bare `/oauth`; Vite's proposed
`^/oauth/` key preserves the NyxID IdP subpaths and does not match
`/oauth-consent`; a client launch id is appropriate for the synchronously
opened window name; and `use-keys.ts` is in the wizard manifest while the
other planned modified frontend files are not. The exact wizard freshness
test is green at this commit.

1. **[P1] C6 / F4-F5 omit the application's real public-route gate.** Adding
   direct children of `rootRoute` is insufficient. `frontend/src/main.tsx`
   independently classifies public paths in `isPublicPath`; neither `/oauth`
   nor `/oauth-launching` is listed. An unauthenticated completion first renders
   nothing while `checkAuth()` runs and then navigates to `/login`, so it never
   broadcasts or shows completion copy. The launching page is likewise held
   behind auth bootstrap, contradicting the claimed unauthenticated, minimal
   interstitial and worsening the opener-severance race below. The task list
   must update and test `isPublicPath`, not only `router.tsx`.

2. **[P1] C7 / F4 / F8 do not guarantee opener severance before external
   navigation.** `window.opener = null` is deferred to a React effect after the
   popup has loaded the full SPA (and, under the plan as written, after auth
   bootstrap), while the parent independently awaits `ensureKey()` and
   initiate and then assigns the provider URL. There is no ordering edge
   between those contexts. A cached/fast initiate can navigate the popup to a
   malicious configured provider before the effect runs; that provider still
   has `window.opener` and can reverse-tabnab the chat tab. The interstitial
   needs an explicit ready/opener-cleared handshake, and the parent must refuse
   external navigation before that acknowledgement. Test 19 proves only that
   `window.open` precedes the initiate promise; it does not test this security
   invariant.

3. **[P1] C7 / F1 / F5 make every real retry fail URL validation.** The
   `oauth_retry.url` is the fresh `authorization_url` returned by initiate,
   which is normally cross-origin (`https://github.com/...`,
   `https://accounts.google.com/...`, etc.). F1 requires the retry guard to
   accept only relative or same-origin absolute URLs, and test 14 explicitly
   requires cross-origin URLs to be ignored. Therefore "Try again" rejects the
   valid URL for every external provider. The reply must be bound to the
   pending retry request and validate an allowed HTTP(S) provider URL from the
   authenticated initiate response, not apply a same-origin rule that is
   incompatible with OAuth.

4. **[P1] Section 7's claim that D-2 is not load-bearing for retry is false.**
   Attempt A's denial leaves its state unconsumed and marks the placeholder
   `failed`. Retry B creates fresh state and resets that same key to
   `pending_auth`. Replaying A's still-live `error=access_denied&state=A`
   now marks B's placeholder `failed`; when B succeeds,
   `write_oauth_tokens_to_key` refuses to update `failed` rows
   (`user_api_key_service.rs:493-515`), so the fresh authorization also fails.
   This is exactly a concrete failure of the new denial -> popup -> retry path,
   not merely unchanged legacy blast radius. D-2, or an attempt-generation
   check that prevents old states mutating the retried placeholder, is required
   before shipping retry.

5. **[P1] B2/B3 violate the core non-`cc` BSON byte-identity claim.** The
   proposed fields have `#[serde(default)]` only. `Option::None` therefore
   serializes as BSON `null`, so every newly initiated provider, SA, wizard,
   and device-code state gains `flow_kind: null` and `attempt_nonce: null`.
   Deserializing and reserializing an old document also changes its shape.
   `bson_backward_compat_missing_new_fields` proves only that old documents can
   be read; it does not assert that absent fields stay absent on serialization.
   Both optional fields need `skip_serializing_if = "Option::is_none"`, plus
   an exact legacy BSON-key test, if byte identity is truly required.

6. **[P1] F1/F5/F6 mistake nonce correlation for authentication against
   same-origin broadcasters.** Every context subscribed to the global
   `nyxid.oauth` channel receives the completion page's `oauth_result`,
   including its nonce. A same-origin page or worker can learn that nonce and
   immediately forge `retry`, `cancel`, or `view_result`; the opener then
   performs the mutation because nonce equality is its only check. It can also
   forge an ack and make the popup close before the intended action is handled.
   The nonce prevents accidental cross-attempt handling and blind guessing,
   but it does not support Risk 5's claim that it stops any same-origin surface
   from faking completion actions. Action authorization needs a protocol that
   does not publish its sole capability to every channel listener (for example,
   a per-attempt channel secret/name kept out of the broadcast payload), while
   result messages must remain wakeups followed by server verification.

7. **[P1] B6 changes a shared legacy error path despite claiming redirect-only
   gating.** Today's missing-code branch returns the legacy redirect before any
   MongoDB access. The plan adds `peek_oauth_state` whenever a state parameter
   is present so it can discover `cc`. A malformed callback for any existing
   non-`cc` flow now waits on MongoDB and can hang until database timeout during
   an outage instead of returning immediately. The eventual Location bytes may
   match, but the shared path is no longer behavior-identical. The test list
   checks only Location strings and cannot catch this regression. The plan must
   either relax the byte-identity claim explicitly or supply an authenticated
   callback discriminator that can be classified without a new legacy lookup.

8. **[P2] F4/F5 do not implement the upstream privacy contract.** Both pages
   boot through `main.tsx`, which runs `checkAuth`, fetches public config, can
   initialize PostHog, and always mounts `ConsentBanner`. Bare `/oauth` is not
   in `SENSITIVE_PATH_PATTERNS`, and the plan changes neither telemetry nor
   `frontend/nginx.conf.template`. Production therefore retains
   `Referrer-Policy: strict-origin-when-cross-origin` and has no explicit
   `Cache-Control: no-store`; the initial `/oauth?...nonce=...` request is
   already in nginx/CDN logs before `history.replaceState`, and same-origin
   bootstrap assets may receive the original URL as Referer before React runs.
   Scrubbing history is useful but cannot justify the claims "no telemetry",
   "no-referrer", or complete log/referrer hygiene.

9. **[P2] F3/F8 release the single-flow lock while a retryable popup is still
   live.** The plan ends the lock when polling reaches any terminal status and
   when the OAuth step unmounts. A denial is terminal in the key row but leaves
   an error popup offering "Try again"; another card can start an attempt after
   the lock is released, then the first popup's retry starts a second attempt in
   parallel. Closing the dialog during `ensureKey()`/initiate similarly ends
   the lock while the async handler can later navigate its orphan popup. Keep
   ownership until the popup explicitly completes/cancels or the initiate task
   is aborted, and generation-key every async continuation.

10. **[P2] F5's no-ack fallback infers that the original tab is gone from a
    signal that does not prove it.** `BroadcastChannel` may be unavailable,
    message delivery may fail, or the dialog may merely have been closed and
    unmounted its receiver while the chat tab remains alive. In all three
    cases, "View connection" gets no ack and navigates the popup to `/keys`,
    creating the duplicate NyxID app window that the design says must never be
    created while chat still exists. A no-ack path needs a neutral manual-return
    UI or a separately verifiable opener-liveness mechanism, not destination
    navigation after 400 ms.

11. **[P2] F9 opens two Radix dialogs instead of replacing the connect dialog.**
    `ConnectCard` keeps `dialogOpen=true` while the plan's
    `onPopupViewResult` only sets local state to mount
    `ManageConnectionModal`. The still-open `AddKeyDialog` and the new always-open
    management dialog then overlap with competing overlays and focus traps.
    The callback must close the add dialog before mounting the management
    modal, and test 25 must assert one modal/focus scope remains rather than
    only asserting that navigation was not called.

12. **[P2] F5/F6 expose a Cancel CTA without a server-side cancellation
    transaction.** `postAction(cancel)` only invokes an unspecified frontend
    cleanup callback; no task consumes the OAuth state or restores/replaces a
    reconnect placeholder. A cancelled denial state remains replayable until
    TTL, and reconnect cleanup is deliberately protected from deletion, so the
    key can remain failed/pending with a live stale state. The upstream design
    required consume-state plus placeholder reset. If that is out of pilot
    scope, the popup cannot promise that Cancel cancels the attempt.

13. **[P2] The stated CI gate contains concrete TypeScript and coverage gaps.**
    F6 uses TanStack Query v4-style `invalidateQueries(["keys"])`; this repo is
    on v5 and consistently requires
    `void queryClient.invalidateQueries({ queryKey: [...] })`, so following the
    plan literally fails `tsc -b` (and omitting `void` is inconsistent with the
    linted codebase). Mixed-version handling also leaves
    `response.attempt_nonce` optional while the store/retry contract requires a
    `string`; new-FE/old-BE can return `undefined`, and no guard or fallback is
    specified. Finally, the final gate runs `cargo test -p nyxid`, not the CI
    command `cargo nextest run -p nyxid`, so it does not reproduce the requested
    backend gate.

14. **[P2] Several listed tests are vacuous or encode the defect, while major
    failure modes have no test.** Rust test 9 explicitly permits replacing the
    successful callback test with a redirect-builder unit test plus a failure
    branch, which does not prove success dispatch uses the builder. Rust test
    19 is merely "existing tests keep passing". The Rust/TS token "parity"
    tests each compare a constant to a duplicate literal in the same language,
    so both can pass after cross-language drift. Rust test 13 cannot prove how
    a popup caller behaves once state is unattributable. Frontend tests 5 and
    14 positively assert the cross-origin-retry rejection that breaks OAuth.
    There is no test for `main.tsx` public classification, the interstitial
    ready handshake, stale-state mutation after retry, absence of legacy BSON
    keys, exact legacy audit payloads/authorization URL, same-origin channel
    eavesdropping, unmount during initiate, terminal-lock ownership, overlapping
    dialogs, no-ack with a live opener, cancellation semantics, telemetry/header
    policy, nginx/Vite routing, or a real browser with COOP/mobile close
    behavior.

## MISSING

- Add `/oauth` and `/oauth-launching` to the application-level public-path
  policy and give completion pages an intentionally minimal boot path.
- Add an interstitial-ready/opener-cleared handshake before any external
  `popup.location` assignment.
- Define retry URL validation that permits the exact authenticated HTTP(S)
  provider URL while rejecting forged channel navigation.
- Make attempt generations authoritative so stale callbacks cannot mutate a
  retried placeholder; consume terminal states and implement real cancel.
- Preserve absent optional BSON fields and pin exact legacy BSON, redirect,
  response, authorization-URL, and audit shapes.
- Separate correlation from action authorization on BroadcastChannel.
- Specify no-store/no-referrer/telemetry suppression at the HTTP/bootstrap
  layer, not after the sensitive URL has already loaded.
- Add browser integration coverage for opener severance, COOP, popup-as-tab,
  refused close, and retry; mocked jsdom APIs cannot validate those properties.

**Verdict: not safe to implement as written; seven P1 findings block the chat-only pilot and disprove the claimed legacy byte identity and retry/security invariants.**
