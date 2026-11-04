# Explicit execution permissions (#2193 / #2194 Phase A)

**Status: #2193 contract plus default-off #2194 Phase A dispatch integration.**
No API/tool writes execution permissions; #2195 owns human permission
updates/resume and #2196 owns legacy mapping. The contract sections below describe
the shared contract; the Phase A section defines its supported runtime integration. Existing unmarked rows,
turns and native adapters retain their behavior. Unrelated requests gain no
database reads.

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
| `revision` | Positive permission revision; admissions fence it live. |

`services/execution_permission.rs` is pure: it receives trusted server snapshots,
performs no I/O, emits no cards, mutates no record, consumes no claim and dispatches
nothing. Its `Covered` is evidence for the dispatcher, not authorization to
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

Full/Auto/Ask remain defaults for uncovered actions; the dispatcher
uses coverage only for the three eligible floors. Existing org NeedsApproval
retains its documented Full-access bypass. The new default-false
`ApprovalRule.mandatory_review` is a reader-only marker for separately authored
org policy, not an interpretation of old rules. Current configuration validation
refuses `true`; participating dispatch refuses this non-satisfiable requirement,
while org-admin authoring requires a later rollout.

## Validity, tasks and credentials

| Human choice | Required evidence | Legacy `expires_at` (TTL storage projection) |
| --- | --- | --- |
| One prepared operation | Exact server operation ID/digest, unexpired, unconsumed claim. Coverage does not consume it; the dispatcher claims atomically using `execution_use` before sending. Uncertain outcomes stay errors. | Operation expiry. |
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
starts a new root. Cancellation/terminal state invalidates task coverage. Phase A
records these roots and continuation fences only for enrolled work.

Delegation only narrows: every supplied parent/child operation ceiling must cover
the same prepared effect. Changing actor or principal cannot reuse a grant. A
delegated specialist retains the validated task root but needs its own permission;
its parent's consent never transfers to the child principal.

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

1. Deploy all readers and dispatchers first, with `permissions:execution-v1`
   default off. Activation requires both reader and dispatch capabilities in the
   all-replica roster and drained-writer gate, and supports per-person overrides
   only. Reads and disabling remain available.
2. #2195 authoring must require a separate readiness capability before writing
   permissions, native references or mandatory org markers. Reader or dispatch
   readiness alone never enables these writes.
3. Old readers ignore the optional payload and reject its scope marker for
   legacy approval. Old listing is acceptable during the reader rollout; old
   replicas must not serve activated cohorts. No TTL migration is needed.
4. Before binary rollback, disable and drain marked roots and claims on upgraded
   replicas. After future authoring, also revoke/drain native references and
   mandatory policies before old binaries return. Persistent records survive
   rollback's unchanged TTL.

Phase A does not include permission authoring/UI, consolidated gap cards,
native-policy-key execution, Machine service HTTP, async/background watches,
voice-task service calls, mandatory-org authoring or legacy migration. Tests cover
the pure contract, serialization/legacy management, real proxy/MCP/LLM admission,
provider attempt counts, task fences, concurrent claims and the unmarked database
command budget. The adapter matrix below defines the exact boundary.

## Dispatch integration: #2194 Phase A

Phase A adds default-off enforcement for existing execution-contract fixtures;
permission authoring and the consolidated approve → persist → resume workflow
remain #2195. It never converts a legacy approval or Always consent into an
execution permission, and creates no proposal fields or inert cards.

### Activation and read budget

`permissions:execution-v1` can be enabled **only for an active person via a user
override**. Global and organization enables are rejected with a validation error;
reads, disabling and clearing remain available. Activation requires both
`permissions:execution-v1` (reader) and `permissions:execution-dispatch-v1`
(enforcement) in the existing all-replica readiness/drained-writer gate.

The override and `User.execution_permission_enrollment = 1` are written in the
same transaction; disabling/clearing removes the marker. Authentication copies
this non-authorizing hint from the person it already loads. First-party turn
admission may persist a server task root; general personal API keys and assistant
thread keys construct a request-local context. Delegated, OAuth-client and
service-account principals never participate. No enrollment scan, new environment
variable, background reconciler, or cross-request authorization cache is added.

An unmarked request performs no execution-permission database command. Only
marked supported calls load a bounded batch of candidate grants for the exact
actor/principal/connection and one request-local policy decision. Candidates keep
their individual claim state; complete reusable coverage is preferred and partial
permissions are never spliced. Marked in-flight work rechecks enrollment and both
readiness capabilities before dispatch, including a gap independently allowed by
the legacy gates. Disabling cannot downgrade already admitted coverage to Full.

### Adapter boundary

