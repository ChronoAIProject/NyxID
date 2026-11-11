# QR login MFA and password settings

Original plan status: APPROVED by Opus 5.5 through the connected OpenRouter service
(`anthropic/claude-opus-5.5`, generation `gen-1792986323-K5kLDBKixDtam8UuxcYh`).
All six initial review findings were addressed before approval.
The original-base implementation was accepted after the primary agent's personal
code and evidence review. Primary review findings R1–R5, the R1 lifecycle
follow-through, and R1b are resolved. Final validation evidence is recorded below.
Integration with newer main has its own review and validation record below.
The user authorized committing, pushing and opening a pull request after
completion. Publication also includes checking integration with the current
`main` branch and resolving any conflicts before opening the pull request.

## User outcome

An account holder who approves a QR sign-in from their already authenticated
NyxID phone app completes sign-in without entering another TOTP MFA code. Security
settings automatically explain this behavior. Accounts without a stored password
do not display Change Password. Accounts with a stored password have a working
password-change action, including accounts linked to a social provider.

## Original-base starting point

- Ordinary browser QR login uses the legacy account-only device request and
  poll-web routes, preserving installed mobile client compatibility. See
  `docs/ADR-015-auth-device-login.md` and `docs/DEVICE_LOGIN_PROTOCOL.md`.
- `auth_device_service::approve` creates the account grant after authenticated
  first-party human approval. The public user code and QR payload alone do not
  authorize login. Request-only app identity verification uses
  `login_approval_service::verify_app`, which satisfies the MFA gate already.
- Password-based login checks `User.mfa_enabled`. This account-level flag must
  never be cleared as a side effect of QR login.
- Settings currently infer password availability from `social_provider`, even
  though social accounts may acquire a password and passwordless accounts may
  have no social-provider marker.
- The draft adds `has_password` to `GET /users/me`, derived solely from
  `password_hash.is_some()`, and hides the entire password card unless true.
- The draft exposes QR verification under Two-Factor Authentication as an
  automatically enabled status, not a toggle that claims to change an unsupported
  server policy. This matches existing behavior for every account. The user was
  offered a configurable switch as an alternative but has not requested one.
- The existing password form posts to `/auth/password/change`, which has no
  backend route. Completing this action is included to avoid leaving a broken
  form for users who do have a password.

## Original approved implementation contract

1. Audit the legacy browser QR path, v2 account grants, and request-only/persistent
   app identity verification. Preserve installed-client request/response formats,
   explicit approval, private polling, single delivery, expiry, account identity
   binding, and the requirement for a first-party human approver. Do not infer
   mobile trust from a user-agent string or client-controlled flag. Preserve
   TOTP checks for password verification. Fix any actual redundant prompt found;
   otherwise retain the existing server behavior and prove it with tests.
   Audit frontend completion and navigation as well: an MFA-enabled profile
   must not cause a completed QR session to open a TOTP challenge. Correct
   stale pending MFA state after the server confirms a restored session:
   `checkAuth` currently leaves `mfaRequired` and `mfaToken` untouched on success,
   while QR polling uses that method. Clear those pending values only after
   a successful authenticated profile response, and test the pending-password
   MFA -> successful QR session transition using the actual store.
   Correct
   `login_request_identity_verified.mfa_verified` to reflect the verified row's
   actual MFA-satisfied state for app approval as well as password + MFA;
   preserve the separate `method` value so the evidence remains distinguishable.
2. Keep the automatic QR verification status under Two-Factor Authentication,
   with copy distinguishing approval from merely scanning a QR. It applies
   whether TOTP enrollment is on or off. Do not automatically enroll TOTP or
   persist an unrelated MFA preference.
3. Expose the server-derived `has_password` boolean in the dedicated profile
   response and TypeScript user contract. Do not expose the hash, raw password,
   or model struct. Hide the entire Change Password card when false or unknown.
   Permit social-linked accounts with a password. Cover both the settings page
   and the assistant's settings panel. Preserve accessible form labels.
