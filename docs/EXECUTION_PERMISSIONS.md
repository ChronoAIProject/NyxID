# Explicit execution permissions (#2193 / #2194 Phases A–B / #2195 Parts A–B1)

**Status: default-off contract, dispatch, and assistant permission authoring.**
The #2195 Part A flow below adds human permission updates and exact-effect resume
for enrolled private owner, shared-recipient and collaboration chat turns using
MCP or HTTP proxy. Other callers retain #2194 Gap → legacy prompts. #2196 legacy
mapping is not included. Unmarked traffic gains no preparation or database reads.

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
| `approval_service::list_grants`, `handlers/approvals` | Execution rows independent of legacy Grant-mode configuration; same owner/admin service visibility. Additive `execution` DTO reports effective validity, not sentinel lifetime. A bounded, redacted review is returned only to the first-party human; retained HTTP bytes, resource policy and authority/credential digests are never returned. Unknown versions marked unsupported. `supported` means the schema is understood, not that a live effect is authorized; task/admission/resource checks remain separate. Legacy fields retained for compatibility. |
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
   default off. Activation requires reader, dispatch and authoring capabilities in the
   all-replica roster and drained-writer gate. Person/org/global overrides are supported;
   broad activation additionally requires enrollment readiness. Reads and disabling
   remain available.
2. Authoring requires `permissions:execution-authoring-v1` in addition to reader
   and dispatch readiness. Native references and mandatory org markers remain
   unwritable. Reader or dispatch readiness alone never enables new proposals.
3. Old readers ignore the optional payload and reject its scope marker for
   legacy approval. Old listing is acceptable during the reader rollout; old
   replicas must not serve activated cohorts. No TTL migration is needed.
4. Before binary rollback, disable and drain marked roots and claims on upgraded
   replicas. After future authoring, also revoke/drain native references and
   mandatory policies before old binaries return. Persistent records survive
   rollback's unchanged TTL.

#2194 Phase A does not include authoring; #2195 Part A below adds assistant
permission cards and web UI. Phase B adds the bounded adapters below, while
mandatory-org authoring remains excluded. #2196 documents and tests legacy records as-is; it does not backfill consent. Tests cover
the pure contract, serialization/legacy management, real proxy/MCP/LLM admission,
provider attempt counts, task fences, concurrent claims and the unmarked database
command budget. The adapter matrix below defines the exact boundary.

## Dispatch integration: #2194 Phase A

Phase A introduced enforcement; #2195 Parts A and B1 below now supply human
authoring and the consolidated approve → persist → resume workflow for their
listed sources. The registry default remains off, while production is globally
enrolled. Neither dispatch nor enrollment converts a legacy approval or Always
consent into an execution permission.

### Activation and read budget

`permissions:execution-v1` supports active-person, organization and global cohorts.
Global/org activation and clears that enable inherited access additionally require
`permissions:execution-enrollment-v1`. Explicit disabling remains available.
Activation requires
`permissions:execution-v1` (reader), `permissions:execution-dispatch-v1`
(enforcement) and `permissions:execution-authoring-v1` (human authoring) in the existing all-replica readiness/drained-writer gate.

A personal override and its `User.execution_permission_enrollment` hint are
written in the same transaction; disabling/clearing recomputes scoped inheritance. Authentication
combines this hint with the bounded snapshot using the person it already loads. First-party turn
admission may persist a server task root; general personal API keys and assistant
thread keys construct a request-local context. Delegated, OAuth-client and
service-account principals never participate. Organization-only reconciliation
and a process snapshot supply hints, never authorization. No population scan or
new environment variable is added; see broad enrollment below.

A non-candidate request performs no execution-permission database command. Only
marked supported calls load a bounded batch of candidate grants for the exact
actor/principal/connection and one request-local policy decision. Candidates keep
their individual claim state; complete reusable coverage is preferred and partial
permissions are never spliced. Marked in-flight work rechecks enrollment and both
readiness capabilities before dispatch, including a gap independently allowed by
the legacy gates. Disabling cannot downgrade already admitted coverage to Full.

### Adapter boundary

