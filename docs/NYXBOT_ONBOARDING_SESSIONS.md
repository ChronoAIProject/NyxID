# Interactive multi-use-case onboarding in NyxBot

Status: proposed implementation contract. This document does not implement the
routes, tools, persistence extensions or interface described below.

## Product model

One saved owner setup can contain several use cases. One NyxBot conversation
coordinates that setup, with a persistent interactive progress panel and focused
chat guidance. Each selected use case has its own configuration, specialist,
requirements and readiness. Shared business details and compatible connections
are collected once. Skills guide the conversation; backend state records what
has actually been configured, approved and tested.

The initial restaurant selection contains FAQ, queue management and reservations.
Telephone is a shared channel, not another use case. FAQ and reservations can
share Telegram and telephone only after the required routing support exists.
Queue management remains private to staff, with staff-triggered SMS alerts.

## Admin-managed use-case catalog

The current checkout supports owner-managed agent skill pins, conversation links
and anonymous platform draft adoption separately. It has no admin-managed mapping
from arbitrary use cases to skills and setup chats. Platform setup templates are
currently a fixed two-value enum; `/admin/platform-bots` configures platform
profiles and guide execution, not a dynamic use-case catalog.

Add a Use cases area in admin, backed by versioned records. Suggested fields:

| Field | Purpose |
| --- | --- |
| Stable use-case ID | Public slug used by links and saved setup selections |
| Title, description, welcome copy | Public selection card and owner setup introduction |
| Status and availability | Draft, published or retired; required implemented capability identifiers |
| Onboarding skill references | Exact Ornn pins for guidance attached to owner NyxBot |
| Default operating skill references | Exact Ornn pins and target roles used to prepare the specialists' private operating copies |
| Requirement references | Supported backend requirement/check identifiers for services and flows |
| Bundle members | Exact use-case definition revisions for a curated bundle |
| Definition revision | Immutable snapshot selected by a new setup |

The skill remains the authored Markdown procedure. This record provides routing
and display metadata, not a second YAML workflow or an arbitrary executable
configuration. Admins select published skills using the existing registry
integration; they do not paste raw credentials, executable snippets or arbitrary
skill URLs. Separate leaf definitions and bundles; reject recursive membership
and unsupported capability/check references.

Only platform admins publish or revise definitions through first-party sessions.
Writes use expected revisions and audit metadata. Validate package integrity and
supported runtime requirements on publication; recheck the acting owner's live
Ornn visibility and requirement availability at setup start. Admin access to a
package does not make it visible to every user. Publish intended onboarding
packages with appropriate registry visibility; never use admin/master credentials
to bypass a user's access. Show unavailable cases as unavailable.

Existing setups retain their selected definition and skill versions when an admin
publishes an update. Offer an explicit migration/review for changed guidance or
requirements. Retirement prevents new selection; existing setups retain history
and remain subject to live permission and platform availability checks. Platform
disable/revocation remains a separate immediate execution fence.

At start, resolve the selected definitions, deduplicate identical skill pins,
reject conflicting versions of the same root/dependency, preserve unrelated agent
skills and request any necessary owner attachment review. A published definition
is not user consent to attach skills or grant service access.

Skills currently attach to `AssistantAgent`, not a conversation. The setup-to-chat
association controls the selected workflow and focus, while attached guidance is
available in other owner chats for that agent. Do not claim chat-only skill
isolation. True per-conversation skill filtering would be a separate runtime
change; the first implementation can use existing agent attachments and explicit
per-turn setup context.

Each initial specialist also needs default operating guidance and an owner
customization lifecycle. See [operating skills and improvement](USE_CASE_SKILL_LIFECYCLE.md)
for private copies, existing authored review/publish/pin support, testing gates
and the distinction between admin default updates and user-specific changes.

This catalog lets admins add combinations and procedures using already supported
tools without a deployment for each use case. A new provider adapter, form type,
business operation or readiness check still requires implementation. A catalog
entry cannot make an unsupported telephone or queue flow operational.

## Carryover from public guide to personal NyxBot

The user may experience one continuous private Telegram conversation with the same
bot identity. Internally, the public guide and personal owner runtime retain
separate authority and model context. Do not mutate the shared public agent into
one user's personal agent or give it that user's connections. Reuse the verified
sender-to-owner routing intended by PR #1901 when that runtime is available.

1. The public guide records a bounded anonymous draft: selected use-case definition
   revisions, permitted business answers, locale/timezone if supplied, and a
   next-step hint. Use a server-owned draft/intent reference, not the full transcript
   or private data in a signup URL. Public discovery exposes only approved catalog
   copy, selection IDs and permitted availability information.
2. Signup/login returns to the server-issued linking/adoption flow. Telegram entry
   still requires the existing web approval and same-private-chat confirmation.
   Bind the intent to its originating channel/sender and generation; enforce its
   expiry and single successful redemption. A web-only selection does not prove
   Telegram ownership.