4. Implement the existing `POST /api/v1/auth/password/change` form contract with
   a dedicated handler and auth service operation following handlers -> services
   -> models. Only a first-party human account session may invoke it. Validate
   current and new password input using existing password length/policy rules;
   new passwords use the existing server authority of 8..128 UTF-8 bytes.
   The change-password frontend must count bytes with TextEncoder and use
   accurate byte-limit messages; remove frontend-only uppercase/lowercase/digit
   requirements so change-password policy matches the server.
   Current-password input is nonempty and at most 128 bytes, with no new minimum
   that would prevent verification of a historical credential.
   verify the current password and require an existing credential, so this cannot
   create a password on a passwordless account. Use the existing password hash
   implementation. Atomically replace the hash only if the verified old hash is
   still current, filtering on `_id`, the verified old `password_hash`,
   `user_type: person`, and `is_active: true`. Treat matched_count != 1 as a
   defined 409 conflict. Return a generic 400 validation error for an incorrect
   current password, avoiding the frontend's 401 session-expiry behavior.
   Throttle before password verification using shared Mongo-backed per-IP and
   per-user limiters (20 attempts/IP/minute, 5 attempts/user/minute), with a
   dedicated password-change namespace; attempts include successes and counters
   are not reset on success. Reuse existing limiter constructors, resolving IP
   through the trusted-proxy helper and keying user limits by account ID.
   Use the existing 429 error shape; verify the frontend displays its message
   without clearing the session. Allow a new password equal to the current one,
   consistent with reset-password behavior; document and test that decision.
   Clear pending password-reset tokens and their expiry in the same update.
   Fence reset-password's final update against its still-current, unexpired
   token so a reset that read an old token cannot overwrite a successful change.
   The reset update matches the hashed token and expiry > current time and
   rejects matched_count != 1; it retains person/active-account restrictions.
   Keep reset validation/error messages consistent with its existing byte rule.
   Prevent passwords and hashes from entering Debug output, logs, audit events,
   telemetry, or responses. Use secret wrappers consistent with the project.
   Emit a metadata-only audit event after a successful update. Session retention
   follows the documented password-change behavior: existing sessions remain
   valid. Do not weaken the password-reset flow or grant this operation to API
   keys, delegated/OAuth clients, service accounts, or organization identities.
5. Update account-security and API documentation to describe the final behavior
   accurately. Remove the stale comment that calls the password route unwritten.
   Keep the diff focused and correct all review findings in this scope.

## Verification and acceptance

- Settings tests: password missing, password present, social-linked with and
  without a password, and a profile from an older backend omitting the new field.
  The card is absent in the first/unknown cases and usable in password cases.
- Profile tests: the boolean follows stored credential presence independent of
  social provider, and serialized responses never contain `password_hash`.
- QR runtime tests with MFA enabled: legacy browser approval delivers a usable
  browser session without a second MFA challenge; request-only and persistent
  app identity flows complete without a redundant challenge. Password identity
  verification still requires a valid MFA code when configured.
  Frontend coverage must prove successful QR completion navigates to the
  authenticated destination without rendering a TOTP challenge for an
  MFA-enabled account. Assert app-proof audit metadata records satisfied MFA
  while retaining the app method.
- Preserve or extend negative coverage for unauthenticated/API-key approval,
  expiry, denial, wrong request/private poll secret, replay/single delivery, and
  cross-account binding. Reuse existing tests rather than duplicate the protocol
  harness unnecessarily.
- Password-change runtime tests: correct/wrong current password, passwordless
  account, social-linked password account, invalid/oversized new password,
  unsupported credential types and org identities, concurrent stale replacement,
  reset-token invalidation and a reset racing with a password change,
  successful subsequent password verification, and metadata-only audit. Verify
  the mounted route through real router requests, not only the service helper.
  Assert wrong-current-password is non-401 and preserves the frontend session;
  repeated attempts hit both shared limiter boundaries before hash verification.
  Cover multibyte 127/129-byte password boundaries and a seven-character input
  containing at least eight UTF-8 bytes in frontend and backend validation.
- Run relevant frontend tests, TypeScript, focused lint, Rust formatting,
  backend compilation, and the relevant backend runtime tests against an isolated
  local MongoDB replica set. No external deployment or production database work.
- Required frontend and backend runtime acceptance is complete; the final
  commands and reconciled results appear under Final validation below.
- The primary agent reviews the complete implementation personally, reports all
  concrete findings to the GPT-6.1-Sol implementation session, and repeats until
  every accepted finding is resolved and required checks pass. Neither reviewer
  nor implementer may delegate or spawn further agents. Do not claim theoretical
  absence of all risk; require evidence for each concrete acceptance condition.

## Original review sequence

1. Opus 5.5 reviews this plan and the relevant source. It returns APPROVE or
   REVISE with concrete findings, including any conflict between user intent and
   the automatic-status contract. Resolve its findings before implementation.
