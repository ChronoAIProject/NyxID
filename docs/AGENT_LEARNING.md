# Agent learning

Status: design for L1. This document is normative; implementation follows in
separate reviewed phases.

## Scope

L1 lets a personal or org-owned agent propose Ornn skills from eligible,
completed work. Learning is opt-in per agent, default-off, and controlled by
the `assistant:agent-learning` flag. It never grants authority and never
publishes or attaches a skill without an explicit human approval.

## Review decisions

- L1 ships in two PRs. PR-1 contains storage, flag checks, consent,
  enrollment, evidence selection, redaction, one-shot analysis and encrypted
  pending proposals. PR-2 contains the review UI, approval card, Ornn
  publication and B2 pin saga. Both remain dormant until the flag is enabled.
- PR-2 verifies Ornn's live contract through `ornn-api` (including
  `getMyOrg`, `listMyOrgs`, `transferSkillOwnership` or the published OpenAPI
  equivalent). If an org-owned skill with equivalent ACLs is supported, L1
  publishes to that org. Otherwise it stops with
  `owner_binding_unavailable` and never uses a maintainer's personal owner.
- `enabled_by` on the config row is the threshold-run billing actor for org
  agents. Losing maintain access or membership pauses threshold runs with
  `owner_access_lost` until another maintainer re-enables learning. Manual
  runs bill their triggering maintainer; personal runs bill the owner; the org
  wallet is never charged.
- The approved defaults are threshold 15 (range 1–50), rejection fingerprints
  retained for 180 days with 256 per agent, private visibility only and no
  public L1 publication.
- `learning_epoch` is an additive serde-defaulted field set by
  `create_thread_for`. Legacy and absent values mean the conversation is not
  enrolled.
- PR-2 verified Ornn OpenAPI 0.18.0 through the catalog service: skills can be
  created or transferred only to a person (`newOwnerUserId`), not an org. The
  publisher therefore refuses org-owned proposals with
  `owner_binding_unavailable` before any Ornn mutation. Personal publication
  uses the approving person's identity, a fixed operation allowlist and a
  NyxID-built private ZIP; readback must match its SHA-256 before the existing
  B2 pin path runs.
- Publication records a local operation id and package hash. A failed create
  is reconciled by the approving person's private search and proceeds only
  after exactly one hash match; it never blindly retries an ambiguous create.
  One owner card binds the proposal, revisions, fingerprint and operation id
  for the publish-and-pin saga.
- Review implementation stores the external operation and lease beside the
  encrypted proposal. The card is consumed and the lease claimed before Ornn
  egress; reconciliation verifies the exact private owner, generated operation
  name and ZIP hash. The B2 pin and terminal proposal state commit together.
  Personal HTTP review endpoints require a first-party human session, while
  NyxBot exposes bounded status, run and metadata-only listing.
- Enrollment first checks pilot override rows. A dormant flag or actor outside
  a pilot returns without learning-configuration or org-membership reads; once
  configuration exists, the request access snapshot is reused for org checks.

## Invariants

- Existing agents, threads, skills, grants, scopes and memories keep their
  current behavior when learning is absent or disabled.
- Evidence is read from existing retained history at analysis time; NyxID
  stores evidence identifiers and bounded metadata, not a second transcript.
- Proposal bodies are private encrypted data and untrusted text. They never
  execute tools, scripts, or permissions.
- Personal evidence stays with that person. Org evidence requires explicit
  opt-in by each participating member and never exposes one member's private
  thread to another member.
- Publication and B2 pinning are separate from analysis, but one owner review
  action may authorize both for the reviewed proposal.

## Sections

The following sections define the data model, eligibility, jobs, analysis
contract, redaction, Ornn ownership, review surfaces, failures, tests and
delivery phases.

## Data model

Learning state is separate from `AssistantAgent`, beside its `skills` and
`grants`, so older writers cannot erase it. Missing documents mean disabled
learning. All identifiers are strings and all timestamps use the repository's
UTC BSON helpers.

`assistant_agent_learning` has one row per agent:

- `agent_id`, polymorphic `owner_id`, `config_revision` and `learning_epoch`;
- `enabled` and `threshold` (1–50, default 15);
- `last_success_cursor` and `last_success_run_id` for a stable completed-turn
  high-water mark;
- bounded counters and `last_run_at`, `last_success_at`, `last_error_code`;
- a lease owner, expiry and fencing number for the background worker.

