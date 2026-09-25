# Rollup: 2026-09-25 ctkm-1

Branch `rollup-2026-09-25-ctkm-1` collects reviewed changes before integration
into `main`. It existed at current `main` commit
`1b031c77062880e572ec375041a4c029241a86f1` when PR #1666 was retargeted on
2026-09-25. The existing branch was reused without resetting its history.

## Device login and scoped approval

[PR #1666](https://github.com/ChronoAIProject/NyxID/pull/1666) addresses the
installed iOS app rejecting nine-character device codes and the approval-time
scope selection requested in
[issue #1535](https://github.com/ChronoAIProject/NyxID/issues/1535).
It supersedes the closed, unmerged PR #1569.

The intended behavior is:

- Ordinary browser login uses the legacy eight-character account-login
  exchange. Grant-capable device requests can also use eight-character codes
  after the staged compatibility flag is enabled. Existing prefixed codes
  remain readable, and ambiguous codes cannot approve either request.
- `/login/device?user_code=…` reviews the request, asks the approver to choose
  full account access or a restricted Agent Key, then shows the applicable
  permission review. The approver can reuse an eligible key or create a new
  one. Existing and new keys use the same permission display.
- An existing browser session skips sign-in. Otherwise the user verifies
  identity for this request through a configured sign-in provider or
  password/MFA. That proof expires within ten minutes and ends after the
  decision. Keeping a browser session is an explicit choice. Verification
  alone does not authorize the requesting device.
- Permission filters show exact matches before keys with additional access.
  The server supplies effective service permissions and revalidates consent
  snapshots before approval. URL hints prefill preferences without granting
  authority or automatically creating keys.
- CLI login options construct the corresponding approval links. Public
  catalog discovery and credential-scoped MCP discovery help requesters
  identify services and operations. The protocol and local NyxID skill
  document the supported URL fields and commands.

The production React implementation and its backend, CLI, mobile capability
and regression-test support are retained. The standalone HTML prototype,
saved service snapshot, prototype notes and prototype-only tests are removed.
They are not runtime dependencies. This work does not add mobile browser
handoff or scoped browser sessions.

## Verification and rollout

The production-code revision passed backend, CLI, frontend and mobile tests,
coverage gates, Clippy, formatting, feature builds and CodeQL. After prototype
removal, the production frontend build and all 11 application browser tests
passed again. The source PR records the checks for the final rollup-targeted
revision and the squash commit that lands it.

`AUTH_DEVICE_EIGHT_CHAR_CODES` defaults to `false`. Upgrade all backend readers
and writers and drain incompatible replicas before enabling eight-character
v2 issuance. Physical installed-iPhone scan and manual-entry acceptance remain
rollout checks. Merging into this rollup does not deploy or enable the flag.

See [the device login protocol](../DEVICE_LOGIN_PROTOCOL.md),
[the compatibility rollout ADR](../ADR-015-auth-device-login.md), and
[the HTTP API contract](../API.md#selectable-device-login-v2).
