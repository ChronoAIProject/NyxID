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
| POST | `/voice/sessions/{sid}/announcements` | Bounded, idempotent speech notification or generated Bot greeting |
| GET | `/conversations/{id}/voice-sessions/active` | Active call for this Bot conversation |
| GET | `/conversations/{id}/voice-requests/{rid}` | Existing task status |
| POST | `/conversations/{id}/voice-requests/{rid}/cancel` | Explicit task Stop |

The endpoint requires ordinary `proxy` scope. A supplied Origin must pass the
existing allowlist; devices may omit Origin. B1 operation-scoped Keys retain
their existing WebSocket restriction. Bodies are limited to 128 KiB. Notices
retain their existing 300-byte text bound and 30-second pending lifetime.
Provider acknowledgement does not prove audible playback.

The existing notification body remains `{"request_id":"UUID","text":"..."}`.
For an optional call-opening greeting, use `{"request_id":"UUID","greeting":true}`
without text. This is a server-authored call event: the selected Bot generates a
brief greeting using the call's identity, persona and language, then listens.
It is never a user utterance, tool request or permission. It uses the same queue,
limits, request-ID deduplication and live exact-Key/service authorization;
reusing an ID across notification and greeting content conflicts.
Authorized Bot discovery advertises `greetings:true`. Upgrade API and Voice
workers before enabling clients' generated greetings. Existing notification
requests and stored rows retain their meaning; absent stored `greeting` means
false. No browser greeting is enabled automatically.

Voice startup uses the same bounded display-name/persona renderer as ordinary
Bot conversations, alongside the existing role, language preference and recent
thread context. Persona shapes style only and never changes tool authority.

The service directly invokes the Voice application. It performs no HTTP request
to itself, forwards no caller Bearer, and adds no outer proxy usage charge.
Existing Voice runtime usage and settlement remain authoritative. It does not
register a generic proxy target execution type or alter MCP/Exact Service.
The [OpenAPI reference](../examples/nyxbot-voice.openapi.json) documents this
bidirectional protocol; do not import it as a set of ordinary MCP tools. Neither
catalog nor connection attaches a tool-discovery specification.

## Lifecycle and enablement

The feature `assistant:voice-agent-key` defaults off. On deploy, startup creates
the [catalog entry](../examples/nyxbot-voice-service.template.json) automatically
if no active or inactive `nyxbot-voice` row exists. Its base URL is this
deployment's `BASE_URL` plus `/api/v1/proxy/nyxbot-voice`. The seed never updates
an existing row, reactivates a disabled row, enables a flag, publishes a Bot
connection or grants a device access. Concurrent replica startup creates one
row. No spec or catalog `ServiceEndpoint` rows are created, and the service
stays out of the default connectable catalog (`include_all=true` includes it).

Runtime checks close audio and prevent new requests after parent/child revocation,
service removal or loss of the selected Bot. Task admission checks the current
binding in its existing transaction. As with browser Voice, call and task
lifetimes remain independent: ending audio or losing device access does not
cancel accepted Bot work. Explicit Stop follows the existing task cancellation
path. No device-source identity is propagated into MCP, child tasks or workers.

Old device firmware that called browser Voice paths must be updated to this
service endpoint. Browser paths continue to reject ordinary Agent Keys.

## Operator rollout runbook (#2247)

**Why the catalog returned 404.** #2169 required manual admin publication of
the template; production had no row. Deploying this seed creates the missing
row. `assistant:voice-agent-key` is already enabled in production for this
rollout; no flag change is required there. Existing installations with an
admin-edited or inactive row retain it exactly, including its base URL and
timestamps. An inactive row still returns 404; an admin must intentionally
reactivate it. Restarting is not an override of an admin decision.
If the seed fails, startup logs a metadata-only error and continues; Voice
publication keeps its existing "not published" error. Resolve the underlying
catalog/database problem and restart or redeploy to retry the insert.

**Prerequisites and flag authority.** The owner needs `assistant:nyxagent-engine`,
`assistant:voice`, and `assistant:voice-agent-key` enabled. Voice also needs a
configured, usable realtime provider and the owner's existing credential,
grant and billing admission. Platform OpenAI/Grok options retain their
separate provider rollout flags. Upgrade and drain old API/background workers
before enabling device voice on a new installation. Only a platform admin or
a role with `nyxid:feature_flags:admin` can enable these built-in flags; an
ordinary person, org admin or feature-flag creator role alone cannot. Admin
Feature Flags supports global, organization-cohort and individual-person
overrides. A person override takes precedence over the cohort/global result.
An org cohort enables eligible people, not org-owned Bots: this service is
still limited to personal owned Bots. The seed changes none of these checks.

**Owner publication and device grant.** In the first-party app, the owner opens
**Agent details → Access → Device voice** and publishes the desired Bot's
connection. Then grant that exact connection to the device Agent Key with
selected-service access and `proxy` scope (`read` too for `/keys` discovery).
Publication itself grants no access. An allow-all Key keeps ordinary all-service
semantics, so selected-service scope is the way to restrict a device to one Bot.
The Bot's grant to the board's own panel service is the opposite direction and
does not authorize the board to invoke Voice. No generic catalog connection
or catalog ID substitutes for the published connection's `service_id`.

**Verification.** Use a CLI profile for the target deployment. In these commands,
`BASE_URL` has no trailing slash, `OWNER_TOKEN` is the owner's first-party session
access token, `ADMIN_TOKEN` is an authorized flag reader's token, and `DEVICE_KEY`
is the device's ordinary Agent Key or login child credential. Keep credentials
out of logs. `BOT_ID` is the personal Bot UUID.

```sh
# Deployment created the active catalog row; default /catalog stays unchanged.
nyxid catalog show nyxbot-voice

# Inspect stored rollout overrides, then the owner's effective feature set.
curl -fsS "$BASE_URL/api/v1/admin/feature-flags" \
  -H "Authorization: Bearer $ADMIN_TOKEN" |
  jq '.flags[] | select(.key | startswith("assistant:voice")) |
      {key, default_enabled, global_override, org_overrides, user_overrides}'
curl -fsS "$BASE_URL/api/v1/users/me" \
  -H "Authorization: Bearer $OWNER_TOKEN" | jq '.enabled_features'

# After the owner publishes in Access → Device voice, service must be active.
# service:null means the owner has not published this Bot's connection yet.
curl -fsS "$BASE_URL/api/v1/assistant/nyxagent/agents/$BOT_ID/voice-service" \
  -H "Authorization: Bearer $OWNER_TOKEN" | jq '.service'

# Discovery must show only connections this device Key can access.
curl -fsS "$BASE_URL/api/v1/keys" \
  -H "Authorization: Bearer $DEVICE_KEY" |
  jq '.keys[] | select(.catalog_service_slug == "nyxbot-voice") |
      {id, catalog_service_slug, is_active}'

# Set SERVICE_ID to the exact published connection UUID, not the catalog UUID.
curl -fsS "$BASE_URL/api/v1/proxy/nyxbot-voice/$SERVICE_ID/voice/agents" \
  -H "Authorization: Bearer $DEVICE_KEY" | jq '.agents'
```

The final response must contain exactly the selected Bot's `agent_id` and the
same `service_id`. These checks prove catalog/publication/discovery authority;
an actual device voice session additionally verifies provider connectivity,
audio and existing billing admission. Provider acknowledgement alone does not
prove audible playback. To stop device access, revoke its Key grant or disable
the published connection; disabling the rollout flag also blocks new calls.