3. Show the signed-in owner the proposed cases and collected facts: for example,
   “Continue setting up FAQ and reservations for this restaurant?” Explicit
   adoption binds the draft to that owner using existing revision/generation
   checks. Repeated completion returns the same result; it cannot transfer an
   already adopted draft to another account.
4. Resolve or create the owned setup and coordinator conversation once. Store the
   adoption result and durable continuation intent so a crash after adoption can
   resume instead of creating another setup. Preserve selected definition revisions,
   revalidate availability and request any missing skill attachment approval.
5. Supply personal NyxBot with a server-generated structured summary of the adopted
   draft and setup reference. Treat public answers as untrusted user data, never
   instructions or verified readiness. Ask only for missing facts; recheck all
   connections, resources and tests under the owner's actual authority.
6. Continue in the personal Telegram route and expose a web resume link to the
   associated setup. Public/group turns remain restricted. Unlinking or changing
   the account invalidates that channel's personal routing; a later login must
   never inherit another owner's setup or conversation.

Carry over the chosen use cases, explicitly adopted business answers and the
next-step intent. Do not carry over platform credentials, agent keys, service
grants, approvals or public-model session memory. Existing valid resources may
be discovered and reused under the owner, but signup does not authorize them.

## Entry and resume

Proposed links on the configured frontend origin:

```text
/assistant?onboarding=restaurant-faq
/assistant?onboarding=restaurant-faq,restaurant-reservations
/assistant?onboarding=restaurant-faq,restaurant-queue,restaurant-reservations
/assistant?onboarding=restaurant-suite
```

Accept a bounded set of known IDs: only these three in the initial release.
Canonicalize ordering, deduplicate IDs, reject unknown IDs and duplicate query
parameters, and expand the curated suite on the server. Reject mixed suite and
individual selection syntax. The selection is a hint, never a prompt, skill
location or permission grant. Reject entry links combined with unrelated
`agent`, `g`, `c` or `setup` context.

Preserve the complete encoded selection through the existing trusted auth return
flow. The initial screen preselects matching checkboxes and describes the setup.
An explicit Start action begins setup, using normal owner skill attachment review
where needed. Loading a URL causes no agent creation, provider connection, skill
attachment or billable turn.

Proposed start request to `POST /api/v1/assistant/onboarding/start`:

```json
{
  "use_case_ids": ["restaurant-faq", "restaurant-reservations"],
  "idempotency_key": "client-generated-start-intent"
}
```

Derive the owner from authentication. Resolve exact visible published skill pins
server-side. Return the owned setup, coordinator conversation and pending review.
Repeated starts for the same active selection offer resume; creating another
installation requires an explicit choice. A durable start intent and atomic
claim prevent duplicate work across retries, approval completion and tabs.

Replace the entry URL with a canonical resume link:

```text
/assistant?c=<owned-conversation-id>&setup=<owned-setup-id>
```

Authorize both resources and their association. Neither opaque ID is an access
token. The existing assistant search parser needs new fields for these routes.
Use server-owned public-to-personal linking intents for Telegram identity handoff;
query parameters cannot replace that proof.

## Durable setup and conversation context

Extend the existing `PlatformSetup` lifecycle rather than creating a competing
installation authority. Its current fixed templates and single-agent resource
binding need explicit extensions. Proposed additive information:

```text
setup_id, owner_user_id, revision
coordinator_conversation_id
selected_use_cases: [id, definition_revision, exact_skill_pin]
focused_use_case_id?
shared_configuration: encrypted draft
requirements: [requirement_id, connection/resource references, dependent use cases]
use_cases: [id, configuration_revision, specialist_ref, phase,
            per-channel readiness, test_receipts, blockers]
steps: [stable step_id, dependencies, state, action/result references]
desired_revision, approved_revision, active_revision?
```

These are conceptual fields, not a replacement for existing model ownership,
encryption, revision/digest and provisioning fences. Store credential references,
never credentials. Add a setup reference to the coordinator conversation and
provide a bounded server-generated setup projection on each owner turn: selected
cases, current focus, verified progress, missing requirements and next actions.
Conversation checkpoints and model statements cannot establish completion.

Attach exact onboarding versions under the effective permission mode: owner
review in Auto/Ask every time, automatic receipt in Full access. Preserve
unrelated attached skills and enforce the existing 16-skill limit. Read the
focused skill as needed; do not paste every full skill into every turn. An
unpublished or inaccessible skill is an actionable blocker, not permission to
substitute an arbitrary skill or platform credential.

The owner web conversation coordinates setup. A properly linked personal Telegram
thread can reference the same setup without merging transcripts or transferring
ownership. Each surface reads the same authoritative state. Callback and turn
continuation claims prevent simultaneous surfaces from duplicating a step.

## Interactive interface

Show a compact, expandable setup panel above the chat transcript. Example state:

