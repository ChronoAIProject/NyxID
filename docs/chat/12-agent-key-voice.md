# Agent Key voice service

An independent device uses its own ordinary Agent Key or login child credential
to access a published Bot Voice service. The service calls the existing Voice
application and realtime runtime. Browser Voice keeps its first-party human
session routes and authentication.

## Binding and authorization

The owner publishes one connection for a personal Bot through **Agent details →
Access → Device voice**. Its existing `UserService.source = nyxbot_voice` and
`source_id = Bot UUID` fix the selected Bot. Publication grants no Key access.
The owner then grants that exact connection to the board Key through ordinary
service scope. No new token type, permission table or hardcoded Bot is used.

A catalog ID, display name, generic catalog connection or `llm-nyx` grant cannot
select a Bot. A request cannot substitute another `agent_id` or `service_id`.
All-services Keys retain their ordinary semantics; use selected-service access
for a dedicated board. Organization and shared Bots are outside this service.

The two grant directions are independent:

```text
Board Key -> exact Bot Voice connection -> fixed Bot -> existing Voice runtime
Bot Key   -> board daily/management service -> authorized board operations
```

Calls retain the exact parent Key, login child credential, Bot and connection.
Another child credential cannot take over a call. Separate device logins must
use separate child credentials; copying the same secret does not establish
separate hardware identity. The human owner retains ordinary call control.

Every service operation checks current Key/service scope, the fixed Bot binding,
operation policy and explicit Deny rules. Voice access creates no service-operation
approval. Bot tools keep their existing grants and owner action confirmations;
a board service Key cannot decide an owner action card from speech. Device-key
voice is an external source: even if its owner is enrolled in execution
permissions, its sessions and delegated tasks retain legacy confirmations and
cannot inherit that enrollment or an execution-permission task root.

## Service URL and operations

The published endpoint is a real URL on the current deployment:

```text
{BASE_URL}/api/v1/proxy/nyxbot-voice/{UserService.id}
```

Devices discover authorized connections through existing `GET /api/v1/keys`,
select an active `catalog_service_slug = nyxbot-voice`, and use its exact ID.
The following paths are relative to that service URL:

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/voice/agents` | The one Bot fixed by this connection, with saved preferences |
| GET | `/voice/options` | Existing voice provider options; optional `conversation_id` must belong to this Bot |
| POST | `/voice/sessions` | Existing startup preferences, `client_request_id` and `sdp_offer` |
| GET | `/voice/sessions/{sid}` | Exact caller's call state |
| POST | `/voice/sessions/{sid}/control` | Existing revision/command controls |
| GET | `/voice/sessions/{sid}/stream` | Existing authenticated control WebSocket |
| POST | `/voice/sessions/{sid}/announcements` | Bounded, idempotent speech notification |
| GET | `/conversations/{id}/voice-sessions/active` | Active call for this Bot conversation |
| GET | `/conversations/{id}/voice-requests/{rid}` | Existing task status |
| POST | `/conversations/{id}/voice-requests/{rid}/cancel` | Explicit task Stop |

The endpoint requires ordinary `proxy` scope. A supplied Origin must pass the
existing allowlist; devices may omit Origin. B1 operation-scoped Keys retain
their existing WebSocket restriction. Bodies are limited to 128 KiB. Notices
retain their existing 300-byte text bound and 30-second pending lifetime.
Provider acknowledgement does not prove audible playback.

The service directly invokes the Voice application. It performs no HTTP request
to itself, forwards no caller Bearer, and adds no outer proxy usage charge.
Existing Voice runtime usage and settlement remain authoritative. It does not
register a generic proxy target execution type or alter MCP/Exact Service.
The [OpenAPI reference](../examples/nyxbot-voice.openapi.json) documents this
bidirectional protocol; do not import it as a set of ordinary MCP tools. Neither
catalog nor connection attaches a tool-discovery specification.

## Lifecycle and enablement

The feature `assistant:voice-agent-key` defaults off. Publish the
[catalog entry](../examples/nyxbot-voice-service.template.json) with this
deployment's base URL, deploy the service route, enable the feature, then publish
and grant a Bot connection. It creates no default Bot or device grant.

Runtime checks close audio and prevent new requests after parent/child revocation,
service removal or loss of the selected Bot. Task admission checks the current
binding in its existing transaction. As with browser Voice, call and task
lifetimes remain independent: ending audio or losing device access does not
cancel accepted Bot work. Explicit Stop follows the existing task cancellation
path. No device-source identity is propagated into MCP, child tasks or workers.

Old device firmware that called browser Voice paths must be updated to this
service endpoint. Browser paths continue to reject ordinary Agent Keys.