2. A single heca GPT-6.1-Sol session at xhigh, full-access implements the accepted
   plan in this existing worktree, adopting or correcting the draft changes.
3. The primary agent reviews code and evidence directly and iterates with that
   same implementation session. No extra code-review agent, Codex subagent,
   deployment, merge, or external messages are authorized by this workflow.

## Original-base implementation review and evidence

The implementation adopts the draft and adds the mounted password-change route,
shared Mongo limits, byte policy, hash CAS, reset final-write fence, and QR store
and audit corrections. No protocol payloads or TOTP enrollment preferences change.

Personal review found that the existing cookie extractor also accepted temporary
`mfa_pending` session rows. Those proofs now fail ordinary account authentication;
mounted-route regressions cover profile reads, password changes and QR approval.
Password-MFA completion still reads its pending proof through its dedicated flow.
Review also added active-person fences in both final writes, live app-proof audit
metadata, cross-account app-proof rejection, and UTF-8 boundary tests.

Primary review findings R1–R5 are implemented. QR completion now requires an
explicit fresh-profile success, with a scoped request-generation/unmount guard
covering API errors, profile application, identity transitions and MFA clearing.
Profile failures expose a retry using the delivered cookie without polling the
single-delivery grant again. Real-store tests cover 401/503 with a different old
account and pending MFA, retry, restart/unmount with late 200/401/503, and
cancellation while the response body is loading. A hook test verifies that
restoration remains retryable beyond the former QR expiry.

R1b also clears the prior authenticated identity at the current scoped QR handoff,
before the profile request. Both current 401 and 503 failures leave `user: null`
and `isAuthenticated: false`, preserving pending MFA until a fresh successful
profile. Shared identity cleanup resets telemetry, assistant identity, service
views, context, drafts and wire logs. Scoped restoration leaves global
`isLoading` untouched throughout; the QR hook owns its loading/error UI. Ordinary
and ephemeral profile refresh behavior is preserved, including their existing
failure semantics. Tests assert identity clearance while the profile is pending,
retry installs the approved account, and cancelled/late attempts preserve newer
identity and MFA state. The account-security guide now matches the actual
Two-Factor Authentication switch and password-confirmed disable dialog.

Emoji-containing 127/129-byte boundaries are covered on both sides. Organization
and inactive-account mounted tests assert exact statuses and numeric errors.
The mounted password race permits only a defined 400/409 loser and verifies the
winner's stored password; the snapshot-level CAS race deterministically proves
409. The main password-MFA regression enrolls a real TOTP factor, denies pending
proof cookies, completes mounted `/auth/mfa/verify`, then uses its returned
browser cookie for profile access and real QR approval.

## Original-base validation (8bdc90115)

Original-base frontend snapshot: **190 tests passed in nine files, zero failures and no
skipped tests**. From `frontend/`:

```sh
npm test -- \
  src/pages/settings.test.tsx \
  src/schemas/auth.test.ts \
  src/stores/auth-store.test.ts \
  src/components/auth/qr-session.integration.test.tsx \
  src/components/auth/web-device-login.test.tsx \
  src/hooks/use-auth-device.test.tsx \
  src/pages/login-device.test.tsx \
  src/hooks/use-auth.test.tsx \
  src/components/auth/mfa-verify-form.test.tsx

npx tsc -b

npx eslint \
  src/pages/settings.tsx src/pages/settings.test.tsx \
  src/schemas/auth.ts src/schemas/auth.test.ts \
  src/stores/auth-store.ts src/stores/auth-store.test.ts \
  src/types/api.ts \
  src/hooks/use-auth-device.ts src/hooks/use-auth-device.test.tsx \
  src/components/auth/web-device-login.tsx \
  src/components/auth/web-device-login.test.tsx \
  src/components/auth/qr-session.integration.test.tsx \
  src/pages/login-device.test.tsx src/hooks/use-auth.test.tsx \
  src/components/auth/mfa-verify-form.test.tsx
```

Evidence: `/tmp/nyxid-qr-round3-frontend-tests-final.log`,
`/tmp/nyxid-qr-round3-typescript-final.log` and
`/tmp/nyxid-qr-round3-eslint-final.log`. TypeScript and focused ESLint both exited
0 on the final frontend files. Prettier checks passed for the edited auth store,
store tests and real-store QR integration tests. The test run prints Node's existing
`module.register()` deprecation warning. An earlier round-3 test run found that
the new wire-log fixture had not enabled capture; the fixture now positively
asserts stored data before exercising cleanup, and the final run passes.