```text
Restaurant setup                                  1 of 3 ready

FAQ agent             Ready to activate            [View tests]
Queue manager         Needs SMS connection         [Continue]
Reservation manager   Needs Notion destination     [Choose destination]

Shared connections
Telegram              Connected                   FAQ + Reservations
Telephone             Connected                   FAQ + Reservations
SMS sender            Not connected               Queue
Notion                Connected                   Reservations

[Change selection]  [Pause setup]

NyxBot: Notion is connected. Which database should receive reservations?
[Choose database]  [Work on queue instead]
```

This is a proposed wireframe, not evidence of real configured services. The panel
counts ready use cases from recorded checks; it does not display a model-invented
percentage. A connected provider is not proof that a use case works.

Render focused cards for:

1. Selecting use cases and reviewing their shared requirements.
2. Entering business details and per-use-case policies.
3. Reusing, connecting or repairing a service through hosted consent.
4. Choosing an actual reachable database, channel, queue source or SMS sender.
5. Reviewing specialists and exact access before provisioning.
6. Authorizing test effects and inspecting recorded results.
7. Reviewing and activating the tested composition or an explicit ready subset.

Forms use `useAppForm`; buttons and fields need accessible names, keyboard
operation, focus management and announced status updates. Reuse native NyxAgent
owner acknowledgement and authored-skill cards where applicable. Existing task
plan and `service.connect` cards illustrate interaction patterns, but belong to
the retained Aevatar path; they do not establish a NyxAgent onboarding producer.

## Chat and cards operate on the same state

“Set the return window to ten minutes” proposes the same validated update as
editing that field. “Do reservations first” changes focus without removing FAQ.
“Add FAQ” changes the desired selection and triggers any required skill/access
review. NyxBot confirms persisted changes after the tool succeeds.

Bind each card action to setup ID, stable step ID, expected revision and an
idempotency key. The server checks ownership, live authorization, step preconditions
and schema validation. Stale cards refresh to current state instead of applying
an old decision. Conversational actions use equivalent bounded tools and the
same service-layer validation; the model cannot mark a test passed by narration.

On hosted connection completion, persist the verified result before claiming one
continuation. Reconcile live service access and selected resources. Decline,
expiry or failure preserves already supplied answers and presents recovery.
Unknown external outcomes require reconciliation before retrying effects.

Reuse shared resources only when the actual connection, selected resource and
authority are compatible. A matching provider name alone is insufficient.
Changing a shared dependency invalidates affected tests and requires review of
the changed composition. An unrelated blocked use case does not prevent collecting
details or testing independently available cases.

Removing a draft use case cancels its pending steps. It does not automatically
delete a specialist, revoke a shared connection or detach an unrelated skill.
Removing an active case requires explicit deactivation. Keep desired, approved
and active revisions distinct so editing a draft cannot silently change live
customer behavior. Show what remains active while a new revision is being set up.

Partial activation requires an explicit reviewed subset. Never label the full
selected suite ready while a required case or channel is blocked. Pause prevents
new setup steps; Stop interrupts the current agent turn. Neither reverses external
work already committed, and in-flight effects must still reconcile into state.

## Telegram representation

Deliver full forms and review cards in the web application. Personal Telegram can
show a short progress summary, accept supported conversational edits and provide
an authorized resume link. Rich web cards do not automatically render in Telegram.
Any new Telegram action callback needs the same owner, step and revision binding.

PR #1901's intended verified personal owner path can consume attached skills and
reference this setup after its remaining runtime work is complete. It does not
implement this multi-use-case contract. Public or group contexts receive only the
bounded public selection/handoff projection, never private setup inventory.

## Required implementation and acceptance

1. Add the admin use-case catalog, multi-selection parsing, server skill mapping
   and trusted auth/linking/adoption continuation.
2. Extend durable setup and conversation associations, preserving legacy rows.
3. Add authenticated start/read/update/action APIs and bounded equivalent NyxBot
   tools, idempotent transitions, callback reconciliation and live progress events.
4. Add the persistent NyxAgent setup panel, forms, resource choosers and recovery.
5. Implement reviewed specialist provisioning, business adapters, shared channel
   routing, evidence collection and activation gates described in the linked plan.

Verify all seven nonempty selections of the three cases; invalid and duplicate
parameters; signup/login/MFA return; reload/resume; shared connection reuse;
focus changes; add/remove; stale cards; duplicate callbacks; parallel tabs;
cross-owner/cross-conversation denial; uncertain provider results; pause/stop;
accessibility; and explicit partial versus complete readiness. Verify linked
Telegram and web actions cannot provision duplicate specialists or replay sends.

See [skill authoring](USE_CASE_TEMPLATES.md),
[restaurant requirements](RESTAURANT_USE_CASES.md),
[implementation proposal](plans/2026-10-15-use-case-bundles.md), and
[chat architecture](chat/README.md).
