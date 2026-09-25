# Device login implementation status

Updated 2026-09-25 on `fix/device-login-approval`, integrated with main
`ff089247` (0.30.2). This work supersedes closed, unmerged PR #1569. No deployment
or physical-device validation is claimed. See [the protocol](../DEVICE_LOGIN_PROTOCOL.md)
for the implemented contract and [ADR-015](../ADR-015-auth-device-login.md) for rollout.

| Requirement | Current implementation |
|---|---|
| Public code compatibility | Legacy eight-character codes; v2 eight-character issuance behind default-off `AUTH_DEVICE_EIGHT_CHAR_CODES`. Both old marker and eight-character readers retained. Ambiguous lookup fails closed; reservations survive uncertain insertion. |
| Approval UI | Three steps: review the request, choose account or restricted access, then select an existing Agent Key or create one if needed. The reviewed stepper, action placement, expandable details and permission dropdown are implemented in React. `/login/agent-key` stays restricted. |
| Permission picker | One component searches NyxID scopes, service permissions and connections, with separate selected pills, counts and service icons. Account-only keys need no service selection. Existing and new keys share a compact final review with expandable permission, service and extra-access details. Runtime inventory comes from options/catalog APIs; mockup account data is not used in the application. |
| Identity verification | Existing browser sessions skip sign-in. Otherwise a browser-bound proof verifies identity only for the pending request, expires within ten minutes and closes on completion. Password/MFA and configured Google, GitHub and Apple providers are supported. Staying signed in is an explicit choice; verifying identity never approves the requester automatically. |
| URL hints and CLI | Bounded allowlisted hints prefill login type, permissions and new-key settings. CLI flags construct the approval URL; `catalog --public` discovers metadata without login, and `mcp discover` lists operations visible to the current credential. Short/long social return hints are restored within the same tab. |
| Effective grants | Server resolves per-key overrides and platform credentials; current/future auto-connected platform access is disclosed, owner-bound and never exact. Approval uses consent snapshots and revalidates live permissions. |
| Normal `/login` | Legacy full-account browser login. Scoped browser sessions and mobile browser handoff are outside this implementation. |
| Documentation | Protocol, ADR, API docs, agent playbook and local NyxID skill reference describe the implemented flow and hint contract. No external skill publication is claimed by this update. |

Verification after integration with main:

- Frontend: 3,667 unit tests; production build; lint with zero errors and 27
  warnings in existing main files.
- Browser: 11 application regressions and three HTML mockup regressions pass.
  Coverage includes full/existing/new approval, one-time password/MFA,
  short/long social return hints, connection ambiguity, platform grants,
  permission dropdown keyboard/mixed state and desktop/mobile overflow.
- Mobile: 50 tests, TypeScript and native dependency checks pass. The installed
  scanner fixture matches the pre-#1544 parser byte for byte.
- Rust: integrated compile, backend/CLI tests, Clippy, feature builds and coverage
  are being verified for the new PR; historical results from #1569 are not
  reported as results for this revision.

Physical iPhone acceptance remains a rollout check. The HTML preview and parser
fixture do not establish it. Enable eight-character v2 issuance only after every
backend reader and writer has been upgraded and old replicas have drained.