| Adapter | Dispatch behavior |
| --- | --- |
| Ordinary bounded HTTP proxy, catalog UUID / connection slug / selected pool member | Participates after concrete resolution and existing ceilings/input checks. Coverage binds one exact connection; it cannot authorize a pool fallback account. |
| Assistant owner, org-member and shared HTTP connection admission | One decision may satisfy the eligible risk/personal-service/shared-actor floors. Existing selection, live key/share/grants, org instruction and model-lane rules remain. |
| MCP endpoint, generic and prepared bounded HTTP | The prepared method/path/query/body/declared headers and exact connection feed the same coordinator; the receipt is carried to final HTTP or node egress. |
| LLM provider and translated gateways | Participates on the final translated request. Stateless model recognition, billing and provider-hosted tool restrictions are unchanged. |
| Native personal Google/Drive permission-bound REST/MCP keys | Existing policy/adapter verification precedes the coordinator. Verification reads cannot consume the primary claim. Admission refusal precedes the primary-send marker; the 90-day credential cap remains. |
| Assistant Machine service HTTP | The signed job/runtime/service ceilings remain; coverage evaluates the actual bounded HTTP effect. External job principals cannot inherit the creator's consent. |
| Async/background watches | Persist original actor/key/turn/TaskRef. Each leased status, result or cancel operation evaluates its own exact effect; submission is not lifecycle permission. Uncertain submissions cannot create another submission. |
| Voice-task service HTTP/MCP | Retain the authenticated source and original request's task root through continuation/recovery. Live voice/WS/media remain excluded. |
| Guests, platform/channel/automation turns, WS/CONNECT, streaming request bodies, SSH/native commands | No execution-permission coverage; existing authority and confirmation rules apply. |
| Managed/composite service adapters, catalog-only calls without an exact connection, resource-policy grants needing a native adapter | No new coverage; no fabricated HTTP/resource evidence. Supported ordinary HTTP still uses its normal ceilings. |

Decision order is ceilings/deny → full coverage evaluation → remaining floors →
claims → final live revalidation → dispatch. **Covered** suppresses only the
three closed eligible floors above, without making fake legacy consents or
receipts. **RequestableGap** uses a consolidated review only for the eligible Part A or B1
source and retainable effect defined below; otherwise it keeps the legacy chain. **Prohibited** returns before a new card
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

Upgrade readers and dispatchers, drain older writers, then activate the intended
person/org/global cohorts through the readiness gate. Old replicas ignore additive fields and keep their legacy behavior;
they must not serve an activated cohort. Before binary rollback, disable and
drain marked roots and claims on upgraded replicas. The `expires_at` TTL remains
unchanged, including persistent permission sentinel retention. No legacy migration, native-policy-key cutover or version bump is included.


## Permission updates and resume: #2195 Part A

### Eligible sources and one private review

A **Part A (v1)** proposal requires a live, enrolled assistant chat key with a
recoverable server `TaskRef`: private owner, shared-recipient and collaboration-group turns,
through ordinary HTTP proxy or MCP endpoint/generic/prepared HTTP. Guest,
platform/channel/automation, old in-flight turns without recorded lineage,
ordinary API-key/external and standalone LLM-gateway callers cannot create v1
proposals. B1 below adds v2 authoring for eligible external/LLM sources; the other
excluded sources retain their existing gap behavior. Unsupported managed/composite transformations also retain legacy
prompts. Phase B adapters never create Part A proposals; the B1 section below adds separately tagged proposals for eligible sources.

The existing `ApprovalRequest` carries one versioned `execution_proposal`.
Ceilings and prohibitions run before its creation. It consolidates only missing
access-mode, personal-service and exact-actor shared-connection consent floors.
There is no second acknowledgement card, inferred legacy grant or Always consent.
Independent floors remain required; saving permission never approves them.
The active acting person is the only decider. Full access cannot self-approve,
and another group participant, owner or manager cannot see the private review.

The proposal binds exact actor/principal/lane, chat key/source credential,
conversation and result audience, task/epoch/deadline, selected connection/epoch,
agent behavior, authority and endpoint snapshots, prepared inputs and eligible
floors. A keyed digest deduplicates matching pending gaps. Denial and uncertain
outcomes stay task-bound. Duplicate calls from the original or resume turn return
the same receipt; after its delivery, a different server-admitted turn can create
a new occurrence under reusable permission. Identical calls within one server
turn are conservatively one occurrence; a model request ID cannot override this.
A task has at most 32 proposal rows. NyxID-generated proxy correlation IDs are excluded from gap/scope identity,
while retention and send-time checks preserve their exact outbound bytes. Only
the trusted server-generated marker allows this; caller-supplied IDs remain
exact, reviewed and Operation-only.

The encrypted retained prepared HTTP effect, including encryption overhead, is
capped at **256 KiB**. Raw/multipart, oversized, credential-bearing headers,
paths requiring URI encoding/decoding, or unreconstructable destinations use the legacy chain with a metadata-only reason
code, before any proposal is written. Connection authentication credentials are re-resolved at dispatch, never retained in the effect.
Private payloads/results are purged after 24 hours or source cancellation/deletion;
request metadata retains the existing approval-request TTL.

