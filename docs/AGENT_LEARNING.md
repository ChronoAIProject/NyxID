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
`max_output_tokens <= 768` (the shared helper grants reasoning models a bounded
1,024-token allowance) and a timeout no longer than 15 seconds. Titles and
learning share the admin-configured utility service/model, live platform ACL,
metadata-only diagnostics and bounded non-billable fallback contract in
[Thread titles](chat/08-nyxagent-engine.md#thread-titles). It uses
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

### Publication recovery and rollout

The review card shows copy for the fixed failure code. NyxID stores and audits
the durable publication stage for recovery.
Ornn problem responses contribute only their root `code` and HTTP status;
NyxID never stores Ornn detail text. Authentication and write permission
refusals are distinct from package validation, interface changes, version
conflicts, unavailable Ornn, and an uncertain dispatch. A verified Ornn version
is checkpointed before the agent pin transaction; a failed pin reports
`published_unpinned`, and retry verifies and pins without another publication
request. Read denial, missing authentication, and transport failure have
separate failure codes; an unavailable validator is never reported as an
invalid package.

For a definitive refusal before Ornn mutation, NyxID clears the started flag
only if that operation has never had an uncertain dispatch. The same
transaction releases its reserved target, allowing a revised draft to claim
that version. A later retry of the first draft can claim it again only if
the target remains free. Invalid package, dependency, name and interface
failures require the owner to discard the draft and ask NyxBot for a revised
one; they do not offer a write retry. A version conflict after dispatch keeps
the barrier and permits only read-only checks until publication is resolved.
Once an attempt is uncertain, every later click
reconciles the exact version read-only. An absent version is not proof that a
delayed request cannot still land. If the version later appears, NyxID checks
the private owner, empty sharing ACL, name, version, metadata hash, empty
dependency closure, and downloaded ZIP hash before attaching it. A version
with different bytes is a conflict and is never adopted. An expired unconsumed
card can be renewed for the same operation; changing package bytes requires
a new review revision while the old operation is conclusively non-effective.

Updates reserve the target `{skill GUID}:{version}` before publication. Another
operation targeting that pair is refused as `target_busy`. A reservation may
be released only with a proposal transition while the operation has not
started, has no live lease, has no verified checkpoint, and has never had an
uncertain dispatch. Uncertain and landed reservations stay. Create operations
use Ornn's unique name constraint and the same reconciliation-only rule after
uncertainty. These fences cover NyxID writers; an external Ornn writer or a
stalled request that later regresses Ornn's `latest` pointer remain residual
risks. Legacy uncertain drafts can recover only when their exact private
version is found; NyxID cannot authorize another write without authoritative
evidence that the old request can no longer mutate.

Deploy this change with a writer drain:

1. Disable effective `assistant:agent-learning` access for every person: set
   the global baseline false and remove or disable personal and org enabling
   overrides. The `flag_enabled_people` path honors personal overrides. An
   alternative is to stop all publication-serving traffic.
2. Drain in-flight publication requests and verify no old writer remains.
3. Deploy only new replicas. Startup classifies every started, unpinned legacy
   operation from its own encrypted draft or persisted operation-bound target,
   reserves update targets, and writes `assistant_learning_migrations` marker
   `_id: publication-targets-v1` only after all operations are classified.
   Verify that marker before enabling publication.
4. Deploy the frontend only after every backend replica serves the new
   preview. The authored card reads its actions from
   `GET .../learning/proposals/{id}?acknowledgement_id=...`; an older backend
   ignores the parameter and returns no actions, so a new frontend in front of
   it shows a read-only card. A new backend keeps the original preview when the
   parameter is absent, so the old frontend keeps working during the window.
5. Restore the prior flag configuration.

Rollback uses the same procedure in reverse: disable effective
`assistant:agent-learning` access, drain publication requests, then replace
replicas. Old binaries replace the whole `publication` object on claim and
ignore target reservations and the migration marker, so they must never serve
publication while rows written by this version exist and the flag is on. Keep
the flag off after a rollback until the new version is deployed again. New
fields left on rows are ignored by old readers and are reused when the new
version returns.

Skipping the drain or the order fails closed. Without the marker, claims and
allocations refuse; a card without actions is read-only; an operation that may
have dispatched stays reconciliation-only. No path infers that a second
publication is safe.

Claims and new allocations refuse with `nyxid_refused` while the marker is
absent. If startup logs an unresolved operation ID, inspect the exact original
operation in Ornn and recover its authoritative target GUID. Record the target
and kind on that publication, then rerun startup migration. A current agent
pin found by name or version is diagnostic only: renames and replacements make
it unsafe as target identity. Do not create a marker manually while any
started operation remains unclassified. Existing started operations with an
unrecoverable body must remain blocked until authoritative evidence is found.

Startup runs the migration once before serving. A pass that fails on storage
or on decrypting a legacy draft (for example a key-service outage) is retried
in the background, from 5 seconds doubling to 5 minutes, and logged as
"could not be decrypted; migration will retry". If a draft still cannot be
decrypted on the third pass, NyxID also logs an error and records
`assistant_learning_migration_unresolved` with `reason: "undecryptable"`, then
keeps retrying. A draft that decrypts but does not decode, or an operation with
no operation-bound target, is recorded with `reason: "unclassified"` and needs
the operator procedure below.

Use the original operation ID and proposal ID as the update fence. After
independent Ornn verification, an operator may set
`publication.target_kind="update"` and `publication.target_skill_id=<GUID>` on
that exact started proposal (or `target_kind="create"` for a proven create),
then restart one new replica to rerun the migration. Check the marker and
barrier row before restoring traffic. Never infer the GUID from the current
agent pin, a matching name, or a matching version alone.

#### Operator release of an uncertain target

Versions are derived from the attached base, so an uncertain attempt (for
example a migrated legacy draft) keeps holding `{skill GUID}:{version}` and no
other draft can publish that version. NyxID never skips to another version on
its own. A platform admin may release the target only with authoritative
evidence that the original request had no effect and can no longer have one:

1. Confirm in Ornn that the version list for the skill has no such version.
2. Confirm in Ornn's request records for that operation ID that the upload
   failed or never arrived, and that no retry of it can still be in flight.
3. Call `POST /api/v1/admin/assistant/learning/publications/{proposal_id}/release-target`
   with `{"operation_id": "<exact operation>", "evidence_ref": "<ticket or log reference>"}`.

The request requires a first-party admin session and fences on the exact
operation. It refuses a verified, pinned or leased publication and a landed
target. In one transaction it deletes the reserved or uncertain target and
marks the operation as never dispatched (`failure_code: operator_released`).
The same reviewed package can then be published again from its card:
**Retry publication** while the approving card is still live, otherwise
**Request a new confirmation**; or the owner can **Discard** the draft. A
legacy package (prepared before #1828) is never published again: its owner
rebuilds it first (see "Recovering drafts created before #1828"). The
`assistant_learning_publication_target_released` audit event records the
operator, proposal, operation, target, previous state and evidence reference
(metadata only). Do not edit the target or proposal documents directly.

#### Verified but not attached

An operation that may already have reached Ornn is never rewritten, and it is
still checked when the conditions for attaching it no longer hold. Two such
conditions exist: the agent's source skill pin changed, or learned evidence or
consent was withdrawn. NyxID reconciles and verifies the exact version
read-only, checkpoints a verified version, and then refuses to attach it with
`base_changed` or `evidence_unavailable` (status `published_unpinned`). The
operation is then settled: neither the card nor the learning panel offers to
check it again, and the server refuses any confirmation that tries, because
checking would only repeat the same refusal. The private version stays in the
owner's Ornn account and can be attached manually from the agent's skills. A
NyxID-side failure that is not a registry outcome (a lost lease, a concurrent
skills change, storage) is recorded by dispatch state only: `nyxid_refused`
before dispatch, `publish_uncertain` after a possible dispatch, `pin_conflict`
after verification. A read NyxID refuses or cannot start (an inactive catalog
row, an identity or storage error) sends nothing to Ornn, so it is also
`nyxid_refused` at validation and base verification, as for a refused
publication request; during reconciliation it stays `publish_uncertain`.

#### Learned evidence withdrawn after dispatch

Withdrawing consent or disabling learning invalidates a learned draft that never
dispatched. A learned operation that may already have reached Ornn is kept
instead: the review list shows it with `evidence_available: false`, its
confirmation only checks, and NyxID reconciles and verifies the exact version
read-only. A verified version is checkpointed but never attached; the proposal
settles as `published_unpinned` with `evidence_unavailable`. The private version
stays in the owner's Ornn account. The owner may attach it manually through the
agent's skill selection or leave it. If it never appears, the operator release
above applies.

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

## Owner-requested skill authoring

`draft_agent_skill` is NyxBot-only: it drafts for review, never publishes on a
model's authority. It reuses the default-off `assistant:agent-learning` flag,
encrypted proposal store, and the one-card fenced publication saga. The additive
proposal source defaults to `learned`; `authored` proposals do not require
automatic learning enrollment or conversation evidence. Flag discovery skips
membership resolution when no relevant rollout is enabled. Only owner NyxBot
chats read the authoring flag during tool discovery; a failed lookup hides only
the drafting tool. Guests and specialists cannot draft; specialists can ask
NyxBot with `request_agent_skills`.

Build agents from description (role/scope), persona (tone/style), and Ornn skills
(repeatable procedures, checklists, references, templates and workflows). Search,
preview and propose attaching existing skills before drafting uncovered procedures.
Never package or publish through Ornn Playground, sandboxes, machines or raw
Ornn upload APIs. NyxID validates bounded text, rejects credential shapes, encrypts
the draft and shows all proposed files on one owner approval card. The card stores
only IDs/revisions; its human review view reads the encrypted proposal.

Approval uses the existing server ZIP builder and signed approving-person Ornn
identity, verifies the returned exact version/hash and pins transactionally.
Improvements require an existing exact attached, private, person-owned base and
use PUT/new-version publication. Denial or expiry cannot publish. As in L1 PR-2,
org publication refuses `owner_binding_unavailable`: maintainers should attach an
existing approved skill until Ornn supports org ownership. No personal fallback.

Authored cards require a first-party human content review. Chat-code replies,
voice decisions and NyxBot permission decisions cannot approve publication.
The package's generated frontmatter and every text file are displayed verbatim,
as inert text. Retry uses the same card and publication operation; it never
replays an uncertain POST/PUT. The initiating model turn may already have ended
when the human approves. The current chat key, owner, proposal/skill revisions
and publication lease still fence the action.

Each authored card fetches its preview with its acknowledgement ID. The server
checks that card's binding and returns the actions available to that card;
the browser displays those actions without inferring them from proposal status.
An expired or outdated card can request a new confirmation for the same
operation, including a read-only check after a possible Ornn write. Denying an
older card after publication has started dismisses that confirmation and leaves
the operation available for recovery. After a refusal that the same package
cannot overcome (an interface, validation, package, dependency or name refusal,
a changed source skill, or a version conflict before dispatch), the card offers
**Deny** or **Discard** and no retry; NyxBot can draft a revised package. When
another draft's operation holds the target version and is the only blocker, the
preview reports a computed `target_busy` (not stored) and withholds the write.
A changed source skill replaces a transient failure code on an operation that
never dispatched, so the card stops offering a retry that cannot succeed.
Confirmations the server withholds return one fixed message and are audited as
`assistant_learning_publication_decision_withheld`, metadata only. Pre-claim
refusal audits also name the card that was clicked.

The issue #1812 report of 2026-10-09 captured `409` / `1004` ("Learning approval
card is missing, expired, used or stale") from the learning panel's
**Confirm publish & attach** on an authored proposal: the panel posted a card
that had never been decided. The panel now directs authored proposals to their
conversation card, and the HTTP approve route refuses authored execution through
an acknowledgement or raises a card elsewhere: for an authored proposal it only
renews the original card (`renewal_of`), in that card's conversation. The
panel links each authored proposal to its card's conversation (`?c=`). When
NyxBot drafted inside a group, that is its hidden member thread, which opens
as a thread page with the full card; the group view only summarizes member
actions and cannot show the draft files. For a learned
proposal, **Confirm publish & attach** acts only on the owner's own pending or
allowed learning card for that exact binding in the NyxBot conversation that
raised it (or the used card that approved the operation, to resume it). It
allows and audits that card (`assistant_acknowledgement_decided`) and refuses
any other card id, including service, account and authored cards, without
changing it. A refused confirmation is shown in the panel.

Authored validation is separate from learned L1 validation at creation, editing,
review and publication. Authored fields reject credential shapes (provider keys,
secret assignments, private keys, bearer/JWT tokens, URL passwords and credential
query parameters). Ordinary reference URLs, paths, versions, example IDs/email
addresses and long checksums are allowed for owner review. The telemetry scrubber
is not an authoring policy: its query-URL, email, UUID and unbounded `basic`/`token`
matches reject ordinary prose. L1 additionally rejects long token-like runs and
retains all of its existing redaction and validation rules unchanged.

Authored caps count Unicode scalar characters: name 64 (Ornn ASCII slug),
description 400, SKILL.md 7,500, optional paths 160 and file contents 2,000 each,
at most eight extra files, rationale/safety notes 1,000 each. The total serialized
proposal, including metadata and JSON escaping, stays at 7,500 characters with a
30,000-byte ceiling (four bytes per character). L1 keeps its 8,000-byte ceiling.
No script or binary files are allowed. The deterministic ZIP adds bounded
frontmatter and nine entries at most; NyxID's existing 4 MiB archive limit remains.
[Ornn's upload defaults](https://github.com/ChronoAIProject/Ornn/blob/c93036d6be7cb19c7ca3294da930594978714fd6/ornn-api/src/infra/config.ts#L132)
are 50 MiB uploaded/expanded, 25 MiB per entry and 1,000 entries; remote format
validation still applies. Envelope encryption adds at most 1,056 bytes (including
the bounded wrapped DEK), far below MongoDB's 16 MiB document bound.

Refusals retain `validation_error` / 1008 and include safe `details`, for example
`{"rule":"credential_shape","field":"files[2].content","line":14}`. Lines are
one-based within the original field; fields are fixed schema names with zero-based
array indices, never supplied filenames. Size errors use `too_large`, `field`,
`limit`, `actual` and `unit` (`characters`, `bytes` or `items`); total-budget errors
use `field: "draft"`. Other fixed rules cover required text, slug/path constraints,
duplicates, text-only content, input/schema/kind shape and base matching. Unknown
JSON keys report their containing object. No matched text or raw parser errors
enter diagnostics, logs or audit. No new permission or publication path is added.

## Recovering drafts created before #1828

Before #1828, NyxID packaged every update of an existing skill as
`category: plain`, without the base skill's tools, runtimes or output type, so
Ornn rejects the update of any non-plain base. Such a **legacy package** is an
update whose publication has no interface snapshot (`package_format` below 2,
which includes rows where the field is absent, and no `interface_encrypted`;
an unclassified row counts as an update when its draft has a base skill).
NyxID never dispatches a legacy package: no card, panel or approve request
can confirm it, and only an operation that already dispatched may still be
checked.

A legacy operation counts as never dispatched when it has no dispatch flag,
live lease or verified skill, whatever its status: besides `pending` and
`publication_failed`, pre-#1828 servers recorded a failure before dispatch as
`published_unpinned` (shown as `publication_failed`, since nothing was
verified) and left a crashed attempt `publishing`. Their stored
`publication_retry_required` code is shown as the rebuild prompt, or as
`publish_uncertain` for an attempt that did dispatch. Such drafts can also be
discarded.

### Owner flow

- **Authored drafts:** every card of the draft, live or expired, offers
  **Review updated package** (the `reprepare` action) while the operation never
  dispatched and the source skill is still the attached pin. The card shows "This
  draft was prepared by an older NyxID version that can't preserve the skill's
  tools/runtimes. Review the updated package." The click calls
  `POST /api/v1/assistant/nyxagent/agents/{id}/learning/proposals/{proposal_id}/reprepare`
  with `{"acknowledgement_id": "<clicked card>"}` (first-party human session,
  live card authority as for a card decision). NyxID reads the exact attached
  base with the owner's own identity, builds the snapshot, and in one
  transaction replaces the operation (new operation ID, package and hash, next
  revision), deletes only the old operation's own `reserved` target, clears the
  failure code and writes the `assistant_learning_proposal_reprepared` audit
  event. It then raises a new card in the clicked card's conversation. The
  package changed, so the owner reviews every file before publishing; older
  cards show "This confirmation no longer matches the draft."
- **Interrupted rebuild:** if the process stopped after the swap and before the
  new card existed, older cards of the draft offer **Show updated draft**,
  which only raises the missing card (a live card for the new package is
  reused). It authorizes nothing and changes no publication state. If the
  swap committed but the card could not be raised, the request still succeeds
  with `card_pending: true` and the card says to use **Show updated draft**.
- **Learned drafts:** the learning panel shows the same copy and a **Review
  updated package** button (the same route without `acknowledgement_id`). The
  rebuilt draft is then confirmed from the panel as usual.
- **Base no longer reproducible:** if the agent's pin changed, or Ornn's base
  moved on or cannot be reproduced, the rebuild refuses (`base_skill_changed`,
  `base_interface_incompatible`, `publication_integrity_failed`) and records
  `base_changed`, `base_interface_incompatible` or `base_verify_failed`; the
  operation is unchanged and the owner can only discard the draft and ask for
  a revised one. A transient read failure records nothing
  (`source_skill_unavailable`).

### Operator flow for stalled legacy dispatches

A legacy operation that dispatched with an unknown outcome holds its target
and is check-only. A platform admin may release such operations in bulk with
`POST /api/v1/admin/assistant/learning/publications/release-stale` (first-party
admin session). Body: `{"dry_run": true, "min_age_hours": 24, "limit": 200}`;
these are the defaults, `min_age_hours` is 24..=8760 and `limit` 1..=200.

1. Run a dry run and review every row. A candidate is a dispatched (`started`
   or `uncertain_dispatch`), unverified, unleased legacy update in `pending`,
   `publishing` or `publication_failed`, or a draft discarded before the
   migration (`rejected`, `invalidated`) whose dispatched operation still holds
   its target. Its age is measured from the latest of its first dispatch, its
   lease expiry and `legacy_classified_at`, which startup stamps once on every
   dispatched legacy update. Nothing is releasable until `min_age_hours` after
   the new version first ran. Releasable rows come first, oldest stamp first;
   rows the stamp missed follow only while `limit` has room. At most `limit`
   rows per call, read with bounded concurrency within a 60 second deadline.
   Both dispatch branches of the query are indexed (`{publication.started,
   status}` and the partial `{publication.uncertain_dispatch, status}`).
2. For each row NyxID reads Ornn with the proposal owner's identity, never the
   admin's: the exact version must be a definitive 404 `skill_version_not_found`
   and the latest version must be exactly the base version in the operation's
   own draft. A discarded draft no longer has its draft, so its version must
   instead be missing from the version list; nothing is rebuilt from it.
   Decisions: `release` (dry run), `released_audited` (applied),
   `release_reservation` / `released_reservation_audited` (a discarded draft:
   only its target reservation is released, it stays discarded),
   `landed_check_again` (the version exists; the owner's **Check again**
   reconciles it), `skip:landed` (the version of a discarded draft exists; its
   reservation stays), `skip:uncertain`, `skip:latest_mismatch`,
   `skip:age_unknown` (no stamp yet), `skip:legacy_target_unresolved`,
   `skip:barrier_changed`, `skip:cas_changed`, `skip:deadline` and `skip:error`.
   Responses carry identifiers and decision codes only.
3. Repeat with `"dry_run": false`. The apply pass reads Ornn again (dry-run
   results are never reused) and releases each row in one transaction fenced
   on the full observed state (revision, status, operation, attempt, package
   hash, target, legacy format, dispatch flags, no verification, no live lease)
   and on the target barrier. The same transaction records
   `publication.release_evidence` and the `assistant_learning_publication_target_released`
   audit event with the admin actor, `evidence: "absence_only"`, the batch ID,
   the evidence and `reservation_only`; a summary event
   (`..._stale_release_summary`) lists the decision counts. A release
   therefore never exists without its audit event.
4. The released row shows **Review updated package**: the owner rebuilds and
   reviews it as above before anything is written again. A discarded draft's
   release only frees the version for new drafts.

Every release is labelled `evidence: "absence_only"`. This is an explicit
exception, limited to legacy operations, to the rule that absence never
authorizes another write.

### Residual risks

- A request stalled longer than `min_age_hours` can still land after release.
  The rebuilt operation targets the same version and pins only after exact
  version and ZIP hash verification: if the old package landed first, the new
  PUT is refused, reconciliation finds a different hash and the operation
  becomes a terminal `version_conflict` (target retained, nothing attached;
  the card explains that an earlier request may have landed late). If the old
  request lands after the new version was pinned, Ornn may move its `latest`
  tag back; the agent's pin stays on the verified new version.
- The flags and drain procedure of "Rollout and compatibility" still apply;
  keep learning disabled until the migration marker is present.
