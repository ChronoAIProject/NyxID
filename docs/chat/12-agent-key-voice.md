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
catalog operation policy and service approval policy. As with other catalog-backed
connections, approval settings use the shared catalog ID; the Key must still hold
the exact Bot connection grant. An approval never grants access to another Bot.
Existing B1 operation
scopes still prohibit WebSockets. A policy that needs an approval fails closed;
this change does not implement a new pending service-approval UI. Browser owner
confirmation remains the Bot's existing confirmation flow.

## Lifecycle

Calls bind the parent Key, exact child credential when used, Bot and service ID.
Key versions also advance on authentication usage metadata, so call ownership
compares stable identities and rechecks current permissions. Task transactions
fence the freshly authorized Key version and bound service version.
Parent/sibling credentials cannot take over a child's call;
the browser owner retains control. Admission, runtime and queued work revalidate
live authority. Revocation ends the call and cancels pending requests. Removing
the service grant, disabling the connection or revoking the child's login closes
access. This does not undo already completed effects.

The service connection and its endpoint have a fixed binding and route. Owners
can disable/delete the service normally. Publishing after removal creates a new
service ID, requiring a new explicit grant on selected-service Keys. Publication
is idempotent while the existing connection is active and records its endpoint
and service in the normal service journal transaction.

## HTTP surface

All paths below are relative to `/api/v1/assistant/nyxagent`.

| Method | Path | Purpose |
| --- | --- | --- |
| GET, PUT | `/agents/{id}/voice-service` | Human owner reads/creates a connection; never grants a Key |
| GET | `/voice/agents` | Only Bots with an authorized bound connection, plus saved voice preferences |
| GET | `/voice/options?conversation_id=...` | Existing provider choices for an authorized Bot |
| POST | `/voice/sessions` | `{agent_id, client_request_id, preferences, sdp_offer}` |
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
URL. Deploy the backend before enabling the flag. No services are provisioned
automatically and no production publication is performed by this branch.

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
operation policy and the existing approval policy. Parent/sibling credentials
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