### Web decision and validity

`GET /api/v1/approvals/requests/{id}` returns the additive proposal DTO only to its
first-party human decider. `POST /api/v1/approvals/requests/{id}/execution-decision`
requires that human and accepts only:

```json
{"approved":true,"expected_revision":1,"choice_id":"operation","choice_digest":"server-issued","idempotency_key":"client-generated-uuid"}
```

The client cannot author a connection, audience, scope, validity payload or task.
Operation is the default. The immutable choices are this operation, this task,
one displayed hour, and persistent until revoked/binding changes. These existing
choices remain bound to the current conversation, including Persistent. Reusable
choices require an enforceable closed shape: exact path/query and JSON inputs,
no semantic headers beyond JSON Content-Type, and empty bodies for bodyless
methods. Sensitive/large inputs permit Operation only. The same shape restriction
runs on every reusable use and again at egress. Omitted input constraints never
mean an approved wildcard. No Full-access suggestion is shown.

### Explicit reuse across private conversations (#2196b)

An additional, separate choice reads: **This agent, this connection and the
displayed operation/inputs, across my private conversations, until revoked**.
It is never preselected. It is available only for a personal owner's ordinary
assistant request, a personal connection, a private audience and an enforceable,
non-sensitive reusable input scope. Org/shared/group/guest/channel and v2
sources keep their existing choices. The card and saved permission both show
whether the scope is one conversation or private conversations.

This is new authoring, not migration. Existing choices retain their IDs, digests,
validity and Exact context, including already-pending reviews. The new choice's
keyed digest binds the whole selected contract, including SameAudience context
and Persistent validity. The human still submits only the server-issued choice
identity/digest, expected revision and idempotency key.

The writer uses optional `ExecutionProposal.v3`, with v1/v2 absent. Old readers
ignore that slot and cannot decide/resume it; their legacy decision guards still
refuse the extension. B2 retains its v2 Machine source/adapter allocation.
Assistant v1/v3 share effect retention, claim, resume, cancellation, receipt and
maintenance logic, with exact-version selectors and bounded indexed sweeps.
Dedupe checks both versions, so readiness changes do not create a second review.
There is no backfill, TTL change, new feature flag or environment variable.

Approval writes the existing ExecutionPermission v1 contract with
`ConversationScope::SameAudience`. Every use still checks the same actor and
agent, exact connection/credential binding, operation/input constraints, private
audience, live ceilings and current task. A new thread has its own key and
occurrence. Reusable permission never makes the original retained effect reusable;
duplicate decisions and uncertain sends retain the existing no-resend fences.

Activation requires `permissions:execution-authoring-private-conversations-v1`
from every serving replica/worker. This is optional for existing enrollment and
v1/v2 authoring. A definitively unready fresh gap retains v1 choices; a readiness
read failure is operational. Existing v3 work never downgrades: decisions,
resumes and sends recheck this capability. Cancellation, expiry, cleanup and
authorized readback remain available.

Deploy all readers/workers and the v3 indexes before exposing the choice through
fleet readiness. For binary rollback, disable effective cohorts and drain/cancel
v3 work on upgraded binaries first; preserve terminal receipts and unknown
outcomes. Saved SameAudience permissions use the already-supported v1 reader and
remain explicitly authorized until revoked or their binding/authority changes.
Disabling rollout does not silently revoke or broaden them.

The transaction rechecks actor enrollment, source/key, live task/audience,
shared behavior and endpoint/connection authority, then commits the card decision,
`ApprovalGrant.execution` and a server-authored resume event together. A queue
failure rolls everything back. Exact decision replay returns the committed result;
a changed revision, digest, choice or idempotency key conflicts. Legacy decision
handlers, Telegram callbacks and legacy decision waiters reject any proposal
before mutation. Unknown envelopes cannot be decided or resumed.

Web source transcript, requester-private group cards and Approvals inbox render
the same request. Reload/polling distinguishes permission saved from operation
queued/running/completed/not-sent/unknown. Grants inspection shows the approved
scope, audience, source and recent dispatch, and uses existing revocation. It
never displays the persistent TTL sentinel as a permission expiry. Mobile and
Telegram remain link-only, with generic notifications through existing configured
channels and no input preview or legacy Allow/Deny action.

### Exact resume and recovery

Resume consumes a typed server event and the encrypted original effect, not a
model-authored retry. It re-enters normal proxy admission/coordinator/remaining
floors and live egress checks with the exact connection, inputs and authority.
A proposal's independent queued → preparing → dispatched CAS admits at most one
provider attempt **for every validity choice**. Reusable permission is never
consumed; Operation additionally uses the existing `execution_use` claim.

