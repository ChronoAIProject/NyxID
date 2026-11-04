# Agent Key voice

An independently running device authenticates with its own credential from the
existing Agent Key login flow. It uses NyxID's existing Bot realtime voice path,
including the Bot's tools, private conversation context, confirmations and billing.
The implementation is opt-in behind `assistant:voice-agent-key` (default off).

## Authorization

The `nyxbot-voice` catalog contract is shared. **Each published connection is
bound to one personal Bot**, using a server-owned `UserService.source =
nyxbot_voice` and `source_id = agent.id`. Names and slugs do not select authority.
There is no hardcoded Bot, new token type, or separate permission table.

The owner explicitly creates a connection in that Bot's **Agent details → Device
voice** section, then selects that connection in the device Key's existing
service scope. Creating a connection does not add grants to Keys. A generic
connection to the catalog entry, ownership of a Bot, a catalog ID, or access to
`llm-nyx` grants no Bot voice access. Only a first-party human owner can publish;
the device cannot create or retarget its own connection. Shared and organization
Bots are unavailable in this first implementation.

For example, a Key allowed only Helper A's voice connection cannot discover,
start or control Helper B, including the owner's Full-access primary NyxBot.
Publishing another Bot later does not widen that selected-service Key. A Key
configured for **all services** includes current and future published connections
under NyxID's existing policy. Use selected service access for a dedicated board.
Auto-connected-only access does not include these explicitly published rows.

Within the selected Bot, execution uses that Bot's normal authority. Publishing
voice to the primary NyxBot therefore delegates its Full account/service access;
the UI states this before publication. Use a specialist with appropriate grants
when the device should have a narrower tool set. Source Key permissions are not
silently copied into or substituted for the Bot's authority.

The native Voice execution POST routes, including announcements, accept a
`proxy`-scoped Key and do not require account-management `write` scope.
Every operation checks the exact bound connection's existing service scope,
catalog operation policy and Key operation scope. Existing B1 operation scopes
still prohibit WebSockets. Discovery checks its own read operation and does not
require the start operation.

Voice forwarding honors existing explicit service Deny rules, including after
admission and before queued notification dispatch. This deny-only check creates
no approval request. Require-approval policies add no Voice access gate.
Voice forwarding adds no service-operation approval gate, pending request,
decision, resume or expiry proof. The exact Agent Key grant permits access to the
selected Bot's existing realtime Voice path; the Bot retains its own tools,
confirmations, permission checks and billing. Service-operation approvals for
unrelated catalog services keep their existing behavior. Neither Voice access nor
a remote speech notification can approve a Bot tool action.

A device Key does not authenticate the person speaking. Device calls never
decide owner action cards from speech. The call identifies the pending action
and asks the owner to review its existing NyxID card; the usual owner decision
resumes the same task. First-party human Voice retains its existing read-back
confirmation. Skill publication always requires reviewing the full draft card.

## Lifecycle

Calls bind the parent Key, exact child credential when used, Bot and service ID.
Key versions also advance on authentication usage metadata, so call ownership
compares stable identities and rechecks current permissions. Task transactions
write-fence the authoritative identity records from the transaction snapshot
before live resolution; concurrent authentication metadata causes a transaction
retry rather than a permanent permission denial.
Parent/sibling credentials cannot take over a child's call;
the browser owner retains control. Admission, runtime and queued work revalidate
live authority. Revocation ends the call and cancels pending requests. Removing
the service grant, disabling the connection or revoking the child's login closes
access. Voice-caused delegated turns and async operations retain their source
while using their own Bot's tool grants. Confirmed source loss stops pending
descendants, removes their pending report/decision events, and permits only bounded cleanup of already accepted async effects.
Temporary storage/readiness failures pause effects and preserve bounded retry.
Delayed report and decision event turns retain one exact Voice source; unrelated
owner/user/assignment turns leave those reports queued. Events from unrelated
owner work wait independently. This does not undo already
completed effects or stop unrelated owner work.