`assistant_agent_learning_members` is keyed by `(agent_id, member_user_id)`.
It stores an org member's explicit `opted_in` state, revision and epoch. A
personal agent has no member rows: its person owner is the eligible principal.
Maintainer access never implies evidence opt-in.

`assistant_agent_learning_runs` stores the job identity, mode (`threshold` or
`manual`), triggering actor, owner binding, config/fence revisions, bounded
eligible thread/turn IDs, the input digest, status, attempt count and lease
metadata. It contains identifiers and counts only, never excerpts.

`assistant_agent_learning_proposals` stores one encrypted proposal package,
encrypted with `AppState.encryption_keys` before insertion. Plaintext fields
are limited to agent/owner/run IDs, status, revision fences, a keyed
proposal fingerprint, bounded evidence IDs, timestamps and failure metadata.
The encrypted envelope contains the draft skill and redacted rationale. Its
custom `Debug` output exposes only IDs, status, sizes and counts.

`assistant_agent_learning_rejections` stores a bounded keyed fingerprint per
agent and proposal shape, with no proposal text. A unique `(agent_id,
fingerprint)` index makes rejection deduplication safe. Entries expire after
180 days and the newest 256 entries per agent are retained; a rejection is
also removed when the owner explicitly starts a new learning epoch. These
limits prevent unbounded growth without retaining rejected private material.

`assistant_agent_learning_skill_roots` records the provenance of each L1
publication: agent, polymorphic owner, Ornn GUID/version/hash, publication
operation ID and the approval/config revisions. It is the only valid source
for an `improve` proposal's `base_skill`; a person-attached B2 skill is never
treated as an L1 root merely because it is currently pinned. Removing a root
from B2 marks its provenance inactive and prevents later improvement proposals.

Learning revisions never alter `AssistantAgent.skills_revision`. Approval
uses the existing B2 exact-pin path and its optimistic revision. Legacy agent
documents and conversations deserialize with learning absent and unchanged;
the separate collections are additive and have no effect unless the flag is
enabled.

## Enrollment and eligible evidence

Learning is cohort-based. When learning is enabled, new conversations snapshot
the current `learning_epoch`; conversations created before that epoch are not
enrolled retroactively. A thread created while the agent or member is not
eligible has no learning enrollment. Starting a later epoch is an explicit
owner action and only affects new threads.

An eligible evidence item is a retained conversation thread in the agent's
enrolled cohort with a completed, successful owner turn after the run cursor.
The item records the conversation ID and turn ID; the analysis reader fetches
the current history only for that bounded set. It does not create a transcript
copy. A threshold counts distinct eligible threads, not messages.

For a personal agent, the principal is the person in the agent's owner
binding, and only that person's own non-guest turns qualify. Existing NyxBot
threads are not retroactively eligible; the owner must enable a new epoch,
after which newly created NyxBot threads carry that epoch.

For an org agent, the principal is the acting member who owns the private
thread. The thread must have been enrolled while that member's
`assistant_agent_learning_members.opted_in` epoch was active. Opt-in and
withdrawal are self-service; a maintainer cannot change another member's
consent. Withdrawal or loss of membership invalidates that member's
unpublished evidence and proposals, including a run that was already queued.
An in-flight worker may finish only the bounded read it already admitted, then
must recheck consent before storing anything. Live org membership and
`can_proxy()` are rechecked before every read, job transition and approval.

The following are always excluded: guest and channel-guest turns, channel or
external event content, automation/trigger turns, another member's group
messages, other members' hidden threads, attachment bytes, raw screenshots,
and tool output containing credentials or secrets. A group thread is eligible
only when the complete sampled turn has a single opted-in member as both
prompt author and owner of the reply; mixed-member context, hand-offs and
member messages make that turn ineligible rather than merely stripping text.
An opt-in never grants a maintainer access to that member's private transcript.

Only completed assistant replies paired with their owner prompt are used.
Active, stopped, failed, cancelled, expired or retention-deleted turns are
skipped. Retention is authoritative at analysis time; missing history reduces
the evidence set and never blocks unrelated conversations.

The cursor is ordered by `(reply.created_at, conversation_id, turn_id)` and is
advanced only after a successful analysis result is durably recorded. Ties are
deterministic, so retries cannot silently omit a thread. A run first captures
a finite batch of at most `threshold` distinct threads plus 5, and stores both
the inclusive candidate watermark and selected evidence IDs. Sampling happens
before advancing that watermark, so skipped candidates are not lost: they
remain eligible on the next run. The helper input must remain within its
12,000-character cap.

## Job lifecycle

