# Configurable MFA for QR sign-ins

Status: design approved by Opus QR MFA policy reviewer on 2026-10-26; implementation in progress.

## Corrected requirement

The account holder can enable a real security setting to skip NyxID MFA for
QR/app-approved sign-ins. This includes approving from an already authenticated
legacy or mobile-exempt phone session without entering an authenticator code.
The static **Always enabled** status introduced in #2042 does not implement this
preference. The earlier plan's verified-parent-only interpretation is superseded
by the user's clarification on 2026-10-26.

Scanning identifies a request. The signed-in person must still explicitly approve
the request, and public codes alone never authorize a sign-in. Password/social
sign-in, TOTP enrollment, recovery codes, and unrelated credential issuance keep
their existing checks.

## Agreed behavior

- Store a per-person `skip_mfa_for_approved_sign_ins` preference, defaulting to false when
  absent. Show a working switch in Settings -> Security, including the shared
  assistant account panel. An older backend must not appear to support a switch
  it cannot persist. QR scans and typed request codes use the same approval protocol.
- Enabling the exception requires enrolled MFA and the account holder's verified session. This is a one-time setting change, not a check on every QR sign-in.
  Disabling needs no step-up. Sid-less bearers cannot use the exception; accounts
  without MFA cannot enable it and never receive an exemption marker. Use the existing current-session step-up UI when needed. API keys, delegated
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
  Disabling MFA clears the setting and rotates its revision.
  Rotate a policy revision on changes so disabling/re-enabling cannot revive stale proofs.
- Make the policy check and final write conflict with a concurrent setting
  change. A read-only snapshot of the preference is not a sufficient fence.
- Log preference changes and use of the exception as metadata-only audit events.
  Keep installed mobile QR/manual-code formats and public API compatibility.

## Verification

- Mount the setting endpoint and prove absent/off/on persistence, human-only
  authority, MFA confirmation for an enrolled account, and metadata-only audit.
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

## Deployment acceptance

This document does not authorize deployment or production preference changes.
After publication/deployment, inspect the frontend build metadata and backend
health commit, then exercise an MFA-enabled account with the setting both off
and on using the installed phone app. The visible acceptance result is a saved
switch and a QR sign-in that completes without a phone or browser MFA prompt
when the exception is enabled.