Native Responses calls are closed after worker recovery, never replayed. A
failed provider close remains unconfirmed, preserving provider identity and
observed usage for bounded reconciliation. It cannot mark final usage confirmed
or freeze response-token settlement. The live slot ends at the provider deadline;
reconciliation continues until its existing finalization deadline. Only reported
usage is charged when that deadline expires.

The service connection and its endpoint have a fixed binding and route. Owners
can disable/enable the same service without changing identity or selected-service
grants. Publishing an existing disabled connection returns that connection;
it does not enable it. Delete retires the live binding in the ordinary service
journal transaction; subsequent publication creates a fresh service ID and
requires a new grant on selected-service Keys. History retains the retired
identity. Publication is serialized on the existing Bot record.

## HTTP surface

All paths below are relative to `/api/v1/assistant/nyxagent`.

| Method | Path | Purpose |
| --- | --- | --- |
| GET, PUT | `/agents/{id}/voice-service` | Human owner reads/creates a connection; never grants a Key |
| GET | `/voice/agents` | Only Bots with an authorized bound connection, plus saved voice preferences |
| GET | `/voice/options?conversation_id=...` | Existing provider choices for an authorized Bot |
| POST | `/voice/sessions` | `{service_id, client_request_id, preferences, sdp_offer}`; legacy `agent_id` must match the service binding |
| GET | `/voice/sessions/{sid}` | Exact caller's session state |
| POST | `/voice/sessions/{sid}/control` | Existing control revision/command protocol |
| POST | `/voice/sessions/{sid}/announcements` | Bounded, idempotent remote speech notification; exact call owner and current grants required |
| GET | `/voice/sessions/{sid}/stream` | Existing authenticated voice WebSocket |

Existing conversation voice paths use the same checks. API Keys with no Origin
header may use native device routes; a supplied Origin must pass the existing
allowlist. Browser sessions retain Origin and session checks. Dictation, Bot
management and unrelated assistant routes remain unavailable to ordinary Keys.

## Deployment and board

Publish the [catalog template](../examples/nyxbot-voice-service.template.json)
through the normal catalog admin flow, host the matching
[OpenAPI contract](../examples/nyxbot-voice.openapi.json), and configure its spec
URL. The template, OpenAPI server and new endpoint use the same reserved
`https://nyxid.invalid/internal/nyxbot-voice` capability marker. The native API
and ordinary UserService proxy execute inside the current deployment; they never
connect to this URL or forward the device Bearer to another host. Outbound-only
executors reject this internal capability.

Upgrade **all API replicas and background workers**, then drain every old worker
before enabling the flag. Older workers ignore the additive source fields and
cannot enforce device revocation. Mixed-version activation is unsupported.
Activation now uses NyxID's existing security readiness roster: declare every
serving API/worker in `SECURITY_ROLLOUT_REPLICAS`, confirm
`SECURITY_ROLLOUT_DRAINED=true`, and wait until each process reports
`assistant:voice-agent-key`. Both flag enable and device admission require that
readiness check. A missing, stale, duplicate or incompatible generation blocks
activation. The roster remains an operator assertion; it cannot discover an
unregistered old process.

Disable device admission before rollback, end device calls, cancel/drain their
queued requests and descendant work (including async watches), and verify there
is no outstanding device source before starting a worker from before this
change. Disabling the flag alone does not make an old worker safe to consume
already-persisted device work. Native web calls also need to finish token
settlement before downgrading to code that does not understand native Responses.
No services are provisioned automatically and no production publication is
performed by this branch.

### Change and activation boundaries

1. **Native Voice and billing:** OpenAI Live's model authors the complete task;
   captions remain display data. This also changes web Voice and is independent
   of the device flag. Migration fills an absent task-model field on old OpenAI
   catalogs, including customized catalogs, preserving explicit model choices,
   explicit null and administrator pricing. Null offers dictation only; discovery
   reports `delegation_model_required` for full conversation, matching admission.