A stale pre-send preparation can be recovered after two minutes. A dispatched
attempt with a lost response is unknown and cannot be sent again. Durable result
receipt replay performs no provider call. Bounded sweeps recover lost queue
wakeups, expire pending reviews and deliver outcomes; fair scheduling prevents
old pending reviews from starving queued work. Remaining independent reviews can
pause the retained effect; their decisions retain their existing semantics.
Cancellation, departure, source deletion and sharing narrowing fence queued work
and purge private payloads. Operation permissions receive terminal retention in
the same terminal update; reusable grants remain subject to their live binding.

Group receipt publication includes only HTTP outcome, never raw provider account
data. The original requester can read the bounded encrypted result on their
private permission card; other participants cannot. A private source also receives
the bounded response, with conventional secret fields redacted. Errors before the dispatch barrier report nothing sent; after it,
an unavailable response reports unknown. Neither a receipt nor a TaskRef is
bearer authority. The original caller's normal billing lane remains unchanged.

### Authoring rollout

Deploy readers, dispatchers and authoring support to every replica and drain old
writers before activation. Global/org enrollment additionally requires the
broad-enrollment capability below. No new env var, TTL migration or backfill is required. Old replicas ignore proposal payloads and retain legacy
behavior; they must not serve activated cohorts. Disable enrollment and drain
pending resumes/claims on upgraded replicas before binary rollback. The unchanged
`expires_at` TTL preserves persistent rows. #2195 Part B adds non-assistant callers;
#2196 handles explicitly reviewed legacy mapping.


## Dispatch integration: #2194 Phase B

Phase B uses the same effective person/org/global enrollment, reader/dispatch readiness and live
egress fences. Non-candidate keys and unmarked old jobs/watches/voice requests do
not load execution permissions or prepare their descriptors for this feature. Marked
background work cannot fall back to an unmarked context after disabling enrollment.

Native keys keep the personal, server-held Google/Drive pilot. The engine alone
mints a verified primary plan after its adapter checks. Verification reads retain
their existing gates, are accounted separately and never select/consume the
primary operation permission. The provider send marks dispatch; an admission gap
or denial before that marker is not `OutcomeUncertain`. Failure after a mutating
send remains uncertain. Policy pause, account/epoch and live key checks still
apply. This does not enable policy-reference issuance or extend credential life.

Machine jobs and async watches persist server-owned task lineage alongside their
existing actor, key, conversation, source turn and concrete child identity. Each
provider attempt fences that child plus its root. A job's command approval does
not become permission for arbitrary HTTP bodies. Status, result and cancellation
are independent watch effects and retain the operation contract/lease ceilings.
Recovery of an uncertain submission is observation only, never resubmission.
Background children keep a message root alive; cancellation and hard deadlines
fence future calls. External Machine principals retain their own authority path.

Voice tasks copy the server enrollment hint before runtime startup and retain the
original request's visible-message TaskRef through confirmation continuations and
recovery. Cancellation fences the same root. This applies only to bounded HTTP/MCP
calls from the isolated task thread, not the live session, WS or media stream.

Before B1, a Phase B gap kept the native/proxy/MCP/watch confirmation behavior (including
existing background refusal when no live decision can be requested). Covered
suppresses only the three eligible floors; independent Machine, org, webhook,
exact-use and resource ceilings remain. No Phase B gap creates an execution
proposal or retained resume payload; that source expansion belongs to #2195 Part B.

## Global and organization enrollment

Global/org enrollment uses `permissions:execution-v1` as a rollout control, not
as the authority for an effect. The saved execution permission, its exact
bindings, revocation and claims remain authoritative. No enrollment epoch or
linearizable flag-change fence is introduced. A send racing a flag disable may
finish; off → on can let still-pending marked work continue under a still-valid
permission. It does not reopen consumed/uncertain claims or revive revoked
permissions.

`execution_permission_enrollment` now materializes **personal/org** candidacy.
A global change never scans or rewrites the person population. A bounded local
snapshot of this single flag's overrides supplies global candidacy: refresh by
change stream and five-second polling, at most 10,000 rows. Cold, overflowed,
failed or older-than-15-second snapshots provide no new candidates. The existing
person marker remains an independent candidate hint. Org enables cannot be
inferred from a snapshot without membership information. Known memberships may
narrow global candidates; a candidate is never authority.

