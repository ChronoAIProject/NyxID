# Explicit execution permission contract (#2193)

**Status: reader and pure evaluator foundation only.** No dispatcher calls this
evaluator, no API/tool writes execution permissions, and enabling the default-off
`permissions:execution-v1` flag does not enable execution or authoring. #2194
integrates execution; #2195 owns human permission updates/resume; #2196 owns
legacy mapping. Existing rows, turns, approval modes and supported native
adapters retain their behavior. Unrelated requests gain no database reads.

## Representation and scope

An existing `ApprovalGrant` may carry an optional `execution` block. There is no
new grant collection or policy language. Absent means legacy, not unrestricted
execution permission. `execution.version = 1` selects the `v1` payload; unknown
versions and absent/malformed payloads never cover execution. Unknown envelopes
remain inspectable/revocable. Incompatible future payloads must use a new version
and payload key. New fields on existing records are serde-defaulted.

| Field | Meaning and existing representation |
| --- | --- |
| `subject` | Exact acting person, principal (`agent`, `api_key`, `oauth_client`, `service_account`) and execution lane. A shared recipient never borrows the owner's or another recipient's consent. |
| `binding` | Exact connection/owner, selected credential ID and replacement epoch, existing `execution_authority` digest, and shared-agent behavior digest. Catalog ID or slug alone never establishes account equivalence. |
| `context` | Result audience: private person, group + membership revision, or channel binding + revision. Conversation is exact, or explicitly `same_audience`; absence is not a wildcard. Approval delivery audience remains an independent gate. |
| `operations` | Existing `AgentOperationScope`, `ScopedOperation` and `InputRules`. Checks the resolved method, canonical path, endpoint/catalog and prepared query/body. Ceilings alone are not affirmative consent. |
| `resource_policy` | Optional existing `permissions::Policy`. Requires a supported adapter and trusted live evidence binding policy, prepared operation and authority digests. Generic inputs cannot prove Drive ancestry. |
| `validity` | One operation, one logical task, fixed window, or persistent (below). |
| `satisfies` | Explicit subset of the three eligible floors below. |
| `revision` | Positive permission revision; future admissions must fence it live. |

`services/execution_permission.rs` is pure: it receives trusted server snapshots,
performs no I/O, emits no cards, mutates no record, consumes no claim and dispatches
nothing. Its `Covered` is evidence for a future dispatcher, not authorization to
skip the other gates. The caller must resolve live ACLs, principal/credential
ownership, task state and resource checks; request/model assertions cannot supply
these facts. Failure to load them is an operational error, never permission.

## Exactly which prompts can be satisfied

The evaluator uses a closed `ExecutionFloor` enum. A permission declaring any
prohibited floor is invalid; an otherwise valid permission encountering a separate
floor returns a gap for that floor, leaving its existing approval/claim gate in
control. Nothing in this release changes those gates.

| Floor | May v1 permission satisfy it? |
| --- | --- |
| Access-mode risk prompts (Auto/Ask) | Yes, within the permission's complete scope. |
| Personal service approval layer (Grant/PerRequest) | Yes, only for the acting person's own connection. |
| Shared-agent connection consent | Yes, only that exact actor in the shared lane, their connection and bound agent behavior. |
| Webhook/automation confirmation (untrusted input) | **No.** |
| `machine_confirm` | **No.** |
| Saved-login `confirm_each_sign_in` | **No.** |
| Delegation widening | **No.** |
| Existing organization service approval layer | **No.** Its current access-mode behavior remains unchanged. |
| Organization instruction requirement | **No.** |
| Separately authored org per-operation mandatory review | **No.** |
| Exact approval / durable operation one-use claims | **No.** |
| Any guest execution | **No; prohibited**, even with a matching permission. |

Full/Auto/Ask remain defaults for uncovered actions; a future integrated dispatcher
may use coverage only for the three eligible floors. Existing org NeedsApproval
retains its documented Full-access bypass. The new default-false
`ApprovalRule.mandatory_review` is a reader-only marker for separately authored
org policy, not an interpretation of old rules. Current configuration validation
refuses `true`; enforcement and org-admin authoring require the later rollout.

## Validity, tasks and credentials

| Human choice | Required evidence | Legacy `expires_at` (TTL storage projection) |
| --- | --- | --- |
| One prepared operation | Exact server operation ID/digest, unexpired, unconsumed claim. Coverage does not consume it; the future dispatcher must claim atomically through the existing one-use mechanism. Uncertain outcomes stay errors. | Operation expiry. |
| One logical task | Exact `TaskRef`, actor/audience, live active root, recorded admitted descendant, hard deadline. | Task's hard deadline. |
| Fixed window | Explicit inclusive start and exclusive end; no extension while waiting. | Window end. |
| Persistent until revoked/binding invalid | Live binding and authority on every use; no permission time expiry. | `9999-12-31T23:59:59Z` (`PERSISTENT_TTL_SECONDS = 253402300799`). |
| Revoked/terminal | No execution permitted; preserve only bounded inspection/cleanup history. | Transition time + 30 days (`TERMINAL_RETENTION_SECONDS`). |

**The existing `approval_grants.expires_at` TTL index is unchanged.** There is no
`purge_after`, index swap or backfill. `execution.validity` is authoritative;
`expires_at` is checked for the expected projection on active records. A terminal
transition must revoke the row and assign bounded retention in the same write.
A task becoming inactive immediately fails live evidence, even before cleanup.
Revocation helpers apply terminal retention only to execution-block rows and
leave legacy expiration values unchanged.

`TaskRef { root_kind, root_id }` normalizes existing message, invocation and group
request roots. It is server-only identity, **never bearer authority**. Continuation
or recovery may retain a root only with a server-recorded admitted descendant
edge and the original actor/audience/deadline. A new request or scheduled run
starts a new root. Cancellation/terminal state invalidates task coverage. This
release introduces neither root-state writers nor continuation plumbing.

