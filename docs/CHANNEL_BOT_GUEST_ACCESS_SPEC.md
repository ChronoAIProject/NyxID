# Channel bot guest access add-on

PR #3004 implements this contract on NyxID `dev` at `4d3cb8902`. The platform flag defaults to off. The implementation plan records the earlier prototype and compatibility reviews. [Implementation plan](plans/channel-bot-audience-boundaries.md)

## Runtime contract

`nyxbot:guest-audience-v1` is a platform-wide runtime flag, off by default.
One admin change selects the existing implementation for fresh requests on
**every bot**, including bots that previously used the add-on. It requires no
redeploy, database restore, per-bot reset or manual drain. Each bot also has a
saved **Use guest access v1** setting. Existing bot records with an absent
setting resolve to off. Once all bot-creation paths support the new default,
every newly created bot saves on at insertion, even while the global flag is
off. This applies to new bots created by existing accounts. A retried create,
reconnect, new chat, binding change, ownership transfer or settings update
on an existing bot preserves its setting. Existing bots need no backfill.
An authorized bot owner or organization administrator can change the setting
for that bot. An opt-in enables the guest-access rules with zero data grants
until a policy grants access. The channel's existing admission controls also apply.

Fresh work uses the add-on only when the global flag is on, the bot setting is
on, and the rollout includes that installation. A missing or empty selection
includes zero bots. A named pilot includes only the listed bot IDs. The
explicit fleet selection includes all present and future bots with the setting
on. Selecting the fleet cannot opt existing bots in. The admin sees the
affected scope before applying the selection. The v1 rollout selection is
fixed while the flag is on. To change the pilot list, the admin switches
off, edits the selection and enables again. Setup identifies the existing
pilot bots affected by that switch and its effect on their in-flight work.
Call bindings resolve to their parent installation before selection. A binding
ID alone cannot enroll a phone, SMS or speech route. Global off wins over
every selection and saved policy.

| Request | Behavior |
| --- | --- |
| Fresh request with the flag off, whether before or after a pilot | Use the existing setup and channel execution paths with the bot's current ordinary settings. The main NyxBot and specialists retain their existing, different guest behaviors. A separate owner-initiated migration preview can prepare settings without affecting those paths. |
| Fresh request with the flag on and the bot setting off or absent, or outside the rollout | Use the existing paths. |
| Fresh request with the flag on, the bot setting on and the bot included | Use the add-on for guest setup and admitted guest turns. Owner and verified-member operations keep their own authority. |
| Legacy work outstanding when a bot enters the rollout | Honor a delivered callback's outstanding, valid reply at its original source. Pending ingress keeps its original claim and retry disposition. Finish legacy work before selecting the bot if completing that work would require reissuing a broad bearer. |
| Add-on work outstanding when a bot leaves the rollout or the flag turns off | Cancel pending effects and suppress undelivered replies. Settle a proven obsolete receipt terminally with the provider. A transient authority lookup failure remains retryable. |
| Duplicate provider event in either direction | Recover the original admission. A completed callback or effect is never dispatched again. A genuinely new source event gets the current mode. |
| Fresh request after re-enabling | Use the saved add-on configuration, current grants and a fresh activation generation for selected bots. |

Track pilot selection and applied policy revision separately. On enrollment,
an existing bot without an applied policy starts with zero guest data grants.
The bot can use attached skills authorized for its audience. Saving a draft
does not publish grants. The same defaults apply to a new bot. Before enrollment,
inventory current guest workflows, affected owners, pins and external keys.
Tell owners which workflows need new grants and which skill contents the bot can
quote to guests.

An authorized owner or maintainer can open a migration preview for a proposed
pilot bot before enrollment. They can stage replacement grants and private or
shared audiences for each pin and preview both guests without fetching guest
data or invoking providers. The editor identifies existing operations without a
supported replacement. For named guests and rooms, the preview can use existing
server-verified channel observations bound to the same bot, binding and owner.
When that evidence is missing, leave the named selection unavailable and show
why. Staging has no effect on ordinary traffic. The owner can
mark an exact draft revision for activation. On selection, recheck ownership,
authoring rights, grants, verified identities and pin revisions and activate the
valid configuration together with the bot's enrollment. A stale prepared
revision leaves that bot in ordinary mode with a visible reason. Other bots
proceed independently. First-time enrollments without a prepared revision use
the documented defaults. Returning bots retain their valid saved policies.
A bot skipped during activation can enter after its draft is corrected and
activation is retried for that bot within the selected rollout. An explicit opt-in
on an existing bot can activate its prepared, valid revision in the same operation.