The shared resolver preserves the existing personal flag precedence, including
Viewer memberships, legacy role/org-user rows and deny-wins org ties. A personal
false overrides global true; clearing an override restores inheritance. Active
membership means `revoked_at: null`, as in the existing flag model. Enrollment
never gives a Viewer resource proxy rights and does not expand supported lanes.
Live resolution is bounded to 256 memberships and 2,048 applicable override rows,
with two-second query deadlines. An incomplete live read is an operational error,
never an allow. A bounded hint computation may instead omit the hint so that
membership departure and account disable remain possible.

Org override changes enqueue one coalesced, leased job for that org, in the
flag/audit transaction. Scans use batches of 100 and a member-ID keyset cursor;
they include soft-revoked members for cleanup. A 30-second claim and generation
check prevent an expired/superseded worker from advancing progress or publishing
stale hints. Completed jobs repair only their org every five minutes to cover
membership/enable races; there is no population job. Signup retains its original
plain insert: a new person has no scoped enrollment to materialize, and global
candidacy comes from the snapshot.

Lifecycle hint reads use standalone queries with a whole-read deadline, applying
the pending membership/account change in memory. They never run a deadline-bound
hint query in the action's Mongo transaction, because a server query timeout can
abort that transaction even if its error is caught. Successful hints commit with
the action. Overflow, deadlines or other hint-read failures leave the hint and
queue the affected org's repair job. Only genuine transaction/write errors can
fail the action because of this bookkeeping. Disable simply clears the hint
without a lookup.

A coalesced, generation-fenced person repair receipt retains the affected actor
when an account-state read could not identify its orgs, or org deletion removes
the membership rows. The worker visits only explicitly deferred people in batches
of 25, resolves current flags, repairs the hint and schedules their org jobs.
Unresolved receipts retry after five minutes. Org deletion queues these receipts
without doing per-person hint reads inside its 100-member transaction; successful
repair removes the receipt and completed deleted-org jobs are retired. Receipt
generations protect repair writes only, never permission authority.

Admin feature-flag list responses add bounded `execution_enrollment` progress (up to
100 recent org jobs plus `has_more`), including cleared overrides' cleanup jobs.

Auth (API keys, first-party bearer/cookie sessions and MCP) computes candidacy
from the already-loaded person, without new queries. Collaboration admission also
uses its already-loaded membership scope; org-group admission reuses its existing
person read. DB-only services share the AppState-owned process snapshot through
weak database-scoped handles. A missing handle is cold, never an implicit enable.

The bounded live resolver runs before creating a new task root or group request,
before queueing a fresh voice task, and at fresh HTTP/MCP/native admission before
permission preparation or proposals. A stale-positive hint resolving off selects
legacy with no durable marked work; a stale-negative stays legacy. A voice session's
boolean is only a candidate hint; the task request is live-checked before marking.
Device-key voice remains excluded. Existing tasks, proposals, resumes, watches,
Machine children and voice continuations recheck the live effective flag at their
existing barriers and refuse/drain when off. They never downgrade to legacy.
Membership revocation therefore removes org-only participation at the next live
barrier even while an old marker remains. A global or user enable can still make
the person eligible under the normal precedence; resource ACLs remain independent.

Production heartbeats advertise `permissions:execution-enrollment-v1` with the
reader/dispatch/authoring capabilities. Global/org enables and clear-that-enables
require these capabilities plus the B1 authoring-v2 capability below. An org-disable clear is conservatively gated when any
global/org-scoped positive could apply, without scanning members to prove overlap;
a personal-disable clear uses live effective resolution. Explicit disables remain
available when readiness is red.

Rollout:

1. Deploy every serving API replica and worker, drain old writers, and confirm the
   configured readiness roster reports reader, dispatch, authoring and enrollment.
2. Pilot an org or selected users. Watch the admin flag response's
   `execution_enrollment` job progress; new members get scoped hints at membership
   admission, and new people enter a global cohort without a signup hook.
3. Enable the global override when ready. Snapshot refresh normally converges
   within five seconds; cold/overflowed/failed snapshots add no new candidates.
4. To roll back, disable **all effective cohorts** (global=false does not override
   explicit org/user=true), then drain marked work on upgraded binaries before
   downgrading. Do not restart old writers against an activated cohort.

The flag is rollout control. A send racing disable may complete, and off→on can
allow pending marked work under still-valid permission. Exact permission revocation
and claim fences are unchanged. Cache invalidation is neither authority revocation
nor a rollback procedure.

## Permission updates: #2195 Part B1