The scheduler considers only enabled rows when `assistant:agent-learning` is
enabled. It creates a threshold run after the number of new eligible threads
reaches the configured threshold. A human or authorized NyxBot `run now`
request creates a manual run immediately; it does not bypass enrollment,
redaction, bounds or review.

Creation, claiming and completion use compare-and-set updates on
`config_revision` and a monotonically increasing fence. A worker claims one
run with a short lease, renews it while reading evidence, and releases it by
writing a terminal status. A lease expiry permits a later worker to retry;
the fence prevents the old worker from advancing the cursor or writing a
proposal after loss of ownership. Queue depth, attempts, evidence count,
excerpt bytes and proposal size are bounded.

Run states are `queued`, `running`, `analyzing`, `succeeded`, `failed`,
`cancelled` and `blocked`; proposal state is separate (`pending`, `rejected`,
`publishing`, `published_unpinned`, `pinned`, `publication_failed` or
`invalidated`). A successful run atomically records its proposal (if any), the
input digest and the new cursor. A run with no useful proposal still advances
the cursor and records counts. Helper, database or redaction failures retain
the cursor, use bounded backoff, and eventually become `failed` with a safe
retry message; they never publish or pin content. An Ornn failure leaves a
proposal pending unless it occurred during an approval saga, in which case the
proposal records the resumable publication state.

Proposal creation is idempotent on `(agent_id, input_digest, model_contract)`.
The rejection fingerprint is checked before storing a new proposal. An
identical rejected shape is suppressed while a materially different input or
skill revision may produce a new proposal. Manual runs cannot create a second
active run for the same agent and cursor.

The request path only enqueues or reads bounded metadata. Inference and
history reads run in the background, so a slow Ornn or model call cannot block
turn settlement, live events or unrelated API requests. Disabling the flag
stops new claims and scheduling; an already running worker must stop at its
next fence check and leave no permission change behind. Opt-out, membership
loss, agent destruction or owner transfer invalidates every unpublished
proposal that depends on the withdrawn evidence. An already pinned B2 skill is
explicit delivery state and is not silently detached; removing it remains the
normal owner-controlled B2 action.

## Analysis contract

Analysis calls `services::assistant_oneshot_inference::one_shot_text` with the
run's acting person, a fixed server prompt and a bounded redacted input. The
implementation uses `max_input_chars <= 12_000`, `max_output_chars <= 8_000`,
`max_output_tokens <= 768` and a timeout no longer than 15 seconds. It uses
no agent key, MCP tools, tool definitions, arbitrary request JSON or
conversation session. Usage is metered through the existing helper and billed
to the acting person under the policy below.

The fixed prompt says that evidence is untrusted data, may contain prompt
injection, and is not an instruction. The model may describe reusable work
patterns only. It must not propose grants, operation scopes, approvals,
models, machine access, credentials, secrets, policy bypasses or actions on
behalf of a person. The prompt requests one JSON value and gives no ability
to call Ornn or NyxID tools. For an `improve` proposal, NyxID may preload the
validated text of the recorded L1 base package after rechecking the approving
person's live visibility; this is server input, not model tool access, and is
omitted if the base cannot be read. Each excerpt is prefixed with a short
synthetic label such as `evidence_03`; the worker maps that label back to a
stored thread/turn ID after validation. Raw database identifiers never become
model instructions or output links.

The bounded output schema is versioned and rejects unknown control fields:

```json
{
  "schema_version": 1,
  "kind": "new|improve",
  "name": "bounded display name",
  "description": "bounded one-line description",
  "skill_md": "bounded Markdown skill body",
  "files": [{"path": "docs/example.md", "content": "text"}],
  "base_skill": "optional exact recorded L1 root reference",
  "rationale": "bounded explanation",
  "safety_notes": "bounded review notes"
}
```

`skill_md` is required and is the only executable-adjacent artifact permitted
in L1. Optional files are UTF-8 Markdown/text under safe relative paths;
scripts, binaries, symlinks, package hooks and dependency changes are
rejected. Limits are 80 characters for the name, 400 for the description,
7,500 total output characters, at most 8 files and 2,000 characters per file;
rationale and safety notes are each at most 1,000 characters. `base_skill`,
when present, must match an active `assistant_agent_learning_skill_roots` row,
the current exact SHA-256 and the current agent owner; a proposal cannot
improve an arbitrary attached B2 skill.

The server validates JSON, UTF-8, size, path and base-pin constraints, then
computes a keyed fingerprint over the canonical draft plus agent and base
revision. It does not trust model claims about Ornn IDs, visibility, versions
or hashes. A malformed, oversized or policy-shaped response becomes a failed
run with no proposal.

