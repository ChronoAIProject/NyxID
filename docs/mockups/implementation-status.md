# Device login implementation status

Updated 2026-09-15 on `nyxid-ios-login-investigation`, integrated with main
`610da777` (0.20.0). Implementation is local; no deployment or physical-device
validation is claimed. See [the protocol](../DEVICE_LOGIN_PROTOCOL.md) for the
implemented contract and [ADR-015](../ADR-015-auth-device-login.md) for rollout.

| Requirement | Current implementation |
|---|---|
| Public code compatibility | Legacy eight-character codes; v2 eight-character issuance behind default-off `AUTH_DEVICE_EIGHT_CHAR_CODES`. Both old marker and eight-character readers retained. Ambiguous lookup fails closed; reservations survive uncertain insertion. |
| Approval UI | Real React `DeviceApproval` uses three step cards, explicit request acknowledgement, capability-gated account/restricted choice and final existing/new/account approval. `/login/agent-key` stays restricted. |
| Permission picker | Standalone search, separate grouped selected panel, counts, mixed group selection, keyboard dismissal and focus retention. Runtime inventory comes from options/catalog APIs; no mockup account data ships in production. |
| URL hints and sign-in | Bounded allowlisted parser, visible invalid/unknown hints, password/MFA/social return preservation, explicit-click tab-local long-hint resumption. Sign-in does not approve. |
| Effective grants | Server resolves per-key overrides and platform credentials; current/future auto-connected platform access is disclosed, owner-bound and never exact. Approval uses additive consent snapshots. Unchanged unavailable extras remain eligible. |
| Normal `/login` | Legacy full-account browser login. Scoped browser sessions and mobile handoff are outside this implementation. |
| Sharing | Protocol, ADR/API and NyxID skill reference updated. The existing private Ornn skill `nyxid-device-login-protocol` is updated to version 1.1; registry validation and readback passed. |

Verification before main integration: primary reported 3,129 frontend tests,
48 mobile tests, CLI suites, unchanged installed-scanner byte identity, and eight
real React browser tests (including short/long social return and MFA) passing.
These results apply to those tested revisions. Integrated platform-grant changes
and newest backend tests are undergoing targeted verification; broad final checks
remain with the primary. Do not interpret the HTML preview or parser fixture as
physical-iPhone validation.
