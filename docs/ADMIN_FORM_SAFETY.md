# Admin form save behavior

Admin edit forms capture saved values when editing begins. Service and provider
pages mount their controls only after the initial request succeeds. A changed
query response must not reset an open draft. Editors backed by continuously
refreshed configuration show a notice when their saved snapshot differs from
newly fetched data and require an explicit reload before confirmation.

The review dialog receives a copied, validated update payload and a description
of its changes. Cancel sends no request. Confirmation submits that copied
payload. Errors retain the review. Credential previews describe replacement or
removal without displaying secret values. Stored provider client credentials
remain write-only; their read response supplies separate configured indicators.

## Update contracts

`changedFields` compares normalized values with the fixed editing snapshot.
Omitted fields mean preserve only on APIs that support that convention. Arrays
and nested configuration blocks remain complete replacement values when changed;
do not recursively prune them. Creation forms keep their existing contracts.

| Editor | Submitted update |
| --- | --- |
| Services | Changed top-level fields; complete SSH, billing, capabilities, headers, or WS rules only when that block changes |
| Providers | Changed fields, explicit empty scope lists, deliberate credential replacement, and structured revocation updates that preserve style/auth/grant settings |
| Platform credentials | Changed descriptor fields; `null` clears, blank secret input preserves, blank non-secret input clears |
| Users, roles, groups | Changed fields compared with values captured when the dialog opened |
| Service accounts | Changed fields compared with the opening snapshot, so a refreshed revocation does not become an unintended re-enable or role restoration |
| Service endpoints | Changed editable fields; explicit nulls clear optional fields |
| Credit allowances and schedules | Changed editable fields; immutable schedule recurrence is excluded |
| Platform operations | Changed top-level settings; a changed typed config remains a complete block |
| Feature-flag metadata and vendor templates | Existing full-replacement PUT contracts; review only user changes and block if the fetched editable configuration diverges from the opening snapshot |

Feature-flag metadata and vendor-template PUT requests still require complete
bodies. Sending sparse objects to those APIs would clear data or fail validation.
Literal sparse writes for those two editors require separate API contract work.
Platform-operation PUT now accepts omitted top-level fields and preserves them;
existing complete requests remain valid.

## Findings addressed

- Provider authorization/token URLs were absent from the read response, while
  form validation required them. Device endpoints were also initialized blank.
- Service edits echoed unrelated identity, discovery, billing and application
  scope fields. This triggered unnecessary reconciliation and validation and
  could replace saved values during an unrelated rename.
- An SSH update omitted `ssh_auth_mode`, allowing the backend's legacy boolean
  fallback to replace `node_key`. The editor now displays all three modes, preserves the
  selected mode on unrelated edits, and derives the legacy certificate flag from
  an explicit mode selection.
- Query refresh effects erased edits. Service/provider/credential/operation
  editors now retain their draft and expose an explicit reload action. Failed
  credential refreshes keep the form mounted. Verify-token regeneration also
  retains unsaved credential edits.
- Selected applications and credit recipients/services disappeared when absent
  from available options. Selected missing IDs remain visible and removable.
- Role/group/service-account descriptions, group parents, provider scopes and
  user avatars did not consistently support deliberate clearing.
- Endpoint optional fields and service-account rate limits used nested Rust
  `Option` values without the nullable deserializer. Explicit null now reaches
  the existing clear branch rather than being treated as omission.

The review also covered OAuth-client actions, invite-code notes, admin nodes,
audit and integrity pages, and create/provisioning flows. OAuth-client actions
already stage explicit confirmations. Invite notes already keep a stable edit
baseline. Nodes use explicit actions; audit and integrity pages are read-only.

## Verification and limits

Regression tests execute real React forms and mutation hooks. They cover initial
loading, saved OAuth/device URLs, name-only update bodies, cancelled reviews,
secret redaction/clears, background-update blocking, SSH mode transitions,
preservation of resale billing settings, service-account permission revocation
while editing, and selected options missing from the current picker results.
Rust tests cover response credential redaction and omission/null/value request
semantics. The platform-operation integration test checks that an enabled-only
update preserves its saved config and vendor binding.

These client checks are not an atomic server revision lock. An unobserved write
after the last browser read can still race with confirmation, especially for
nested blocks and full-replacement APIs. A future server-side expected-revision
contract is needed to reject every such conflict at the database boundary.
These changes do not guarantee that simultaneous edits by two administrators
cannot overwrite each other.

Validation completed on 2026-09-16:

- The full frontend suite passed: 301 files, 2,958 tests.
- TypeScript project checks and ESLint on all changed TypeScript files passed.
- 53 focused Rust tests passed across provider responses, service endpoints,
  service accounts, admin users, and platform operations. Database tests ran
  against an isolated MongoDB 8.0.12 instance. The operation test read back the
  stored row after changing only `enabled` and confirmed that the vendor and
  typed config were preserved.
- `git diff --check` passed. No manual browser acceptance run was performed.

The central regression assertion is an exact mutation payload. Renaming a saved
service sends only `{ name: "Renamed service" }`, and no request occurs before
confirmation. Tests also cancel the review, change the fetched snapshot while
review is open, double-click confirmation during a pending request, and retry a
failed save without losing the draft.