At publication NyxID, not the model, builds the Ornn ZIP with one generated
root directory, a server-normalized `SKILL.md` frontmatter and the approved
text files below that root. The package version and hash come only from Ornn's
validated response. Frontmatter cannot request tools, runtime hooks,
dependencies, sharing or a different owner.

The review renderer marks every field as untrusted generated guidance. It
escapes Markdown/HTML, disables links that could execute or exfiltrate data,
and never renders a proposal as system instructions. Proposal text is not
inserted into an agent turn until a published B2 pin is explicitly attached.

## Redaction and privacy boundary

Evidence extraction starts from typed message text and bounded turn metadata.
It excludes attachments and tool-result bodies by default. If a future
extractor admits a tool result, it must first pass the existing credential and
secret redaction utilities, including `telemetry::scrub` and the action
description/request-field allowlists; an uncertain result is dropped. Those
utilities are not by themselves a complete free-text detector, so L1 adds a
dedicated conservative pass and tests it against bearer/API-key prefixes,
cookies, PEM blocks, high-entropy token shapes, credential-named JSON fields
and provider-specific secret forms. A detector failure drops the excerpt.

Redaction runs before truncation and before hashing. It removes bearer/basic
authorization, API-key/token prefixes, URLs with queries, email addresses,
UUID-shaped identifiers, private-key material, cookies, headers marked
sensitive, credential-shaped JSON fields and known NyxID/Ornn secrets. It
also drops JSON fields whose names identify credentials rather than trying to
mask their values. The resulting excerpts use stable labels such as
`[SECRET_REDACTED]` and contain no raw attachment bytes. Only synthetic
evidence labels enter the model input; real IDs remain out-of-band metadata.

Each excerpt has a hard character and line bound. The complete helper input,
prompt and output have independent byte/character limits; truncation occurs
at UTF-8 boundaries. Evidence IDs, counts, actor kind and redaction counters
are metadata kept beside the encrypted proposal, never interpolated as
higher-priority instructions.

The personal owner is told which of their threads were sampled. An org member
must opt in through a clear notice that their redacted excerpts may inform an
org-owned proposal. Other members receive only a generated draft and bounded
evidence metadata; they cannot open the contributing member's private thread.
The evidence reader rechecks live ownership, membership and retention on
every fetch. Revocation stops future reads and pending jobs at the next fence.

Prompts, excerpts, model output, encrypted payloads and provider errors never
appear in logs, audit records or `Debug`. Audit stores only actor, agent,
owner kind, run/proposal IDs, status, counts, revisions, sizes and reason
codes. The fingerprint uses the existing keyed material-fingerprint helper,
not a plain digest of private text.

## Ornn publication and identity

Analysis never contacts Ornn. On review approval, NyxID uses the existing
`ornn-api` catalog service and the approving maintainer's live person identity
(`require_identity_assertion`); a shared master credential, anonymous request,
cached cross-person response or member fallback is forbidden. The publisher
uses the same ordinary proxy path as B2 and an explicit, fixed operation
allowlist; proposal fields can never select a URL, service, header or
credential.

For a personal agent, the publish owner is that person's owner binding. For an
org agent, the publish owner is the org binding, while the approving person is
the acting identity and must have live Admin/Member write access. NyxID must
send an explicit polymorphic owner target only if the Ornn API supports
org-owned skills with the same ACL semantics. If it does not, approval stops
in `owner_binding_unavailable` and never publishes the skill under the
maintainer's personal account; the UI explains the blocked owner binding.

The concrete package flow is: `POST /api/v1/skill-format/validate` with a
bounded `application/zip`, then `POST /api/v1/skills` for a new private skill,
or `PUT /api/v1/skills/{guid}` with the same ZIP for an improvement version.
The operation policy permits only these routes plus the metadata, version,
closure and download reads needed to verify the result. Visibility remains
private by default; L1 may send only the fixed `{"isPrivate":true}` settings
update when the Ornn contract requires it, and cannot make a skill public.
NyxID validates the returned GUID, exact version, dependency closure and
SHA-256 through the same B2 preview/integrity checks before recording a root.

Each external attempt carries a NyxID operation ID in the local saga record.
If a timeout leaves the result ambiguous, the worker reconciles by querying
the approving identity's private skills for the exact operation fingerprint
and package hash. It may continue only after finding one unambiguous result;
otherwise it stops as `publication_failed` with a human retry action. It never
blindly creates a second skill or claims exactly-once behavior for an external
API that has no idempotency key.