B1 adds a v2 proposal envelope alongside unchanged assistant-chat v1 proposals.
An enrolled personal API key can request a consolidated permission through the
ordinary UUID/slug/selected-pool proxy or supported MCP HTTP operations, either
standalone LLM gateway, native personal Google/Drive REST/MCP, an admitted async
watch operation, or a human-origin voice task. Assistant Machine service HTTP also participates when the node and server
negotiated `machine_execution_approval_v1` and all replicas meet
`permissions:execution-authoring-machine-v1` readiness. Old nodes keep their
legacy gap path indefinitely. Device-key voice,
delegated/OAuth-client/service-account principals, guests and live sessions are
excluded. Enrollment uses #2246's effective person/org/global flag resolution:
a process-local single-flag snapshot and already-loaded person hints select
candidates, then live single-flag resolution admits or drains marked work. The
registry default is off; production currently enables the flag globally. Every
v2 create, decision, claim, sweep and readback uses the same live resolver.

### External requests and readback

External requests have no server TaskRef and cannot choose Task validity. Parent
keys may choose Operation (default), an enforceable window, or persistent
permission. Login-child credentials offer Operation only, bound to that exact
child: approval cannot silently authorize sibling credentials. Native permission
keys retain their existing personal adapters, resource-policy verification and
90-day credential cap.

Only for callers explicitly opting into NyxID async approval, a bounded
`Idempotency-Key` is a keyed-hash deduplication locator under the full
actor/key/child/source/effect/authority binding. Its downstream meaning is
preserved. Reusing it for different inputs conflicts. Connected callers always
get a per-request occurrence, even when they supply the same idempotency key:
a retry after timeout or disconnect asks again. Without a dedupe key, an async
call also allocates one occurrence; a lost initial handle cannot be matched to
a new, unlabelled request. `x-nyxid-operation-id` retains its existing durable-claim
meaning. Receipt identity expires with retained request metadata; it is not a
permanent provider idempotency guarantee.

Ordinary external proxy (UUID, slug and selected pool), MCP and standalone LLM
requests keep the legacy **connected wait by default**, using the acting person's
notification channel `approval_timeout_secs`. Native permission-key requests use
the same rule. Approval resumes the exact effect on the original connection and
returns the real HTTP response, MCP tool result or LLM stream through its original
adapter. Rejection and expiry retain the legacy error wording. Timeout or
disconnect permanently marks the occurrence `not_sent`, expires the decision and
removes the retained payload: a later approval or sweep cannot send it.
Notifications are claimed once and sent inline immediately after the review
commits; the maintenance sweep is a recovery backstop. Connected waits poll at
the legacy 1000 ms cadence and remain immediately cancellable. Wait and terminal
decision errors include an absolute approval URL based on `FRONTEND_URL`.

Background execution and polling require the explicit HTTP header
`X-NyxID-Approval-Mode: async` (including on MCP POST). Missing or other values
keep the connected wait. `Idempotency-Key` and `Prefer: respond-async` alone never
opt into NyxID background work; they retain their downstream semantics. The
NyxID control header is never forwarded to the provider. Opted-in calls return
`approval_required` immediately; add an idempotency key when retries must join
the same occurrence. Server-owned watches and voice continuations retain their existing
durable source lifecycle. They do not borrow a foreground request's waiter.

For opted-in requests the approval-required response carries the opaque request ID. Poll
`GET /api/v1/approvals/requests/{id}/status` with the **same parent key, login child
(if any), and actor**. The additive `execution` field reports state, a safe failure
code and an encrypted-at-rest bounded result after completion. Wrong credentials
get not-found-shaped responses. Polling cannot enqueue, approve, claim or send an
operation. Review and decision remain first-party-human-only; mobile and Telegram
remain link-only. The web inbox displays the source and available validity
choices, and existing grants inspection/revocation remains available.

### Resumption and source fences

The decision, permission and source-specific durable resume intent commit in one
transaction. Every occurrence has an independent preparation/send CAS regardless
of whether its permission is reusable. Reauthentication, selected connection,
credential replacement epoch, policy/endpoint snapshot, original task/source and
remaining independent floors are checked again. A stale preparer is recoverable
only before its send barrier. Dispatched/unknown effects never resend. Pool occurrences pin the selected
connection on retries and resume, and fence the original pool configuration and
membership before sending; round-robin changes cannot select a new recipient of
the approved effect.
Transaction lock counters are excluded from connection digests; policy,
credential and destination changes still invalidate the binding.

Native pending review is typed and exits before the permission engine's primary
DispatchMark. Verification reads remain separate and cannot consume the primary
claim. Watches park a single status/result/cancel occurrence and reacquire their
lease; a later poll is a new occurrence, and submission never grants its
lifecycle. Voice continuations retain the original human request and task root.
LLM responses stream to a connected waiter after approval. Explicitly asynchronous
output is a bounded retained receipt and is never reconstructed by calling the
model again. Only opted-in requests can continue after the original caller leaves;
status polling never claims or dispatches an effect.

