# Admin form save behavior

Admin edit forms capture saved values when editing begins. Service and provider
pages mount their controls after the first successful load. Background loading,
refetch failures with cached data, and same-record refreshes retain open drafts.
Feature-flag cards retain drafts when collapsed or filtered out. Pristine metadata
editors follow refreshed saved values, including before a card is first opened. Switching the
record ID ends that editor's lifetime, including any pending review; endpoint
editors also include the parent service ID in their identity.

The review dialog receives a copied, validated payload with its target ID and a
description of the changes. Cancel sends nothing. Confirm submits that same
payload and guards duplicate clicks synchronously. Failed writes keep the draft
and show the error in the review. ApiError currently has no field-error map.
Credential previews describe replacement or removal without displaying secret
values. Stored provider client credentials remain write-only; read responses
supply configured indicators and saved endpoint URLs.

## Update contracts

`changedFields` compares normalized values with the fixed editing snapshot.
Omitted fields preserve saved values on the update APIs below. Changed arrays
and nested configuration blocks remain replacement values unless the contract
explicitly supports a partial block. Creation/provisioning workflows retain
their existing contracts and are outside this edit-form change.

| Editor | Submitted update |
| --- | --- |
| Services | Changed top-level fields; complete changed SSH/capability/header/WS blocks. Unrelated updates do not discover or rewrite spec URLs or start endpoint sync. |
| Service billing/platform metadata | Saved inference, platform-key grants, credential-class prices and charge-only-NyxID toggle are populated. Lane-only changes send only changed lanes; null clears a lane or inference. Legacy billing edits preserve resale/pricing/cleanup fields and omit unchanged lanes. Blank replacement credentials are omitted and replacements are redacted. |
| Providers | Changed fields, deliberate empty scope lists, optional-text clears, and credential replacements. Revocation URL changes preserve style/auth/grant/request-encoding settings; null clears structured revocation. |
| Platform credentials | Changed descriptor fields; null clears, blank secret input preserves, blank non-secret input clears. Shared OAuth backing and the effect on provider connections/logins remain visible in clear confirmations. |
| Users, roles, groups, service accounts | Changed fields compared with the dialog's opening values. Edited role/permission arrays are compared with the latest observed record before confirmation. |
| Service endpoints | The review and mutation use the same parsed JSON delta. Null clears optional fields; whitespace-only JSON formatting is a no-op. |
| Credit allowances and schedules | Changed editable fields; immutable schedule recurrence is excluded. Replacement recipient/service selections have observed-conflict checks. Status changes also require review. |
| Platform operations | Changed top-level settings; changed typed config is a complete block. A broken vendor binding can be disabled without decrypting its credentials; enabling still validates the binding. |
| Flag metadata | Sparse PATCH of description/owner. Omission preserves; null or trimmed blank clears. |
| Vendor templates | Sparse PATCH of submitted editable fields. Omitted fields preserve; auth_key_name/operation accept null. Other fields reject null and retain existing validation. Disable also requires confirmation. |
| Feature rollout | A copied batch shows each flag, target scope/ID, and before/after override. No mutation occurs before confirmation. |
| Anonymous public rules | Staged enabled/method/path/quota changes, sparse normalized payloads, wildcard exposure warnings, and reviewed deletion. |
| Invite notes | Copied note and code ID are reviewed before saving. |

Two additive routes provide sparse updates without changing existing clients:

- `PATCH /api/v1/admin/feature-flags/{flag_key}/metadata`
- `PATCH /api/v1/admin/platform-ops/vendor-templates/{template_id}`

Both routes reject unknown keys and return the authoritative saved descriptor.
Their existing PUT routes retain full-replacement compatibility. PATCH metadata
keeps empty rows internally to avoid a read-then-delete race with disjoint edits;
empty metadata is still presented as the code default. Template PATCH validates
the merged configuration, writes only submitted changed fields, and fences the
operation/slug/URL/auth fields used by cross-field validation. A dependent-field
race retries validation against current data or returns conflict; it cannot
persist an invalid combination simply because two individually valid patches
were concurrent. This narrow predicate is not a universal browser revision lock.

Provider optional text clears remove the stored field. Cleared user display
names/avatars and role/group/service-account descriptions are stored as null.
Group parent and service-account rate-limit clears retain their existing null
semantics. Endpoint and rate-limit request deserialization distinguishes omitted,
null, and provided values.