Original-base backend runtime acceptance: **264 distinct selected tests passed**, with zero
skipped/ignored tests in every group, against the isolated local Mongo replica
set. These are reconciled results across the diagnostic run and the two affected
fixture regressions rerun after rebuilding:

| Test filter | Distinct passing tests |
|---|---:|
| `password_change` | 13 |
| `handlers::auth_device::tests` | 26 |
| `handlers::login_approval::tests` | 7 |
| `services::auth_device_service` | 47 |
| `services::auth_service::tests` | 25 |
| `handlers::users::tests` | 24 |
| `handlers::mfa::tests` | 6 |
| `mw::auth::tests` | 116 |
| **Total** | **264** |

The diagnostic run passed 262 of 264. Two newly extended fixtures lacked startup
system-role seeds; both fixtures were corrected. The only subsequent Rust edits
were those seeds and improved assertion diagnostics. The test executable then
rebuilt successfully in 10m 42s using:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  cargo test -p nyxid --bin nyxid-server --no-run
```

Build evidence: `/tmp/nyxid-qr-password-build-final-runtime.log`. Runtime commands
used the built executable with each table filter:

```sh
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:55554/?replicaSet=nyxidQrPasswordTest' \
  target/debug/deps/nyxid_server-a25f220fcfa166bf \
  '<filter>' --test-threads=1 --nocapture