The encrypted retained adapter input, including overhead, remains capped at
256 KiB. Unsupported/raw/multipart/oversized or unrecoverable inputs (including
translating AI-chat pools, source-bearer forwarding and submissions needing automatic watch registration) fall through
before a proposal write, with a metadata-only reason code. Results/payloads have
24-hour retention, and partial or unavailable results are reported explicitly.
Permissions outlive credentials only according to their reviewed validity;
permission does not prolong an expired key, admission deadline or stopped task.

### Rollout

Activation additionally requires `permissions:execution-authoring-v2` across all
replicas, alongside the existing reader, dispatch, v1 authoring and broad-enrollment
capabilities. Global activation is already live, so v2 becomes applicable to that
cohort as soon as all replicas advertise the new capability.
Deploy readers, source runners, indexes, refusal paths and web DTOs first; drain
old writers before enrolling a cohort. Unenrolled requests perform no new
preparation or reads. V1 and legacy card decisions are unchanged, and legacy
handlers refuse any proposal before mutation. Disable and drain pending v2 work
on upgraded replicas before rollback. The existing `expires_at` TTL and
persistent-grant sentinel are unchanged; there is no backfill or new environment
variable.


### Machine service HTTP proposals (Part B2)

The original Assistant job's signed node/runtime, context revision and authority
lease bind one bounded HTTP effect. Approval resumes only that service request;
it never reruns the command, launches/restarts a job, renews a lease or lends the
job creator's permission to an external Machine principal. Original actor/task,
service declarations, machine ceilings, independent confirmations and org
instruction receipts remain mandatory at dispatch. Lost job/source authority
blocks both unsent work and result delivery.

Connected waiting remains the default, capped by the original gateway's 120-second
header deadline, the person's approval timeout, job/task and key expiry. Upload
and preparation consume that window. If fewer than 15 seconds remain when a gap
is found, NyxID returns the legacy `ApprovalFailed` timeout-shaped error with an
absolute approval URL and writes no card or proposal. Disconnect/timeout while
waiting makes the occurrence permanently unsent and undecidable. Approval returns
the real provider response on the same connection. A failed final source or
cancellation check before provider handoff records `not_sent`. A failure after
handoff may be uncertain and is never replayed automatically.

Only negotiated `X-NyxID-Approval-Mode: async` opts into background approval.
`Prefer: respond-async` and `Idempotency-Key` retain their downstream meaning and
never opt in by themselves. An async caller may use Idempotency-Key to join the
same occurrence under the exact job/actor/runtime/lease/effect binding. Connected
retries receive new occurrences. The NyxID mode header is not sent downstream.

The local job gateway exposes `GET /_nyxid/approvals/{opaque_id}/status` after
negotiation. The node signs every readback; only that live job/runtime/lease and
original actor may retrieve the retained result. The handle is not bearer
authority, polling never dispatches, and no assistant key is sent to the node.
Readback stops when the job finishes, is cancelled or loses authority.

Result delivery checks live authority once before headers and then at most once
per second or per 1 MiB released, whichever comes first. The periodic check is an
indexed aggregation, not a transaction per chunk. Local cancellation is immediate;
already delivered bytes cannot be recalled. Legacy/non-B2 response delivery adds
no database commands. Retained requests still have the 256 KiB envelope cap;
raw, multipart, oversized, git and unrecoverable effects stay on legacy prompts.

The server change is inert for nodes that do not advertise the capability. CLI
support ships separately when the owner releases the node binary; no forced node
upgrade or version bump accompanies server deployment. All API/worker readers
must support the new closed Machine source/adapter variants before readiness
allows authoring. Keep existing B1 sources available during that rollout. Before
rolling back to B1-only binaries, drain/remove every Machine-variant proposal row,
including terminal rows, since old typed readers cannot deserialize that variant.

## Legacy mapping and admission diagnostics (#2196a)

There is no data migration, backfill, index, TTL change or new environment
variable. The following records continue through their original evaluators.
Missing account, input, audience or lifetime bindings never become authority by
inference. An explicit upgrade requires a new human-reviewed server-built
execution contract; it does not relabel or delete the old record.