Public publication requires a separate future owner action and is not an L1
option. If Ornn cannot represent the polymorphic org owner, approval stops in
`owner_binding_unavailable` before the first mutation; it never publishes
under a maintainer's personal account.

## Review and management

The Assistant agent detail page adds a Learning tab. A maintainer can enable
or disable learning, choose a threshold from 1–50, start a bounded run, see
the eligible count and last-run state, and open pending proposals. An org
Viewer sees the tab and status but cannot change configuration, consent,
proposals or skills. An org member's consent control is available only to that
member; it states that redacted excerpts may inform a shared org skill.

The proposal editor shows generated name, description, Markdown and text
files, rationale, safety notes, bounded evidence identifiers and the base
skill provenance. Evidence links open through the ordinary private-thread ACL
and are checked again on every request; a maintainer sees metadata when the
contributor has withdrawn or lost membership, never a copied excerpt.
Generated fields remain editable untrusted text. The UI must show the exact
owner binding, run/config revision, model contract and publication destination
before approval.

Approve is one explicit human action card bound to `(agent_id, proposal_id,
owner_id, config_revision, agent.skills_revision, proposal_fingerprint)` and
an operation ID. The card authorizes both the fixed Ornn publication saga and
the B2 pin update; no second card is raised and skip-destructive settings do
not bypass it. Before each external mutation NyxID rechecks live maintain
access, owner binding, proposal status, exact base/root pins and the current
agent skill revision. A stale card expires with a conflict and cannot widen
authority. Reject deletes the encrypted body in the same transaction as the
terminal status and records only the keyed fingerprint and reason code.

NyxBot exposes status, `run now` and bounded proposal-list tools. It can use
them only for an agent whose owner or org binding the acting person may
maintain; it cannot edit, approve, publish, pin or change another member's
consent. Specialist threads can request learning through the existing
permission flow, but the request is decided by NyxBot and still needs the
human approval card. Every tool accepts an immutable agent ID and explicit
org target where applicable; mutable names never select an owner.

Learning ingestion and skill delivery are separate controls. Disabling
learning stops new evidence jobs and proposals but does not detach an already
approved B2 skill. Removing a B2 pin takes effect immediately through the
existing skill-management path.

## Billing and ownership

Every analysis invocation supplies the acting person to
`one_shot_text`. Personal-agent runs are therefore billed to the personal
owner. An org-agent run is billed to the person who enabled or manually
triggered that run, using `BillingOwnerResolver::resolve_for_execution`; the
org wallet is never charged. That actor paid for the inference and explicitly
accepted the evidence use, while the org remains the publication owner.

A threshold worker records the person who admitted the run when it claims the
lease. If that person loses org access before inference, the run is cancelled
without a provider call. A later maintainer may start a new run and becomes
the new billing actor. Billing metadata contains service/model identifiers,
quantity and the run ID only; prompts, excerpts, output and credentials are
never copied into usage or ledger descriptions.

Analysis is optional metadata work and never delays a user turn. A missing
platform model or unavailable personal service records a bounded
`inference_unavailable` reason and leaves the cursor unchanged. The helper's
existing service resolution, live ACL, credential and cancellation semantics
remain authoritative; L1 does not add a master key or a fallback identity.

## Failure, retention and revocation

All worker and approval errors map to stable reason codes with bounded client
text: `learning_disabled`, `owner_access_lost`, `consent_withdrawn`,
`evidence_expired`, `inference_unavailable`, `invalid_model_output`,
`proposal_conflict`, `owner_binding_unavailable`, `ornn_unavailable`,
`integrity_mismatch` or `publication_ambiguous`. Raw provider errors and
response bodies are discarded. A retry may re-read retained history only
after the same live ACL and consent checks; it never resurrects a withdrawn
proposal.

Retention is evaluated at evidence fetch and approval. A deleted or expired
conversation is removed from the candidate set, and any proposal whose only
evidence is no longer readable becomes `invalidated` with its encrypted body
deleted. If one of several evidence items expires, the review page shows the
remaining identifiers and requires the maintainer to re-run before approval.
The rejection collection has a MongoDB TTL index. Terminal-run and proposal
retention cleanup is a bounded maintenance task and is part of the delivery
work that follows the dormant PR-1 worker; no cleanup can expose evidence.

