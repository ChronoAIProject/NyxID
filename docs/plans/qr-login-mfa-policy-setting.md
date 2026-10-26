# Configurable MFA for QR sign-ins

Status: implemented and validated in
[PR #2047](https://github.com/ChronoAIProject/NyxID/pull/2047). On 2026-10-27,
Opus gave final approval at `db13dbdc35148ae2be3fcb1d2ef72376c2b74916`:
**"FINAL APPROVED: no open findings."** The saved setting defaults to on, as
required by the user's clarification on 2026-10-26.

## Investigation evidence

The deployed frontend's `build-version.json` and backend `/health` both reported
commit `2ab239cecbda` (backend v0.73.1), which contains merged #2042. This was an
implementation gap: the setting was a static label and approved sign-in
derivation still required a verified parent. The actual `/login` MFA screen
also lacked a way to open its existing QR sign-in panel.

The installed mobile client's approval screens submit the request before
handling a server `2003 mfa_step_up_required` error. The saved server policy can
therefore permit approval without a mobile protocol change. This source check
does not substitute for installed-phone acceptance after deployment.

## Corrected requirement

QR/app-approved sign-ins skip NyxID MFA by default unless the account holder
explicitly turns off a real security toggle. This includes an already authenticated
legacy or mobile-exempt phone session without entering an authenticator code.
The static **Always enabled** status introduced in #2042 does not implement this
preference. The user's clarification on 2026-10-26 requires a saved default-on
toggle and supersedes the earlier default-off interpretation.

Scanning identifies a request. The signed-in person must still explicitly approve
the request, and public codes alone never authorize a sign-in. Password/social
sign-in, TOTP enrollment, recovery codes, and unrelated credential issuance keep
their existing checks.

## Agreed behavior

- Store a per-person `skip_mfa_for_approved_sign_ins` preference, defaulting to true when
  absent at any level of the profile. Show a working switch in Settings -> Security, including the shared
  assistant account panel. An older backend must not appear to support a switch
  it cannot persist. QR scans and typed request codes use the same approval protocol.
- Turning it back on after an explicit off requires the account holder's verified session when MFA is enrolled. A no-op true save needs no step-up.
  Disabling needs no step-up, and accounts without MFA can save either value.
  Sid-less bearers cannot use the exception; accounts without MFA never receive
  an exemption marker. Use the existing current-session step-up UI when needed. API keys, delegated
  credentials, OAuth clients, organizations and service accounts cannot change it.
- With the setting enabled, completed first-party human sessions can approve QR
  requests without an extra MFA prompt. Preserve real MFA evidence separately
  from an explicit QR-policy exemption; do not mark bypassed requests or sessions
  as having verified TOTP.
- With the setting disabled, keep the current requirement for a verified parent
  on an MFA-enabled account. A verified app approval still does not require a
  duplicate code in the receiving browser.
- The exception covers legacy browser QR login, v2 device account/Agent Key
  grants, and app-based identity verification for request-only or persistent
  browser approval. QR-derived Agent Key browser handoffs must preserve evidence
  through their bound one-time login code. Ordinary login-code minting is outside
  this exception. The dedicated Agent Key login protocol is included too.
- Check live policy at approval and at final credential delivery. Disabling the
  setting blocks outstanding bypass-dependent grants/proofs from completing.
  Completed sessions retain their fixed expiry and may be revoked normally.
  Disabling MFA preserves the choice and rotates its revision.
  Rotate a policy revision on changes so disabling/re-enabling cannot revive stale proofs.
- Make the policy check and final write conflict with a concurrent setting
  change. A read-only snapshot of the preference is not a sufficient fence.
- Log preference changes and use of the exception as metadata-only audit events.
  Keep installed mobile QR/manual-code formats and public API compatibility.

## Verification

- Mount the setting endpoint and prove absent defaults on, explicit off/on persistence, human-only
  authority, MFA confirmation only when re-enabling for an enrolled account, and metadata-only audit.
- Run the real device approval/delivery flow for an MFA account with an unassured
  legacy/mobile-exempt parent: off refuses; on succeeds; issued authority records
  a QR exemption and can authenticate/refresh without becoming MFA-verified.
- Cover legacy browser polling, v2 account and Agent Key grants, request-only and
  persistent app identity, and QR-bound browser handoff redemption.
- Disable between approval and delivery, proof verification and final decision,
  and handoff issuance and redemption; the exception must not survive the change.
- Preserve rejection for anonymous/API-key approval, expired or replayed codes,
  cross-account binding, unverified password identity, and unrelated login-code
  issuance. Existing verified-MFA flows must continue to pass.
- Settings tests must exercise the switch with MFA on/off, loading/failure,
  account replacement during a save, and both full-page and assistant panels.
- Retain the real `/login` MFA-to-QR tests from the current worktree. Compile and
  run backend tests against an isolated MongoDB replica set; run frontend tests,
  TypeScript, lint and the production build. Record actual limits and results.

## Validation results

All runtime results below are from the approved source commit
`db13dbdc35148ae2be3fcb1d2ef72376c2b74916`. No source changed after these runs.

| Backend suite | Passed |
| --- | ---: |
| `handlers::mfa::enforcement_tests::approved_sign_in_policy` | 14 |
| Remaining `handlers::mfa::enforcement_tests` | 14 |
| `handlers::login_approval::tests` | 13 |
| `services::auth_device_service` | 47 |
| `services::auth_agent_key_login_service` | 28 |
| `services::login_code_service` | 10 |
| `handlers::users::tests` | 24 |
| `models::session` | 4 |
| `services::token_service::tests` | 15 |
| `mw::auth::tests` | 121 |
| **Total** | **290** |

All passed with zero failures or ignored tests against an isolated MongoDB 8
replica set. The 276 surrounding tests exclude the policy module to avoid double
counting. The final Rust build emitted no warnings. Local libtest runs used two
threads: the initial four-thread surrounding MFA run exceeded the existing
two-slot recovery-code hashing admission limit in two enrollment fixtures. The
two-thread rerun passed without a source change. CI runs tests in separate
nextest processes and passed too.

Frontend validation passed all 125 tests in seven files: Settings, LoginPage,
QR session integration, auth hooks, auth-device hooks, MFA step-up, and MFA verify
form. TypeScript, focused ESLint, the production frontend build and footprint
checks, `cargo fmt --all --check`, the backend boundary script, and
`git diff --check` passed.

[CI for the approved source](https://github.com/ChronoAIProject/NyxID/actions/runs/40353270240)
completed with 35 successful checks, 14 skipped, one neutral, and none failed or
pending. This includes the complete backend suite, all ordinary and coverage
shards, backend coverage verification, Clippy, Rust feature combinations,
billing smoke, frontend validation, and CodeQL. Opus checked the commit, policy
log, all nine surrounding-suite records, and CI results before final acceptance.

Installed-phone acceptance remains pending after deployment. Browser visual
inspection was unavailable because the browser runtime rejected its
`node:process` import; no screenshot or physical-device result is claimed.
Replacement-code behavior is covered by execution, while replacement precisely
between handoff lookup and claim was approved through source review without a
test forcing that interleaving.

## Deployment acceptance

This document does not authorize deployment or production preference changes.
After publication/deployment, inspect the frontend build metadata and backend
health commit, then exercise an MFA-enabled account with the setting both off
and on using the installed phone app. The visible acceptance result is a saved
switch and a QR sign-in that completes without a phone or browser MFA prompt
by default, and the same approval requires MFA after the owner turns the toggle off.
Use a coordinated traffic cutover to a fully upgraded backend fleet: old replicas
cannot deserialize the new exemption, and default-on approvals write it immediately.