```

Both exact affected tests passed on the rebuilt executable, selected by their
full names (one test per invocation):

- `handlers::auth_device::tests::staged_codes_keep_cli_and_browser_verification_urls_and_installed_account_approval`:
  1 passed in 21.61s.
- `handlers::users::tests::profile_reports_stored_password_independently_of_social_provider`:
  1 passed in 0.38s.

Evidence: `/tmp/nyxid-qr-fixture-regressions.log`. The original diagnostic record
`/tmp/nyxid-qr-runtime-results.json` retains the two superseded fixture failures;
its passing cases and the exact passing regressions above account for all 264
distinct tests. The individual diagnostic logs are recorded in that JSON.
The final repeat additionally passed all 13 password-change tests in 100.04s
(`/tmp/nyxid-qr-final-runtime-password_change.log`). The primary reviewer
deliberately stopped the remaining repeat after accepting backend runtime
evidence; this is **not** a completed 264-test rerun on the final executable.
Round 3 changes only frontend code/tests and documentation, so no additional
heavy Rust build or broad runtime rerun was started.

`cargo fmt --all -- --check` and `git diff --check` passed. The primary agent
verified the runtime logs and confirmed that the reviewed Rust files remained
unchanged during the final frontend round. No concrete implementation findings
or required checks remain unresolved. Compatibility evidence consists of local
protocol, browser and installed-client fixtures.

Implementation and all correction rounds used the same persistent GPT-6.1-Sol
thread (`01a17655-2a49-70c1-81aa-6555494e18ef`) in a heca-managed terminal, with
`model_reasoning_effort="xhigh"`, `approval_policy="never"` and
`danger-full-access`. The primary agent performed code review personally and
accepted the final result; no additional code-review agent was used.

The primary agent stopped the dedicated Mongo process (port 55554, PID 2617)
and removed its temporary database directory after verification, on
2026-10-26 at 05:02:49 UTC. Cleanup evidence is
`/tmp/nyxid-qr-password-mongo-cleanup.json`; the original process/database metadata
is `/tmp/nyxid-qr-password-mongo.json`. No repeat-runner child remained when
round 3 resumed. No commit or PR had been created at that acceptance point;
the user's subsequent request authorizes publication. No deployment was made.


## Integration with main d102570af

The original Opus approval and 190/264 results above describe the original base.
They do not approve or validate this integration. The primary retains personal
review and acceptance. The merge of main is being resolved in the existing
worktree; the primary handles the merge commit, push and PR.

Main enforces MFA on each new sign-in and distinguishes completed-session
acceptance from authority to approve a new device. Its default-off
`auth:mobile-mfa` rollout exemption remains intact. Unverified legacy or
mobile-exempt approvers must verify their session before approving a new sign-in.
After app approval, the receiving browser inherits verified authority and needs
no second challenge. Legacy account QR and selectable v2 protocols retain their
separate delivery paths, proof binding and single-use guarantees.

QR restoration clears the prior account, telemetry, account-local state and
account-bound analytics consent before requesting the delivered session profile.
Pending password MFA persists until a fresh successful authenticated profile.
Generation guards cover both response arrival and asynchronous body loading,
including before global API 401 mutation. Late/cancelled attempts preserve newer
identity, consent revision and MFA state. Profile retry never polls/redeems the
grant again. Scoped restoration owns its local loading UI and leaves global
`isLoading` unchanged; default, ephemeral and required-session callers retain
main's behavior, including required-session error propagation.

Voluntary password change uses a DB-only retryable transaction with the exact
verified old hash, active/person/not-deleting guards, reset-token cancellation
and `session_authority::revoke_pending`. Dedicated and legacy pending MFA proofs
and unfinished login approvals are invalidated while completed sessions and
refresh authority remain valid. Proof creation/completion fences the same user
document. Reset uses its live token/server-time expiry CAS and main's full
session-authority revocation in one transaction. Same-password replacement is
still allowed. Shared authenticated-purpose policy supersedes the earlier
marker-only cookie filter. Main's enrollment, disable/recovery, session/token
fences and social mobile rollout semantics remain in force.

The primary personally reviewed the integrated production changes, including
pending-proof revocation and reset/change races. The QR regression locates the
delivered browser session by its cookie token hash, independently of the source
phone session and the intermediate approval session.

### Defaults verified in the integrated code

- Settings displays **Skip MFA for NyxID app sign-ins — Always enabled** in
  Studio and the shared assistant account panel. A delivered, approved browser
  session does not request another factor after its profile is confirmed.
- `auth:mobile-mfa` defaults to false. Mobile social redirects and native social
  token exchange therefore bypass MFA unless a live user/organization override
  enables the flag. Email/password mobile login continues to require MFA for
  enrolled accounts.
- A phone session that used the mobile social exemption can authenticate, but
  needs verified authority before approving another device. The browser's
  automatic exemption does not remove this approval gate.

### Integrated validation

- Full frontend suite: 6,682 passed across 610 files. Focused authentication,
  QR, settings, consent and MFA suite: 257 passed across 15 files.
- TypeScript, full frontend lint, production build, Rust formatting and whitespace
  checks passed. The regenerated wizard bundle matches its 178 source files and
  five extra inputs; source hash `3630747ae7bd35215ed42627f88e7678970ef9c48a04b75cbf26afd1e97f4496`.
- Final backend validation uses [PR #2042 CI](https://github.com/ChronoAIProject/NyxID/actions/runs/40277035395)
  at code commit `2d663e9c5458e2620af46ce34615cec9ce444e79`. Its test archive built
  successfully. The inventory contains 10,437 tests, of which 10,424 match the
  ordinary run and 13 are pre-existing ignored tests. All password-change, QR
  MFA and mobile-default regressions match and are not ignored.
- The first integrated runtime run completed all 10,424 selected tests: 10,422
  passed and two failed. The approved-QR browser regression, mobile social
  default/override regressions, password byte boundaries, credential CAS and
  pending-proof revocation checks passed. The failures were stale expectations
  for the shared redacted Debug format and an OAuth fixture without the active
  client/consent required by main. The correction preserves valid third-party
  credential denial coverage rather than accepting an unauthenticated fixture
  as proof of that boundary.
- CodeQL reported 27 hardcoded cryptographic-value annotations in synthetic
  test credentials. Correction uses runtime-generated credentials, preserving
  byte boundaries, historic short-password verification and shared race inputs.
  No security queries or workflows are suppressed. The primary reviewed the
  correction: missing/unbound OAuth authority must return 401/1001; a client
  with live protected consent must authenticate to the mounted profile route
  before password change returns 403/1002. Debug assertions follow main's
  shared redaction contract and exclude all generated credentials.
- Final acceptance requires the corrected revision's runtime and security
  checks to pass. Its run and final results are tracked in
  [PR #2042](https://github.com/ChronoAIProject/NyxID/pull/2042)'s Test Plan.

The duplicate local backend build was cancelled after exhausting available RAM;
it produced no accepted runtime evidence. The primary stopped only its owned
worker/build processes and dedicated MongoDB (PID 56050, port 61275), then removed
that database directory. Local cancellation and cleanup records are
`/tmp/nyxid-qr-integration3-local-cancelled.json` and
`/tmp/nyxid-qr-integration3-mongo-cleanup.json`. Final backend acceptance is based
on the CI results above, which compile and execute the published code revision.