Changing the agent owner, destroying the agent, removing an org member or
disabling the owner account fences queued and running work. The worker checks
the fence before every read, model call, write and external mutation. Pending
proposals are invalidated transactionally with the ownership change. A skill
already published and pinned through B2 remains an explicit owner decision;
L1 does not silently remove it or claim that learning revocation retracts
content already shared in Ornn.

An approval saga is resumable only from its recorded state and exact operation
ID. A published-but-unpinned result is visible as such and cannot be silently
replaced; the maintainer may retry the B2 pin after a fresh revision check.
If publication succeeds but verification is ambiguous, NyxID blocks further
mutations and presents the operation ID for reconciliation. It never exposes
the package or a downstream credential in an error, log, audit row or
`Debug` implementation.

## Rollout and compatibility

`assistant:agent-learning` is a per-owner, default-off feature flag resolved
through the existing live feature-flag service. With the flag off, settings,
consent, run creation, proposal review and publication all refuse with a
bounded `feature_not_enabled` message; no evidence is read and no worker is
claimed. Existing B2 skills, agents, conversations, grants, scopes and
ordinary NyxBot behavior are unchanged.

Deploy every backend and frontend replica containing the schema, ACL checks,
worker and flag handling before enabling the flag for any owner. New rows are
in separate collections and old binaries safely ignore them, but enabling
before all replicas are upgraded could make a write appear to succeed on one
replica while another cannot enforce the review contract. The UI hides the
Learning controls when the flag is off; direct API and NyxBot calls still
return the same refusal.

Rollback first disables `assistant:agent-learning`, waits for running leases
to expire or be fenced, and only then rolls back binaries. Existing approved
B2 pins remain attached until an owner removes them. Re-enabling after a
rollback requires the full deploy ordering again; epochs and proposal state
are preserved and rechecked rather than replayed blindly.

## Test plan

- Legacy agents and conversations, missing learning rows, flag-off settings,
  NyxBot, B2 delivery and ordinary turns behave byte-for-byte as before.
- Personal enrollment excludes pre-epoch threads, guests, channel/event,
  automation and failed/stopped turns; threshold counts distinct threads and
  cursor retries do not skip ties or sampled candidates.
- Org consent is self-only; maintainers cannot opt in another member. Live
  membership and `can_proxy()` removal fences reads, queued/running jobs,
  evidence links and approvals. Mixed-member group turns and private-thread
  cross-reads are refused, and personal/org agents cannot cross-contaminate.
- Query-count tests prove no learning reads on unrelated requests and bounded
  candidate/history reads per run. Lease expiry, fencing, duplicate run
  creation and owner transfer are covered with concurrent workers.
- Redaction fixtures cover each detector, uncertain tool output, UTF-8 and
  byte limits, prompt injection, attachment exclusion and metadata-only
  audit/log/Debug output. Encrypted proposal storage is unreadable without
  `EncryptionKeys`; reject and invalidation delete the body.
- One-shot helper tests cover bounded prompt/input/output, timeout,
  unavailable models, person billing, org billing actor and no agent-key or
  tool dispatch. Provider cancellation never blocks turn settlement.
- Schema tests reject unknown fields, unsafe paths, scripts, binaries,
  oversized packages, arbitrary B2 bases and malformed model JSON; fingerprints
  deduplicate rejected shapes without retaining text.
- Ornn contract tests assert the fixed validate/create/update/read routes,
  private visibility, actor identity propagation, org-owner capability refusal,
  hash/closure verification, ambiguous-result reconciliation and no duplicate
  publication. B2 pin conflicts leave `published_unpinned` for review.
- UI tests cover flag-off hiding, personal/org roles, consent, threshold,
  evidence ACL, edit/reject/approve cards, stale revisions, failure states and
  the separation between learning ingestion and attached-skill delivery.

## Shippable PRs

1. **PR-1 — dormant ingestion:** add the separate learning collections,
   encryption and indexes, feature-flag checks, owner/org ACL helpers,
   self-only consent, enrollment at `create_thread_for`, bounded redaction,
   leases/fences, cleanup, one-shot analysis and encrypted pending proposals.
   No review or Ornn mutation is enabled yet.
2. **PR-2 — review and delivery:** add the human proposal editor, evidence
   links, reject/delete, one-use approval cards, NyxBot status/run/list tools,
   the verified private Ornn publisher, org-owner contract check and B2 pin
   saga. Approval and publication land together so review has a useful outcome.

Each PR is independently deployable with the flag off and has a rollback that
disables the flag before binaries are reverted. Neither PR changes grants,
operation scopes, approvals, memory visibility or existing B2 pins.