| Existing record | Retained guarantee |
| --- | --- |
| Service / Agent / Key allowlists and conversation-key mirrors | Access ceilings and live owner/org/share/platform ACLs; no affirmative operation consent. |
| Scoped operations / InputRules | Compiled method/path/query/body and contract ceilings on each prepared effect; permissions cannot widen them. |
| Legacy ApprovalGrant in grant mode | Recorded service/requester/scope, finite expiry and revocation. Absent `org_scoped` remains personal; true retains deliberate org-member reuse. It satisfies only its own approval layer. |
| Per-request service approval | One original waiter, expiry and cancellation; no reusable grant or inferred exact provider-idempotency promise. |
| Exact approvals and action cards | Exact bindings, one-use claims, expiry and terminal receipts. Management cards amend only their stated ceiling after the authorized decision. |
| Shared Always / Once / Skip | Actor connection, conversation/key, operation, share/behavior/authority bindings and original claim/skip rules. Always is not cross-conversation consent; Once never becomes Always. |
| Full / Auto / Ask | Prompt defaults only. A matching execution permit satisfies only its named eligible floors. Explicit deny, org instruction, webhook, Machine/login and delegation requirements stay independent. |
| Native permission keys | Immutable personal policy, adapter resource verification, live pause/revision/revocation and at most 90-day credential life. Verification is separate from the primary write claim. |
| DurableOperationGrant | Original scheduled owner/key/connection/endpoint, finite validity, input limits, quotas, replay policy and execution journal. No counter reset or persistent conversion. |

Legacy requester identity remains `AuthUser::approval_requester_id()`; it is
not retrofitted to the new contract's explicit API-key principal. In particular,
a general key's legacy token-subject binding must not be mistaken for a per-key
execution contract.

Billing remains on the original resolved credential/execution lane. Shared
actor-owned credentials remain the actor's; platform-key usage is billed to the
acting person. A permission or approval never substitutes a billing owner.

Operational admission telemetry is metadata-only: a single aggregate
`execution_permission_decision` event per request context, including prepared
MCP re-entry and connected-wait recursion. It uses the existing structured server
log pipeline, not product analytics (whose consent lookup would add commands).
There is no synchronous per-request audit write, no diagnostic database lookup,
and no event/extra command for an unmarked coordinator. Legacy outcomes are
logged at debug; covered, requestable-gap, hard-denial and operational-error
outcomes remain info. Diagnostic state tolerates mutex poisoning, including
when an abandoned request is dropped during unwinding. Existing transactional
requested/saved/denied/dispatched audits retain their original coupling.

The fields are the server operation UUID, route, decision, closed cause, stage and
whether this is a resume. Coverage also records the closed lane, validity and
floor enums and whether an operation claim is required. No input values, paths, headers, credential identifiers,
provider bodies or text enter the event. Decision causes distinguish:

- `covered`: `permission_covered`.
- `requestable_gap`: `permission_missing`, `subject_changed`,
  `connection_binding_changed`, `context_changed`,
  `operation_or_inputs_outside_scope`, `validity_not_covering`,
  `additional_floor_required`.
- `hard_denial`: explicit policy/ceiling, revoked or invalid permission, unsupported
  version, used claim, dead task, resource denial or ineligible decider. A marked
  resume that no longer covers refuses; it cannot fall back to Full.
- `operational_error`: unavailable authority/readiness/enrollment resolution,
  candidate overflow, required resource verification, uncertain outcome or
  `admission_abandoned` when a started admission future is dropped.
- `legacy`: a stale-positive hint resolved off, unsupported source/transport, or
  no eligible floor. This is not a denial of the ordinary legacy path.

The event describes **admission**, not provider completion. Independent floor
prompts and later live fences remain possible; provider outcome/attempt evidence
comes from existing dispatch and result receipts. Candidate failures are not
logged individually or mistaken for the aggregate result when another whole
permission covers. The rollout flag remains a hint plus live admission check,
not authority; disable drains marked work without retroactively undoing a send.

At rollout, an ACK_HEADER retry retains the ingress decision to use an existing
legacy card. This suppresses only new proposal authoring: saved permissions,
ceilings and live drain checks still run, and the normal legacy gate must match
and consume that exact card. A forged ID or changed call cannot authorize a
send. Shared Once and Always decisions are observed under the original exact
actor/key/conversation/connection/operation/behavior/expiry binding. Observation
issues no execution permit: the normal shared gate still checks live authority
and claims Once before egress. A consumed HTTP Once decision cannot authorize
another occurrence; changed consent must pass the original live checks.

The remaining #2196 slices are verified-owner channel source/resume/result
delivery and integrated B2 evidence. Existing assistant choices, including
persistent, still bind one conversation. Slice (b)'s separate, explicit v3 choice
supports private cross-conversation reuse only under its readiness gate and
eligibility rules above. Channel/automation/guest and live voice remain outside
the supported source list above. No migration diagnostic implicitly enrolls
them or changes their audience.