With the add-on enabled, the bot can read its enabled, pinned skills for any
admitted conversation. An owner can share information with a named guest or
audience and enable create/read/update/delete on a guest's own records for a
supported source. The server verifies identity, resource ownership, permitted
fields, operation and reply audience. The setup step shows these choices,
including any permissions proposed by a skill and applied by its maintainer.
The contract applies across channel transports. Telegram, Lark/Feishu and
Discord each require an end-to-end acceptance run.

Enrolled hosted guest turns in v1 accept text. Personal channels with guest
media enabled lose guest image and document processing while enrolled.
Setup and the enrollment inventory surface this change. Off and outside-pilot
requests retain the existing per-channel media and readiness checks.

Personal-record operations require a provider-verified one-to-one conversation
with the requesting guest. When a guest asks about personal records in a
shared conversation, the hosted bot asks them to open a private chat with
the same bot. The request starts again there with that chat's verified sender
and destination. The group turn has no personal-record tools or results.
An owner can explicitly authorize a resource for a shared room. Everyone who
can see that room can see its answer, even when only selected guests may ask.

The first own-record source is the bot's personal records collection. NyxID can
enforce the verified actor and bot in the database operation that commits a
change. For the related Discord event workflow, a Sheets grant identifies an
exact connection, workbook and bounded range to read. Appending a question
requires a separate append grant for the named tab. Google's values API uses
the supplied range to locate a table and chooses the insertion position.
Setup describes the entire tab as the possible write boundary. A range such
as `Questions!A:H` alone cannot authorize a bounded write. The shared-room
append accepts a single row with `RAW` and `INSERT_ROWS`. A private turn
cannot write its inputs into the shared tab. For both Google Workspace and the
standalone Sheets catalog, bind each grant to the actual connection, enforce its
owner's applicable approvals and check the resolved Google destination at send.
Existing grants alone leave both operations unavailable on enrollment.
[Reported Discord event workflow](https://github.com/ChronoAIProject/NyxID/issues/2704)

Hosted guest discovery describes this restricted Sheets contract on both Google
catalogs. Direct MCP `tools/list` and `nyx__search_tools` publish a bounded,
tab-qualified read range (at most 32 columns and 2,000 cells), without `FORMULA`
rendering. Append guidance requires a shared room, a named tab with a column
range of at most 32 columns, exactly one nonempty row, and exactly one each of
`valueInputOption=RAW` and `insertDataOption=INSERT_ROWS`. The JSON body is at
most 16 KiB, contains `values` and optionally `majorDimension=ROWS`, and accepts
only strings, numbers and booleans as cells, with no more cells than the selected
columns. `includeValuesInResponse` is omitted or false; response-render options
and `body.range` are unavailable. The discovery schema narrows these choices
while preserving the existing `body` argument wrapper. Rendered instructions
precede the connection label. Compact search retains the exact authorized read
range guidance, append requirements and effective approval requirement when it
omits the schema, including with a 200-character connection label.
Discovery publishes no workbook, tab or range from the guest data grants, and
does not establish approval or access to any example resource. Execution checks
the exact request, live policy, Google destination and applicable approvals.
Owner, verified-member and non-enrolled discovery keep their ordinary contracts.

## Code and storage boundaries

Register the built-in flag as global-only. Use the authoritative global reader
at the authority boundary and store pilot membership by bot in the add-on.
Reject organization and person overrides. Check pilot membership only when on.
A fresh source on a verified bot record whose setting is off or absent uses the
ordinary path without an add-on storage read. A failed bot read cannot supply
the missing-field default. Recover previously admitted work and credentials by
their stored identity before applying fresh-request defaults.
A successful read of an absent global override selects off. An unreadable
override or selection supplies zero authority to run tools, forward credentials
or deliver results. Live setup uses the same selection as execution. The
pre-enrollment editor is a separate management operation.

Preserve each provider's acknowledgement contract, deadline and meaning of
success. A receipt-only ACK can precede execution. Ordinary prompt ACKs
must not acquire an add-on database dependency or a new 650 ms timeout.
Where a success response promises handling, require completed handling or
durable, deduplicated intake that can retry classification first. Document each
adapter's recovery mechanism, including visible failure for providers without
redelivery. A transient selector error records the disposition of the affected
item and allows other valid items in the batch to proceed. This applies to
direct, inline and managed webhooks, hosted callbacks and native ingress.

Persist a server-produced turn record with the implementation mode, bot,
verified actor and assurance, answer audience, source event, policy revision
and activation generation. Keep the guest classification on add-on guest turns.
Only explicitly authorized add-on operations can cross an existing guest
restriction. Retries, workers and tool dispatch load that record. Existing
ordinary rows deserialize as legacy. A feature-owned envelope with incomplete
identity fails closed, including on older queues or workers.

| Component | Responsibility |
| --- | --- |
| Existing channel handlers and workers | Call the selector at the entry boundary and dispatch to the existing functions for ordinary work. At callback and retry boundaries, dispatch by trusted work or credential identity. Keep existing authorization logic directly callable and covered by its existing tests. |
| `backend/src/services/channel_guest_access/` | Owns identity mapping, policy evaluation, private delivery checks, bound skill reads, ownership-aware connector operations and versioned work. It uses existing verified adapter facts, pin validation, operation ceilings, billing and audit services. |
| Feature-specific models and storage | Keep policies, ownership mappings, private sessions, work and result references separately addressable. Give feature-owned work a distinct namespace. Preserve ordinary grants, chat settings, credentials, sessions and wire formats. Apply account erasure and retention to feature data even while disabled. |
| `frontend/src/features/channel-guest-access/` | Owns live setup for enrolled bots and an owner-initiated staging and preview surface for proposed pilot bots. Server endpoints check management authority for staging and enrollment for live changes. |

Store skill policy declarations separately from existing pin and reference
formats that reject unknown fields. Keep the existing guest service-level enum
and record own-data scope in the add-on policy.

Off-path guest requests must work with empty or unavailable add-on policy
storage. A never-enrolled bot with no staged draft can transfer or be deleted,
including through organization deletion, without creating or reading add-on
policy. For a staged, enrolled or formerly enrolled bot, fence the old owner
immediately, retire its drafts, policy and capabilities, and complete scoped
erasure with durable retry, including on organization deletion.
Re-enrollment or identifier reuse cannot revive the previous owner's data.
An upgraded worker must recognize an add-on callback or retry even
when the feature is off. Use dedicated work and token formats that legacy
queues and authenticators exclude. Give add-on turns separate conversation
rows, active-turn locks and model sessions, keyed by the channel conversation,
verified actor, audience and activation generation. A new legacy turn in the
same channel thread can acquire its ordinary conversation while an add-on
turn is settling. Normal user edits to ordinary bot settings
remain effective when switching back. No configuration restoration job runs.

Each new admitted unit of guest work gets its own conversation credential.
Retire the previous credential
atomically and reset the upstream session. Coalesced events, steering and
recovery of that same unit keep their original credential. Check the key, turn,
and work identity again at dispatch. An older in-flight call must settle
against its original reservation. Retain bounded local history only within the
same bot, verified guest, destination and authority revision. After permissions narrow,
exclude content admitted under the earlier authority from the next prompt.

Organization specialists enforce current use, maintenance and governance
rights independently. Changing guest policy or pin authorizations requires
the existing authority for each resource. A known member denied specialist
use cannot retry as a guest. A verified external guest can enter only through
the configured guest path. Preserve the established payer and connection-owner
approval checks through queued execution, settlement and revocation.

Capacity waits, restarts, source-result lookups and saved replies retain the
original bot, source route, credential binding and work identity. Source-result
readback exposes the ordinary authorized metadata with its existing expiry.
Private add-on transcripts, response references and sender metadata are scoped
to their authorized audience across polling, streaming and conversation lists.
Use the current record-bound storage conventions for retained setup payloads,
delivery targets and native reply handles, including field purpose, record,
owner, version markers and the applicable channel. Cover feature fields in
encrypted-field verification, migration, transfer and erasure.

Provider-specific restrictions apply in every mode. Messenger and Instagram
use the connected asset and participant identity, explicit customer-DM setup,
text reply contract, signed message time, live participant control and reply
window. Bind observed guest IDs, work, records and erasure references to the
correct installation and grant. Suppress owner access tokens on all Meta
callbacks. Apply grant-scoped erasure to new feature data attributable to the
grant, preserving sibling inboxes and existing statutory retention.

## Rollback and side effects

Bind feature executions and relay capabilities to a bot, actor, source event,
audience and activation generation. Update the generation inside the global
flag administration transaction on every effective transition, including
clearing an override back to default off. The bot setting
has its own durable revision. Turning it off fences that bot's pending work at
dispatch and delivery, and immediately permits its fresh ordinary requests.
Turning it back on cannot revive work from an earlier revision. Changes to one
bot's setting do not cancel work on another bot. Enablement requires readiness
across the relevant ingress, execution and delivery workers. Disable remains
available in a partially upgraded fleet. Fence admission, dispatch and delivery
against the committed transition. Each worker owns cancellation of its stale
turns and streams. Use generation checks at the effect boundary even if a
worker misses the disable notification or restarts. A fresh admission after
switch-off must proceed independently of worker cleanup.
Cancel undispatched add-on effects, revoke stale capabilities, and suppress
pending add-on replies or streams on disable. Reconcile already-dispatched
provider effects and billing with their original idempotency identity. Keep
the original event claim across modes to prevent duplicate execution. An
off/on cycle cannot revive a queued action from a previous activation.

Legacy replies finish all retryable authorization checks before spending a
single-use reply token, or use a recoverable claim covering those checks and
dispatch. A temporary storage error before provider submission leaves the
reply retryable. Reuse the existing reply journal for accepted, partially
accepted and ambiguous sends. Never automatically resend a reply whose provider
acceptance is unknown. Pending legacy Aurinko deliveries settle or recover
through their original receipt, without repeated refusals solely due to enrollment.

Ordinary behavior includes the ordinary data visibility of a connected
service. If the legacy configuration grants broader reads, switching back
restores those reads for fresh legacy requests. A provider record created
during the pilot will still exist after rollback. Data that must remain private
in both modes needs provider-enforced access controls or a source isolated
from legacy credentials. The add-on's private transcripts, settings and
ownership metadata stay in their own storage. Setup and rollout documentation
must distinguish the add-on's protections from the source's own guarantees.

Inventory both the short-lived relay bearer and the durable agent API key for
each external binding. The assigned key can authorize asynchronous replies.
Prove that a retained credential cannot exercise broader data access or send
a private result to an unauthorized destination on a covered binding. Isolate
or revoke broad credentials as needed. Verify initial ingress, retry ingress,
and late reply and edit paths independently. On disable, ordinary admissions issue credentials through the
existing contract. Old add-on credentials and retries cannot become legacy
requests. Independently held provider credentials at external agents remain
under the external runtime's control.

Hosted bots resolve pins and connected Sheets resources from their bound
assistant. An external relay receives separate event-bound reply and guest
tool credentials. Its guest tool endpoint currently supports the bot's own
personal-record collection. On enrolled generic external routes every channel
sender, including a person who owns the bot, uses this contract. The callback
contains text and scoped tokens. It omits attachments, raw provider data,
thread context and the owner bearer. After binding, its assigned durable key
loses general proxy and model use while enrolled. On a new external route,
activate the add-on only after setup assigns a compatible key explicitly
dedicated to that route. With a shared or incompatible key, the route keeps
ordinary behavior, setup shows the reason, and the key retains its existing uses.
A route already active under the add-on refuses work if its key loses eligibility.
A generic external route has no hosted assistant
or Sheets connection from which to resolve a grant. The external runtime
must use the guest credential and isolate any history or credentials it
manages itself. [External relay contract](CHANNEL_BOT_RELAY.md)

## Skill read authorization

Expose only the enabled, pinned bot package and its verified dependency files.
Implement the allowance at prompt preparation, native tool discovery, MCP
dispatch, preload selection, proxy authorization and the organization skill
capability. Guests retain their actor identity at every step. Verify current
bot, pin, file, audience-use rights and revocation for each read. Organization
packages require an Ornn authorization suitable for bot use.

When the owner requires approval for every request, a maintainer may explicitly
authorize standing bot reads for this audience and exact pinned attachment,
subject to the existing approval authority and the publisher's sharing rights.
Check that authorization before every fetch, including a preload. Without it,
the normal applicable approval is required. Replacing the pin or revoking its
authorization invalidates the standing read. Independent operation restrictions,
approvals and billing still apply. Test owner and guest against the same pin
with this setting enabled and verify the guest cannot read an unrelated package.

## Acceptance criteria

1. Record baseline outcomes on the current implementation for owner, guest and
   verified organization member, including chat admission, skill visibility,
   service levels and selected operations, tool calls, relay token scope, replies
   and billing. Include raw proxy, MCP, direct relay, native/gateway and retry
   paths where each is supported.
2. Run the same cases against the upgraded build with the flag off, first on a
   fresh bot and then on a bot with saved add-on settings after on → off. Compare
   permission decisions, API contracts, setup and downstream effects. Ignore
   nondeterministic IDs and generated prose. A previously working guest request
   completes using the existing implementation in both cases.
3. With the flag on, verify guest access to an on-demand skill and preload,
   explicit sharing, A's private CRUD, B's denial and private/group delivery.
   Include removal of a grant and a change of record ownership.
4. Disable between admission and dispatch, during a provider call, and before
   reply delivery. Send a fresh request while old work is settling. Verify a
   legacy response, no duplicate effect, no private context crossing modes,
   and correct accounting. Keep the old worker deliberately busy and prove the
   next fresh message in the same thread is admitted on its first ordinary
   opportunity. Record admission latency against the legacy baseline and time
   to stop stale workers against the bounds agreed in week 1. Re-enable and
   verify the saved configuration and rejection of stale work. Repeat across
   replicas and after worker restart.
5. Reject scoped flag writes, refuse enablement on an unready fleet, and
   exercise both explicit off and clearing the global override with a saved
   policy and pilot row. Test a failed global read on every acceptance
   transport. Measure normal relay ingress and retried ingress against their
   respective baselines, including replies authenticated by the durable key.
6. Exercise missing, empty, one-bot and explicit fleet selections. Include an
   unselected call binding. Compare off and outside-pilot ACK status, latency,
   batch diagnostics, transfer and deletion against baseline with add-on storage
   unavailable. Exercise Discord Interactions, Slack, WhatsApp, X, the managed
   dispatcher, Telegram and Lark/Feishu according to their receipt semantics.
7. Switch off → on with a pending mail retry and a delivered legacy callback
   still awaiting reply. Inject a transient failure after reply validation,
   before provider submission. The original reply succeeds once on retry.
   Include duplicate deliveries, capacity waits, restart, edit, partial provider
   acceptance and both JWT and assigned-key replies. Test real-shaped legacy
   API keys, scoped keys and login-child credentials through normal authentication.
8. Exercise the Discord event workflow on the baseline, with the flag off,
   outside the pilot, on with zero grants, on with explicit replacement grants,
   and after switch-off. Cover both Google catalogs, raw proxy and direct/wrapped
   MCP, a bounded read, a shared-tab append, owner edits, policy refusal and payer.
9. Test a use-only member, a maintainer, an excluded member and an external
   guest on the same organization specialist. Include revocation during queued
   work. Verify cross-guest and expired source-result references reveal no
   private metadata. Test two Meta inboxes with independent identities, reply
   windows and erasure, including with the feature disabled.
10. Compare a personal channel with guest media enabled on a supported relay in
    all modes, including a fresh image after switch-off. Check callback fields
    and general proxy/model key use for an enrolled external agent, a hosted bot
    and a second bot using a shared key.
11. Verify legacy and context-bound encrypted records through key rotation,
    migration and worker restart. A payload placed in the wrong event, field or
    ownership context is rejected. Test enablement across supported mixed
    versions and disable plus drain before reverting the binary. Compare the
    full backend suite on the integrated branch with the current baseline.
    Record fixture-backed UI results and authorized live provider runs separately.
12. Stage replacements for a working legacy Sheets grant and restrict a private
    pin before enrollment. Confirm ordinary behavior throughout preparation, a
    successful first enrolled Sheets request, and no exposure of the restricted
    pin on that first turn. Change ownership or a pin between preparation and
    activation and verify the stale draft never takes effect. Include a bot
    with no prepared revision and a legacy operation with no available replacement.
    Delete a staged bot, also through organization deletion, and verify that
    prepared grants, skill selections and audience references are erased.
13. Use one existing account with an old bot and a newly created bot on each
    supported creation path. Exercise global off, a named pilot, and fleet
    selection. The old bot keeps ordinary behavior until explicitly opted in.
    The new bot saves on and runs the add-on only while both gates allow it.
    Repeat with a new bot created during global off, an existing bot reconnected
    or transferred, and a retried create. With both bots processing requests,
    switch one off and on and verify its stale reply is suppressed, the other
    bot's reply completes, and the global switch still overrides both.
    Under fleet selection, create an external bot on an existing account and
    assign a key already in use. Verify the route reports ordinary mode and the
    existing key and bot still work. Assign a dedicated key and verify that only
    the new route activates.

Scope, sequencing, enrollment decisions and the revised effort estimate are in the
[implementation plan](plans/channel-bot-audience-boundaries.md).
