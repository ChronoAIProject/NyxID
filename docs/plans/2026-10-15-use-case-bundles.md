# Use-case bundles for NyxID onboarding

Status: implementation proposal grounded in the current checkout; no runtime
changes or live provider validation. Follow-up to the public/internal BotFather
split, whose implementation contract remains authoritative for existing bots.

Dependency: [PR #1901](https://github.com/ChronoAIProject/NyxID/pull/1901), the
public/personal Telegram manager runtime. This proposal remains draft pending
that dependency and reconciliation with its final verified linking, owner-turn
and continuation contracts. #1901 does not implement the catalog, interactive
multi-use-case sessions or specialist customization proposed here. Merging these
documents alone enables no runtime feature. The source-level compatibility
inspection cited elsewhere used `f8f623a42`; it is not validation of later PR heads.

Authoring decision: each use case is an onboarding SKILL.md followed by owner
NyxBot to connect services, create/reuse specialists and guide verification.
There is no separate YAML template. See [onboarding skills](../USE_CASE_TEMPLATES.md)
and the [three restaurant definitions](../RESTAURANT_USE_CASES.md). Existing
backend schemas/permissions remain authoritative; skills do not implement
missing provider or setup integrations. These files are not published/installed.

The [interactive NyxBot session contract](../NYXBOT_ONBOARDING_SESSIONS.md)
defines multi-selection entry links, one saved setup shared across its selected
use cases, the persistent conversation panel, and equivalent chat/card actions.

The [operating skill lifecycle](../USE_CASE_SKILL_LIFECYCLE.md) requires default
specialist procedures, owner-private copies and feedback-driven improvements.
Reuse existing authored review/publish/pin support; implement provenance and
execution gates so tested installation revisions follow the actual operating pins.

## Product contract

A user chooses one use case or a bundle, completes a guided setup, reviews the
exact behavior and access, tests it, and activates their bot. A restaurant bundle
contains three use cases: FAQ over Telegram/telephone, queue status with
staff-triggered SMS, and reservation intake over Telegram/telephone logged in
Notion. These are three configuration cards in one installation, not three
accounts or three customer bots. Default to three specialists, one per use case;
the queue specialist is private. Telephone is a shared channel. Current
single-agent installation binding requires an explicit routing extension to
support multiple specialists on one public bot/number.

Selecting the complete bundle requires all three cards and required channels to
pass readiness. Users
may explicitly select a smaller subset; show it as a partial suite. Never label
an installation fully ready while a selected required capability is unavailable.
Collect shared business facts and connect reusable services once.

The public BotFather explains NyxID, recommends supported use cases, collects a
bounded draft, and guides signup. Signing up establishes identity, not connection
consent. Authenticated owner setup selects personal connections/resources and
reviews permissions. Public guide context never receives personal inventory,
credentials, private history, or the resulting bot's execution authority.

## Existing NyxID foundations and gaps

| Concern | Existing foundation | Required extension |
| --- | --- | --- |
| Use-case guidance | Existing Ornn skills and native NyxBot setup/team tools | Publish onboarding skills and add authenticated handoff/durable setup tooling |
| Setup lifecycle | `PlatformSetup`, encrypted facts, revision/digest approval, provisioning and tests | Multiple selected capabilities, connection bindings and per-capability readiness |
| Installation | `PlatformInstallation`, single agent ID, child markers and generation fences | Bind approved capability/specialist composition and shared-channel routing |
| Specialists | `AssistantAgent.grants`, `guest_access`, `operation_scopes` | Compile reviewed capability requirements into explicit restrictions |
| Connections | `UserService`, `UserApiKey`, `AgentServiceBinding` | Resolve compatible connections and selected resources from live authorized inventory |
| Operation limits | `AgentOperationScope`, `ScopedOperation.inputs`, shared permissions engine | Provider-specific requirement mapping and business-level validation |
| Public customer runtime | `platform_customer_service` calls tool-free inference over published facts | A separately reviewed bounded operational runtime |
| Provisioning worker | `SetupStep` currently has BeginTelegram and ConnectTelegram | Typed additional steps and recovery for required provider resources |
| Phone providers | Twilio/ElevenLabs catalog specifications | Verify supported call direction, provider provisioning, callbacks and end-to-end behavior |
| Reservation/queue backend | Current restaurant template publishes a request link; Notion catalog spec exists | Implement bounded Notion intake and an authoritative queue-source adapter with staff SMS |

Presence of a catalog specification does not prove a configured provider or a
working deployment. No live catalog, credentials, queue backend, booking provider,
or phone delivery was verified for this proposal.

References: [Platform BotFather](../PLATFORM_BOTFATHER.md),
[operation scopes](../AGENT_OPERATION_SCOPES.md), [Tools](../TOOLS.md),
[NyxBot specialists](../chat/09-nyxbot-orchestrator.md).

## Definitions and persistence

Keep three concepts explicit:

1. An **onboarding skill** defines one outcome, questions, supported setup tools,
   service requirements, specialist role/access, dependencies and acceptance.
2. A **bundle** selects compatible onboarding skill versions and shared defaults.
   NyxBot guides the combined setup; backend code validates each exact requirement.
3. An **owner setup** records the user's choices and becomes the existing
   installation through its existing lifecycle.

Start with reviewed repository skill drafts, published through existing Ornn
support and explicitly attached to authenticated NyxBot. Use actual tool schemas
for structured updates and authoritative setup state for resume. Shared guidance
does not require a YAML loader. Model extraction is untrusted; backend validation
controls state. Do not invent tool endpoints, resource IDs or readiness evidence.
Compile operation rules server-side from reviewed live endpoint selections.

Proposed additive setup data:

```text
bundle_ref?: {id, version}
coordinator_conversation_id
focused_capability_id?
desired_revision, approved_revision, active_revision?
selected_capabilities: [{id, version, configuration_revision}]
onboarding_skill_pins: [{source, skill_id, version, sha256}]
specialist_refs: [{capability_id, agent_id}]
shared_facts and per-capability configuration: encrypted draft content
bindings: [{capability_id, requirement_id, user_service_id, resource_selection}]
steps: [{step_id, dependencies, state, action_ref, result_ref}]
compiled_plan: server-resolved operations, input restrictions, dependencies,
               limits, stable planned resource IDs, contract digests
capability_status: collecting | needs_connection | needs_review |
                   provisioning | needs_test | ready | unavailable | paused
readiness_evidence: capability/configuration/contract revisions, check, timestamp,
                    safe result and provider receipt reference where applicable
```

Store references to credentials through existing connection/binding mechanisms;
never copy secrets into templates, facts or readiness records. Do not put private
configuration in the existing public publication snapshot. Preserve current
person/org ownership conventions; first release remains personal-owned, matching
platform onboarding. No new business/tenant identity is required.

Legacy single-template rows retain their meaning. Introduce the additive
representation with an explicit schema/version strategy and enforcing-replica
rollout before enabling new bundles. Existing restaurant starter installations
remain FAQ/request-link installations and never silently acquire tools.

## Planning and authorization

The owner setup resolves the selected definitions and performs a deterministic
planning pass:

1. Validate shared facts and each capability's closed configuration.
2. Resolve dependencies, compatible providers, live service access and actual
   authorization scopes. Offer existing suitable personal connections.
3. Ask for missing connections, resource choices and unresolved policy conflicts.
4. Compile requirements to exact service/endpoint identities and supported input
   restrictions. Include budgets, public fields, guest behavior and provider
   contract revisions in the reviewed plan.
5. Present one review with a per-capability access breakdown. Approval binds the
   entire selected composition and existing setup/identity generations.
6. Provision through typed service-layer steps with durable idempotency and the
   existing approval fences; test each capability and the customer channel.
7. Activate only the reviewed, tested composition.

Reuse connections without broadening individual operations. Never combine
restrictions by independently unioning allowed resource IDs and actions: that
could authorize unintended action/resource pairs. Preserve each compiled
operation's input constraints and reject unresolved composition conflicts.

Operation scopes only narrow service grants. In the existing model an absent
scope means whole-service access, so an operational template must always emit
an explicit scope and fail if it cannot enforce it. Configuration currently
requires `assistant:operation-scopes`; report that deployment prerequisite
rather than bypassing the feature flag. Existing guest rules and owner approvals
remain additional gates; guests cannot obtain or spend owner approvals.

If a provider operation needs owner approval, use a pending request plus a
separate authenticated staff action. Setup consent does not override that gate.
Adding a capability, replacing a connection/resource, widening an operation or
upgrading a template requires a new reviewed revision. Successful readiness is
invalidated when relevant bindings/contracts/configuration change.

## Operational public runtime

Granting services to today's public specialist is insufficient:
`platform_customer_service` intentionally uses only approved publication, and
the provisioner creates that specialist with no service grants. Preserve this
mode and add an explicit operational installation mode for new reviewed setups.

The new mode resolves only activated capabilities and their exact approved
bindings. Reuse existing operation compilation/enforcement and credential
resolution through a purpose-scoped execution adapter. Do not impersonate a
human, attach the owner's full NyxBot, or route an opted-in installation through
the generic personal bot fallback.

Before choosing a NyxAgent guest-thread adapter versus a bounded business-tool
loop, complete an integration spike proving installation fences, isolated
customer context, restricted discovery, billing and revocation on that path.
Ordinary specialist authority alone does not establish installation authority.
The runtime design remains a required implementation gate, not an assumed
existing facility.

Every operation intersects live installation consent, active capability,
connection ACL, service grant, operation/input restrictions, guest rules and
applicable owner policy. Disable permission escalation from public customers.
Partition customer conversations; no shared private agent memory or account
context. Verify booking/queue ownership for lookup, update or cancellation.
Static scope rules do not substitute for these dynamic business checks.

Domain adapters enforce concurrency and idempotency: competing reservations
cannot overbook, duplicate messages cannot create duplicate entries/calls, and
ambiguous provider timeouts are reconciled rather than blindly retried. Return
success only from authoritative results. Recheck live consent before effects;
revocation stops subsequent dispatch, while already committed effects require
explicit business recovery rather than a claim that they were undone.

Phone is a channel plus an assigned job. Reuse the same reservation/FAQ handlers
for phone and Telegram. Declare inbound/outbound support separately and bind
numbers, purpose, duration/budget and permitted recipients as applicable.
Do not assume generic HTTP scopes can secure arbitrary phone scripts, streamed
audio or WebSocket providers; choose a bounded supported adapter. Existing scoped
services refuse WebSocket upgrades.

## Restaurant acceptance contract

| Use case | Setup requirements | Required evidence before ready |
| --- | --- | --- |
| FAQ agent | Reviewed facts, Telegram bot and inbound business phone integration | Correct published answers and unknown-fact fallback received in Telegram and on a real test call |
| Reservation manager | Telegram/phone, exact Notion destination and property mapping, business rules | Both sources log one pending request each; acknowledgements reflect verified writes; replay and uncertain writes recover safely |
| Queue manager | Authoritative queue source/location, staff management, verified contacts, SMS sender and return window | Staff sees live state; chosen entry gets the bounded SMS; duplicate intents, opt-out and delivery failure are handled |

Provider readiness and delivery evidence are separate. Read-only probes can run
automatically; test writes/calls require a sandbox or a clearly reviewed test
action and safe cleanup policy. Never book a real customer or place an arbitrary
call as a hidden readiness probe.

The full suite is active only when all selected requirements and delivery checks
pass. Subsequent connection expiry or withdrawal marks affected capabilities
unavailable and stops their dispatch. Independent capabilities may continue if
the approved composition permits it; the UI must show partial availability.

## Delivery slices

1. Publish/review onboarding and separate operating skills; add the versioned
   admin catalog, authenticated handoff, saved interactive sessions and reviewed
   specialist provisioning. Reuse hosted connection watches and existing authoring.
2. Deliver the private staff queue slice first: authoritative queue status,
   bounded SMS alerts, owner-private operating copy, exact contact/deadline
   handling, duplicate-send prevention and delivery receipts. Integrate owner
   feedback with existing authored publication, provenance, effective-pin checks
   and authorized regression tests. Implement actual execution suspension and a
   restricted test path before promising test-before-live updates.
3. Implement Telegram reservation intake into Notion, including bounded runtime,
   explicit destination restrictions, pending/staff behavior and reliable write
   recovery. Integrate activated operating skills into the bounded public FAQ and
   reservation runtime. Mark telephone pending until its end-to-end path is tested.
4. Add inbound telephone and reviewed shared-channel specialist routing, reusing
   FAQ/reservation handlers. Advertise the
   full three-use-case bundle only after every promised channel passes testing.

Required verification includes two-owner isolation, customer record isolation,
signup without implicit connection consent, absent/expired connections, feature
flag disabled, scope omission, provider contract drift, stale reviews, duplicate
events, concurrent capacity, worker restarts, capability removal/revocation and
full-suite versus partial activation. Run existing platform onboarding and
ordinary specialist regression suites plus real channel/provider staging tests.
Tests are proposed here, not executed or claimed passing.

## Decisions needed for the first operational slice

- Queue source/backend and how staff or customers populate entries and verified
  SMS contacts. Notion was requested for reservation logging, not assumed as the
  authoritative queue system.
- Staff-triggered queue SMS and reservations pending staff confirmation are
  documented working assumptions awaiting product confirmation. Auto-confirming
  bookings would need capacity management beyond the requested Notion log.
- Supported inbound telephone/voice provider, number onboarding/forwarding, and
  SMS sender/destination support. Phone here means receiving FAQ/reservation
  calls, not automatically placing outbound calls.

These decisions affect adapter implementation, not the reusable bundle/setup
model. No production configuration or active installation is changed by this
proposal.