Delegation only narrows: every supplied parent/child operation ceiling must cover
the same prepared effect. Changing actor or principal cannot reuse a grant. A
cross-principal delegation mechanism is not implemented here; later integration
must prove a reviewed descendant binding without transferring owner consent.

Candidate selection prefers a completely covering persistent/window/task
permission before an operation claim. Partial rows are never spliced. If none
covers, operational failures remain errors; otherwise an eligible requestable gap
precedes unrelated prohibited rows. Live ceiling denial and guest execution take
precedence over all candidates. The eligible human and private card audience must
be established separately; without an eligible decider a gap is prohibited.

## Binding and invalidation

| Event | Coverage result |
| --- | --- |
| Same-account, same-epoch OAuth refresh/token expiry then successful refresh | Remains covered. Refreshed token bytes/expiry are absent from the execution-authority digest. |
| Refresh/provider/database failure | Operational error; never advice to re-consent. |
| User-initiated account replacement, credential ID/epoch, override or platform/BYOK change | Binding gap; no old permission reuse. Credential replacement bumps the epoch; background refresh must not. |
| Destination, identity injection, contract/authority digest change | Binding gap; live execution projection must be rebuilt. |
| Resource/input outside scope, changed audience/revision or exact conversation | Gap if a permitted human can decide; otherwise prohibited. Resource proof denial stays prohibited. |
| Permission revocation, revoked key/share/org membership or other live ceiling denial | Prohibited. |
| Task stop/terminal/deadline or unrecorded descendant | Prohibited for that task. |
| Shared persona/skill/behavior digest change | Binding gap. A new actor cannot reuse existing shared connection consent. |
| Access-mode change alone | Not part of the explicit binding; explicit scope remains. Other floors still apply. |

Permission is distinct from the short-lived credential exercising it.
`ApiKey.permission_policy_id` is an optional reader reference to an immutable
native policy. Missing keeps today's one-key pilot semantics; key rotation
preserves a present reference. No endpoint sets it, and native policy resolution
still uses its current key-bound path. Native issuance remains personal,
server-held Google/Drive only, with its existing 90-day credential limit.
No new adapter, org/Machine route or generic proxy permission route is enabled.

## Legacy readers and management

Execution rows must use `scope: "execution:v1"`, never null or a legacy `v1:...`
scope. `approval_policy::ParsedScope::parse` rejects it. Real legacy
`check_approval` tests cover both personal and `org_scoped=true` records (including
the org branch's personal-grant fallback); a future TTL cannot make them legacy
authority. `org_scoped` does not relax the execution payload's exact actor.

| Reader/writer | Handling |
| --- | --- |
| `approval_service::check_approval` / `grant_scope_covers` | Unchanged legacy dispatch logic rejects the marker. |
| `approval_service::list_grants`, `handlers/approvals` | Execution rows independent of legacy Grant-mode configuration; same owner/admin service visibility. Additive `execution` DTO reports effective validity, not sentinel lifetime. No prepared inputs, resource policy or authority/credential digests exposed. Unknown versions marked unsupported. `supported` means the schema is understood, not that a live effect is authorized; task/admission/resource checks remain separate. Legacy fields retained for compatibility. |
| `revoke_grant`, `revoke_all_grants`, service-policy reconciliation | Existing ownership/filter semantics; execution rows get bounded terminal retention in the same update, legacy dates preserved. `unscoped_only` still matches only `scope:null`, excluding the marker. |
| `assistant_action_effects_org::grant_for_actor` | Existing owner/admin/resource checks govern revocation; calls the shared revoke helper. Does not execute grants. |
| `org_service` blocker/cleanup queries | Active persistent grants block deletion; revoked/expired grants remain eligible for cleanup. |
| `assistant_group_service` crew retirement | Existing requester-key filter and transaction use the same revocation update. No routing or admission changes. |

Legacy allowlists, AgentGrants, InputRules, native policies and receipts are not
upgraded into explicit consent. Existing Always/Once/cards/exact/durable grants
retain their own scope, consumption and binding rules. #2196 requires explicit
review wherever account, subject, inputs or audience cannot be proven from the
old record; no widening by inference.

## Rollout and unsupported behavior

1. Deploy readers first. `permissions:execution-v1` is registered default off in
   both flag registries and the existing security-readiness capability registry.
   Admin activation (global/person/org overrides) uses the existing all-replica
   roster and drained-writer gate; reads and disabling remain available.
2. This capability attests to **this contract reader**, not future enforcement.
   #2194/#2195 must require a distinct enforcement/authoring readiness version
   before writing permissions, native references or mandatory org markers.
   A green #2193 capability alone must never enable those writes.
3. Drain old API, worker, permission and credential writers before future
   authoring. Old readers ignore the optional payload and reject its marker for
   approval. Old listing is acceptable during the reader rollout. No data or TTL
   migration is needed for this step; binary rollback is safe with no new writes.
4. After later authoring, disabling a flag must not downgrade new-format
   constraints into legacy authority. Revoke/drain new native references,
   mandatory policies and pending work before old binaries return. Persistent
   records survive rollback's unchanged TTL.

Unsupported in #2193: runtime prompt suppression, grant creation/update APIs or
UI, task root persistence/admission/recovery, transaction claims and cancellation
fences, native-policy indirection execution, cross-principal delegation,
mandatory-org enforcement, migrations and expanded adapters. Tests here cover
pure contract decisions, BSON compatibility, legacy refusal, management visibility
and revocation, and readiness activation. Real transport/provider-attempt and
concurrent claim tests belong to #2194/#2195, not a mock of unused dispatch.