## Conflict, draft, and cache behavior

User/role/group/service-account dialogs and credit/endpoint editors check the
fields included in the proposed write against their opening values. An observed
role revocation blocks an edited replacement role list; an unrelated rename can
still proceed without restoring roles. Anonymous-rule exposure edits also check
the related enabled/method/path fields. Other configuration editors compare their
editable projection rather than volatile response timestamps or billing sync
status. Intentionally entered hidden credentials additionally use timestamp or
presence changes to detect observed credential replacement.

Selected IDs remain visible and removable when outside current search results,
inactive, or unavailable. Known user names/emails, service names, and group role
names remain the display fallback before raw IDs. SSH mode is explicit, the
legacy certificate flag follows it, and leaving node_key warns that node-local
keys remain until pruned.

Hooks that publish saved operation, credential, metadata, rollout, or anonymous
rule responses cancel outstanding reads before publication. A pre-save delayed
GET cannot replace that saved cache afterward. Query invalidation still refreshes
related views. Credential deletion records write success independently from its
follow-up read: a failed GET closes the destructive confirmation and requires a
read-only refresh before editing. Retry does not repeat DELETE. Verify-token
regeneration preserves other draft credential fields.

Rollout batches are nontransactional. Each successful entry is consumed only if
its draft version still matches the reviewed version. Failed entries and newer
edits remain for another review; the outcome identifies failed targets. A second
confirmation cannot launch the same in-flight batch. Public-rule inputs are
locked during saving and same-record refetches do not reset their drafts.

The adjacent audit also checked OAuth-client actions, admin nodes, read-only
audit/integrity pages, and existing creation flows. OAuth clients and node actions
retain their action-specific confirmations. This document does not claim every
creation/provisioning workflow has a generic change-review dialog.

## Verification and limits

Validation on 2026-09-17 covers the integrated changes, including the independent
Astra and Fable review corrections. Both reviews closed with no known unresolved
finding in the audited scope. Final source hashes were checked against Fable's
review manifest.

- The final focused frontend run passed 103 tests in 18 files. These exercise
  populated saved values, selections that become inactive or unavailable, exact
  sparse bodies, cancellation, source switches, edited authorization conflicts,
  unrelated renames after revocation, structured revocation preservation/clear,
  secret redaction, delayed reads, failed refresh after successful DELETE,
  deferred metadata hydration, rollout partial failures, SSH transitions, and
  billing/platform metadata integration.
- The initial full frontend sweep passed 3,270 tests and failed one outdated
  provider-editor expectation. That test was corrected for the review step and
  exact sparse body, then passed in the final focused run. The full suite was not
  rerun after the final corrections; these overlapping counts are not additive.
- The final production build passed TypeScript, the main Vite build, the separate
  credential-accept build, and the mock-footprint assertion. Full ESLint passed
  with 27 existing warnings outside the change; lint also passed on the final
  changed frontend files.
- Nine Chromium checks passed: six credential scenarios, two role-editor checks
  at desktop and mobile widths, and successful credential deletion followed by a
  failed read and read-only retry. They verify redaction, confirmation before
  writes, cancellation preserving drafts, exact sparse requests, and a DELETE
  count of one through recovery.
- All 349 selected backend tests passed across 15 affected handler/service
  modules against an isolated MongoDB 8.0.12 replica set, with no ignored tests
  or database skips. The test executable was compiled from the final backend
  source with debug symbols disabled for the backend crate; test assertions
  were unchanged.
- Workspace Clippy passed with warnings denied (`cargo clippy --workspace
  --all-targets -- -D warnings`). Rust formatting and whitespace checks passed.

Backend regressions cover sparse metadata/template persistence and PUT
compatibility, nullable request parsing, optional-field storage shape,
spec-discovery gating, merged-template validation, and disabling a broken vendor
binding. The CLI wizard's source manifest has no overlap with the changed source,
and its committed closure hash matches; no wizard bundle rebuild is required.

Observed-conflict checks do not prevent an unseen concurrent write after the
last browser read. Same-field writes and changed replacement arrays/blocks can
still race at the server; existing PUT replacement clients retain that risk.
There is no universal expected-revision/CAS expansion in this change. Nested
blocks are reviewed as JSON, and batches do not provide transaction rollback.