2. **Execution sources and recovery:** delegated turns/events/watches retain the
   initiating device authority independently of their reply destination. The
   common execution-source boundary translates the existing persisted fields;
   only a root continuation settles the original request. Native response state
   follows planned → active → settled and keeps the existing BSON field names.
3. **Device service and notifications:** ordinary UserService grants select an
   exact owned Bot. The device flag and fleet capability gate these entry points;
   they do not turn off the native web changes in the first boundary.

Native HTTP and internal proxy adapters call `voice::application`; HTTP
extractors, Origin checks and response encoding stay in the adapters. Resolved
proxy targets carry a typed execution kind, so forwarding does not infer internal
execution from a mutable slug or endpoint URL. A request reuses its fresh caller
identity; effects still fence live authority in their transaction.

Catalog read fences are per catalog and owner. Only the first registration
writes the catalog; steady-state device admissions write the owner's guard.
Catalog authority mutations touch registered guards in the same transaction,
including the normal catalog deactivation path. Registration also touches the
catalog, closing the race with a mutation that scanned before registration.
Registrations must not be deleted independently. Future catalog authority writers
must use `catalog_mutation_service` (bulk migrations must drain serving workers).
The guard index bounds the mutation lookup; mutation fanout grows with registered
owners, while unrelated owners no longer serialize on each admission.
The runtime reviews ongoing call authority every five seconds; task/control/
notification effects still perform fresh transactional checks. This is a polling
interval for an open call, not a cached grant for new work.

The board signs in once through Agent Key login, chooses its own restricted Key,
refreshes its authorized Bots and saves a selection. Changing the local selection
does not create a server grant. Only the board's child credential is stored;
no Mac session or provider credential is needed. Firmware currently supports the
saved `openai_live` WebRTC route with push-to-talk or automatic conversation. Actual provider audio requires
the deployed backend, published Bot connection, Key grant and saved voice setup;
local parser/SDP tests alone do not establish an end-to-end call.


## Remote speech notifications

`POST /voice/sessions/{sid}/announcements` accepts only `request_id` (UUID) and
`text` (nonblank, at most 300 UTF-8 bytes without control characters). It requires
a personal active Live call, exact caller ownership, the bound voice connection,
catalog operation policy and Key operation scope. Parent/sibling credentials
cannot mutate a child's call. Queue admission and dispatch fence current
Key/child/Bot/service authority with the live call generation and lease.

Each call admits up to 4 pending notifications and 128 total. A duplicate ID with
the same text returns its existing state; changed text conflicts. Unsent messages
expire after 30 seconds. The worker persists dispatch before network I/O and never
retries an uncertain send after a crash. Deduplication records expire after the
call deadline plus 24 hours. No notification is persisted as a user utterance or
accepted as a tool permission. The provider receives quoted data through
`session.commentary.append` with application-authored relay instructions.
`session.commentary.appended` acknowledges acceptance, not audible completion.
Snapshots expose only bounded request IDs and stages, not notification contents.

A panel may expose inbound daily and management services independently. Those
Bot-to-panel grants are separate from the panel Key's access to a Bot's voice.

## Conversation and concurrency contract

Discovery returns each exact Voice `service_id` and the Bot's existing home
`conversation_id`. Device starts use that home thread; web and other devices
can see the same private history. This release adds no device-private history or
implicit cross-Bot resume. The existing live-call unique slot is per person, so
a device call occupies the same slot as a browser call. Explicit continuation
requires the same parent and exact child credential; a sibling or parent cannot
take over. Human owners retain browser controls, including ending a revoked
device call.

Provider options expose `announcements`. It is true only for personal
OpenAI Live call options, and false for dictation, shared execution and other
protocols. Availability and voice support alone do not imply notification support.
