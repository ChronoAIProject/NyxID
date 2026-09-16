# Admin form save behavior

Admin edit forms capture saved values when editing begins. Service and provider
pages mount their controls after the first successful load. Background loading,
refetch failures with cached data, and same-record refreshes retain open drafts.
Feature-flag cards retain drafts when collapsed or filtered out. Switching the
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

Focused regressions exercise real forms and mutation hooks: populated saved
values, exact sparse bodies, cancellation, source switches (including endpoint
parent identity), edited authorization conflicts, unrelated renames after
revocation, structured revocation preservation/clear, secret redaction, own-save
cache state, delayed reads, failed refresh after successful DELETE, metadata
filter/refetch draft retention, rollout partial failures/newer drafts, SSH
transitions, and upstream billing/platform metadata integration.

Added Rust tests cover sparse metadata/template persistence and PUT compatibility,
nullable request parsing, optional-field storage shape, spec-discovery gating,
merged-template validation, and disabling a broken vendor binding. New Rust code
has been formatted; compilation/database execution is pending the parent's final
integrated backend gate. The old backend executable cannot validate these changes.

The previous 2026-09-16 validation (301 files/2,958 frontend tests and 53 focused
backend tests) predates the independent-review fixes and main integration. Those
numbers are superseded and are not evidence for the current source. Current
focused runs passed 86 tests in 14 files, followed by 41 tests in six affected
files and four new tests in two files (these sets overlap). The parent's first
full frontend run passed 3,270 tests in 326 files and failed one outdated provider
editor test. That test now passes with configured labels, saved URL fixtures,
review confirmation, and an exact sparse credential body. Parent full ESLint
passed with 27 existing warnings outside this change. Final integrated build,
backend, and review status are recorded separately by the parent.

Parent-provided real Chromium evidence: two role-editor tests passed at 1440px
and 390px (saved fields, stacked dialogs, Cancel preserving draft/no request,
confirmation sending exactly `{name:'Renamed operators'}`, no page errors).
A credential-clear browser test also passed: DELETE succeeds, the following GET
fails, confirmation closes, and Retry reads restored credentials with total
DELETE count still exactly one. Credential E2E specs now include the review step
and field-clear intent; their final integrated browser run is pending.

Observed-conflict checks do not prevent an unseen concurrent write after the
last browser read. Same-field writes and changed replacement arrays/blocks can
still race at the server; existing PUT replacement clients retain that risk.
There is no universal expected-revision/CAS expansion in this change. Nested
blocks are reviewed as JSON, and batches do not provide transaction rollback.