| Adapter | Phase A behavior |
| --- | --- |
| Ordinary bounded HTTP proxy, catalog UUID / connection slug / selected pool member | Participates after concrete resolution and existing ceilings/input checks. Coverage binds one exact connection; it cannot authorize a pool fallback account. |
| Assistant owner, org-member and shared HTTP connection admission | One decision may satisfy the eligible risk/personal-service/shared-actor floors. Existing selection, live key/share/grants, org instruction and model-lane rules remain. |
| MCP endpoint, generic and prepared bounded HTTP | The prepared method/path/query/body/declared headers and exact connection feed the same coordinator; the receipt is carried to final HTTP or node egress. |
| LLM provider and translated gateways | Participates on the final translated request. Stateless model recognition, billing and provider-hosted tool restrictions are unchanged. |
| Native permission-bound keys, Machine service HTTP, async/background watches, voice service tasks | Phase B. Explicitly excluded from new coverage and retain their existing gates. |
| Guests, platform/channel/automation turns, WS/CONNECT, streaming request bodies, SSH/native commands | No execution-permission coverage; existing authority and confirmation rules apply. |
| Managed/composite service adapters, catalog-only calls without an exact connection, resource-policy grants needing a native adapter | No new coverage; no fabricated HTTP/resource evidence. Supported ordinary HTTP still uses its normal ceilings. |

Decision order is ceilings/deny → full coverage evaluation → remaining floors →
claims → final live revalidation → dispatch. **Covered** suppresses only the
three closed eligible floors above, without making fake legacy consents or
receipts. **RequestableGap** goes through today's prompt chain unchanged; multiple
legacy layers can still ask until #2195. **Prohibited** returns before a new card
or effect. Database, provider, billing, capacity and unsupported-adapter failures
remain operational errors. The separately authored mandatory-org marker, when
encountered in participating admission, refuses coverage; its authoring remains
disabled. Existing unmarked org rules keep their Full-access semantics.

The permission binds actual outbound inputs, including stream-usage normalization
and MCP header preparation. The specialized Codex sender also validates the
constructed destination/body and runs the live one-use barrier before its HTTP
send; transport failures retain an uncertain claim and cannot replay. HTTP
redirects cannot extend coverage. Immediately
before each send, live permission revision/revocation, replacement epoch, selected
account/destination, authenticated key ceiling and any presented login credential's
independent revocation/expiry, turn/task, share/behavior and
result audience are revalidated. Transactional write fences serialize the
provider barrier with key/connection/permission edits and cancellation. A group
turn never defaults to a private audience: collaboration groups use their
membership revision; legacy groups bind a stable digest of their exact roster.
OAuth refreshable bytes are excluded from the credential identity; account
replacement is not. The authority snapshot precedes policy evaluation. A final
committed-selector check catches newly inserted policies that a Mongo snapshot
could not write-conflict against; a refused unsent one-use attempt becomes
`not_sent` and is never reopened.

### Task and operation lifecycle

`TaskAdmission` is additive on existing roots/turns/messages and server-authored
continuation events/cards. It contains the `TaskRef`, original actor/audience,
cancellation epoch, immutable 24-hour hard deadline and parent-turn lineage.
New human requests create distinct roots, even when they drain older event context.
Event turns partition queued work by root, and cancelled/terminal roots are
filtered in bounded batches before admission. Direct specialist delegation
fences the source live turn and task root in the child admission transaction;
Stop also stops its marked descendants, expires their pending/allowed task cards,
and fences card creation/decisions against the same root. Private message Retry retains its
validated root, actor and deadline; card continuations use their stored root.
Invocation and GroupRequest roots use their existing terminal/cancellation and
requester fences. Runtime continuations retain the same admitted turn. An
unrecorded or changed actor/audience cannot select a task by supplying an ID.
Destination principals still require their own permissions: a task ID transfers
no consent. Legacy turns without a recorded task can use reusable coverage within
their real audience, but cannot claim task-scoped coverage by inference.

Only `Validity::Operation` uses the versioned `ApprovalGrant.execution_use`
sibling. After the remaining legacy floors pass, an absent → claimed CAS and a
second claim-owned claimed → dispatched CAS fence the live permission and roots.
Provider I/O occurs outside retried transactions. Concurrent duplicates send at
most once; a lost outcome or orphaned claim never resets to available. Completion
or uncertain outcome records the claim state and revokes the operation row with
the existing 30-day retention projection. Reusable permissions are not consumed.
The remote send is linearized at the dispatch barrier; cancellation after it can
stop further sends but cannot undo an already admitted provider effect.

Upgrade readers and dispatchers, drain older writers, then enable per-person
cohorts. Old replicas ignore additive fields and keep their legacy behavior;
they must not serve an activated cohort. Before binary rollback, disable and
drain marked roots and claims on upgraded replicas. The `expires_at` TTL remains
unchanged, including persistent permission sentinel retention. No new permission
writer, legacy migration, native-policy-key cutover or version bump is included.
