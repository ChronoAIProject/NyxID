# Assistant Chat Architecture

Last verified against Aevatar `e5bba2e9719ad5132004b882744caa3875db1123` and the
AC-2 authority probe (2026-09-02).

## System boundary

The browser assistant is a three-hop system with two mutually exclusive send
engines:

```text
React browser
  -> NyxID /api/v1/assistant/**
     -> default: admin-managed DownstreamService slug "aevatar"
        -> Aevatar /api/chat and conversation-history resources
     -> experimental: admin-managed DownstreamService slug "chrono-llm-public"
        -> Chrono LLM /v1/chat/completions (relative to the service base_url)
```

The browser owns presentation state, optimistic turns, stream consumption, and
engine selection from the effective feature flags. For Aevatar it also owns the
local mirror of actor-projected UI state. NyxID owns caller authentication,
platform-target selection, strict body reconstruction, and response
normalization. Aevatar owns typed actor execution, persistent history,
committed state versions, and turn identities. The Direct engine is stateless:
its `direct-*` conversations and transcripts exist only in browser memory, and
each turn resends the bounded local transcript.

The route graph is defined in `backend/src/routes.rs:build_router`. Aevatar
forwarding and resource-family switching are in
`backend/src/handlers/assistant.rs`; Direct forwarding is in
`backend/src/handlers/assistant_direct.rs`. Their request grammars live in
`backend/src/services/assistant_service.rs` and
`backend/src/services/assistant_direct.rs`. Browser engine routing is in
`frontend/src/pages/assistant.tsx:AssistantPage`; typed actor orchestration is in
`frontend/src/hooks/use-assistant-chat.ts:useAssistantChat` and
`frontend/src/lib/assistant/chat-stream-orchestrator.ts:runChatStream`; the
memory-only Direct seam is in
`frontend/src/hooks/use-assistant-direct.ts:useDirectAssistantChat` and
`frontend/src/lib/assistant/direct-transport.ts:DirectAssistantTransport`.

## Authentication boundary

All stateful assistant routes are nested beneath `/api/v1/assistant` in the human-only router. A verified `AuthUser` is required, and the router rejects:

- API-key credentials;
- service-account tokens;
- delegated tokens; and
- relay tokens.

This makes the assistant a human browser-session surface. The handlers receive the verified user identity after middleware and derive the Aevatar history scope from `AuthUser.user_id`. A caller cannot submit another user ID or scope in a request body.

`GET /api/v1/assistant/actions` is intentionally different. It is a public, static composition manifest mounted outside the authenticated assistant router. It contains no user or runtime secret data. See [Actions registry](06-actions-registry.md).

The route placement and rejection layers are authoritative: `backend/src/routes.rs:build_router`, `backend/src/mw/auth.rs:reject_api_key_tokens`, `reject_service_account_tokens`, `reject_delegated_tokens`, and `reject_relay_tokens`.

## Platform service selection

Assistant handlers do not resolve a user-owned `UserService`. The default
Aevatar handlers resolve the active admin-managed `DownstreamService` whose
slug is `aevatar`; the Direct handler uses `chrono-llm-public`. Both require the
row not to need a per-user credential and call the administrative proxy path.
This has several consequences:

- The caller cannot choose the upstream base URL.
- A personal or organization service with the same slug does not override the platform target.
- User connection state does not gate assistant access.
- User service scopes, node pins, and agent credential bindings do not select the assistant upstream.
- The upstream URL comes from the service row's `base_url`; NyxID has no assistant-specific Aevatar URL environment variable.

The selection is implemented by `backend/src/handlers/assistant.rs:forward` and
`backend/src/handlers/assistant_direct.rs:completions` through
`proxy_service::execute_admin_proxy`. The selected service row must be active
and must set `requires_user_credential` to `false`.

The Direct surface is default-off and independently enforced on every direct
route through the effective per-user feature resolution for
`experimental:direct-chat-engine`. When disabled, all three routes are
not-found-shaped and the frontend remains on Aevatar. The current implementation
does not include the endpoint selector, consolidated wire-log flag,
`/assistant/chat-config`, or gear panel proposed by the
[endpoint selector addendum](direct-chronollm-endpoints-addendum.md).

## Identity and capability chain

NyxID authenticates the human, selects the platform row, and supplies two different credentials to Aevatar when the row enables them:

1. An identity assertion proves who the human is. `identity_propagation_mode` must be `jwt` or `both`, and `identity_jwt_audience` must match Aevatar's expected audience, currently `urn:aevatar:api`. NyxID injects this assertion as `X-NyxID-Identity-Token`.
2. A delegated capability authorizes Aevatar to call back into NyxID tools for that user. `inject_delegation_token` must be `true`. NyxID injects the capability as `X-NyxID-Delegation-Token`.

The delegated capability scope comes from the service row's `delegation_token_scope` when that scope can call the REST proxy. An empty or LLM-only scope that cannot authorize the required REST surface falls back to `proxy` and emits one warning. The live token lifetime is the compile-time constant `backend/src/crypto/jwt.rs:MCP_DELEGATION_TOKEN_TTL_SECS`, fixed at 300 seconds. It is not an environment variable or an `AppConfig` field; changing it requires a code change.

The required live service-row configuration is therefore:

```text
slug:                         aevatar
active:                       true
requires_user_credential:     false
identity_propagation_mode:    jwt or both
identity_jwt_audience:        urn:aevatar:api
inject_delegation_token:      true
delegation_token_scope:       proxy or another REST-capable scope
forward_access_token:         true    <-- REQUIRED. Setting this false takes chat down.
```

Identity and delegated-capability construction live in `backend/src/handlers/proxy.rs`; assistant-specific forwarding and fallback scope selection live in `backend/src/handlers/assistant.rs:resolve_forward_scope` and `build_forward_authorization`.

### Transport authentication and capability selection

`forward_access_token` must be `true` on the live row. That flag never copies the
browser's inbound Authorization value. It enables the bridge in
`backend/src/handlers/assistant.rs:needs_forward_token_bridge`. For a
cookie-authenticated session, the bridge mints a NyxID delegated token and
overwrites the upstream `Authorization` header. A verified Bearer caller keeps
the verified Bearer. The assistant route policy excludes API keys, service
accounts, delegated tokens, and relay tokens before the handler runs.

Aevatar authenticates caller identity and selects tool capability separately. If
`X-NyxID-Identity-Token` is present, its dedicated authentication scheme
validates signature, issuer, audience, lifetime, subject, `iat`, and single-use
`jti`. The signed subject is authoritative for the Aevatar scope. The
`Authorization` header remains on the request for execution. A valid Bearer has
precedence as the execution credential and a delegated Bearer is classified as
`ProxyDelegation`. If Bearer is absent, `X-NyxID-Delegation-Token` can supply
that proxy-delegation capability. Identity alone is insufficient: typed
streaming returns 401 when neither execution credential is present.

The pinned sources are
`src/Aevatar.Mainnet.Host.Api/Responses/NyxIdIdentityAssertionAuthentication.cs`,
`NyxIdIdentityAssertionValidator.cs`,
`agents/Aevatar.GAgents.NyxidChat/NyxIdChatEndpoints.Streaming.cs`, and
`NyxIdChatEndpoints.cs` at Aevatar
`e5bba2e9719ad5132004b882744caa3875db1123`.

Keep `forward_access_token: true` until a deployed probe proves Aevatar receives
both a valid identity assertion and a usable capability through the replacement
path. Source support for the delegation header is necessary but is not a live
deployment receipt. Turning the bridge off before that proof can leave Aevatar
with caller identity but no authority to run tools.

`JWT_ASSISTANT_FORWARD_TTL_SECS`, `crypto/jwt.rs:generate_assistant_forward_access_token`, and the `assistant_forward` claim are compatibility tombstones. They preserve rejection behavior for a prior token shape and do not control live assistant authentication. New code must not depend on them.

## Aevatar configuration

Aevatar validates the NyxID identity assertion using:

- `Aevatar:Authentication:NyxIdIdentityAssertion:OidcDiscoveryUrl`;
- `Aevatar:Authentication:NyxIdIdentityAssertion:Issuer`;
- `Aevatar:Authentication:NyxIdIdentityAssertion:ExpectedAudience`; and
- `Aevatar:Authentication:NyxIdIdentityAssertion:MaximumLifetimeSeconds`.

The pinned Mainnet configuration expects `urn:aevatar:api`. NyxID tool callbacks use `Aevatar:NyxId:ApiBaseUrl`. Action-registry composition is controlled by `Aevatar:NyxId:AssistantActions:Enabled`.

The upstream source anchors are `src/Aevatar.Mainnet.Host.Api/appsettings.json`, `src/Aevatar.AI.ToolProviders.NyxId/NyxIdToolOptions.cs`, `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionsOptions.cs`, and `NyxIdAssistantActionRegistryStartup.cs` in Aevatar.

## Default Aevatar engine and legacy history

Aevatar's `POST /api/chat` classifies every request by the presence of a top-level `type` property. This dispatch rule is implemented in upstream `src/Aevatar.Mainnet.Host.Api/Chat/MainnetChatEndpoints.cs:ClassifyRequestAsync`.

| Request shape | Engine/resource family | Conversation IDs | Browser use |
| --- | --- | --- | --- |
| top-level allowlisted `type` | typed `NyxIdChat` actor | `nyxid-chat-{32 lowercase hex}` | every new send and typed continuation |
| no top-level `type` | Studio workflow compatibility resource | `chatc-...` | historical list, transcript, and delete only |

Every browser send is a typed command to `POST /api/v1/assistant/chat`; a first
`text` command omits `conversationId`, and a continuation includes the exact
previously observed `nyxid-chat-...` ID. NyxID rebuilds a closed, allowlisted
typed body and never forwards caller-supplied engine, scope, headers, or unknown
fields. An unknown explicit command type is rejected locally and cannot fall
through to Studio.

`chatc-...` is a legacy resource family, not an alternate chat engine. It may
remain visible in the shared history index and supports only its historical
transcript and delete resources. The browser never sends to it, creates a local
workflow placeholder, performs create recovery, uses a workflow WebSocket, or
falls back to it after a typed failure. Legacy parity is not a correctness goal.

The typed actor creates its public identity upstream as
`nyxid-chat-{Guid.NewGuid():N}`. The browser adopts it only from a valid
`RUN_STARTED`: top-level `actorId` and `turnId` are required, and
`runStarted.threadId` / `runStarted.runId`, when present, must exactly echo them.
The identity is immutable for that delivery; a missing or conflicting identity
is a protocol error, not a cache rekey or recovery opportunity.

The typed actor and Studio workflow remain mutually exclusive upstream. Within
the default Aevatar engine, the browser exposes only the typed send path;
retaining legacy read/delete does not permit a second Aevatar send path. The
separately feature-gated Direct engine is selected before dispatch and never
acts as a fallback after an Aevatar failure.

## Experimental stateless Direct engine

The implemented Direct engine is an internal-testing surface described in the
[Direct Chrono-LLM spec](direct-chronollm-spec.md). Its browser calls use
`POST /api/v1/assistant/direct/completions`; model and skill choices come from
the two flag-gated metadata routes. NyxID validates a closed text-only request,
prepends its server-owned base prompt and optional curated skill, forces
streaming with usage reporting, and forwards `chat/completions` to the
admin-managed `chrono-llm-public` row.

Direct conversations have `direct-*` identifiers used only by the frontend
engine router. They have no server-side list, history, state, or delete
resource. Reload, logout, or a verified identity change clears them. They do
not support tools, actions, approvals, task controls, attachments, Aevatar
actor state, or cross-engine fallback.

### Cutover gate

The deployment's typed plan gate is `mode: "auto"`; a locally auto-admitted
plan becomes `satisfied` with the reason "This plan contains only locally
auto-admitted operations." The browser still decodes the complete actor plan
and gate shape because a different deployment may require an explicit gate.

The typed actor currently terminates a connected-service inventory request with
`RUN_ERROR / USE_SKILL_ACCESS_DENIED`: its tool set omits the legacy
`nyxid_services` capability. That Aevatar provisioning defect blocks feature-
complete connected-service rollout, not deletion of the already-retired legacy
send path. It is not a reason to add a typed-to-workflow fallback or to claim
feature parity from record counts; the observed legacy
"117 connected services" count includes 75 inactive records.

## Persistence ownership

Aevatar exposes separate resource families:

- Typed rows are listed by canonical `/api/chat/conversations`; the scoped
  history index is authoritative only for retained legacy `chatc-*` rows.
- Typed history and delete use `/api/chat/conversations/{id}`.
- Legacy `chatc-*` history and delete use `/api/scopes/{scope}/chat-history/conversations/{id}`.
- Typed state is available only from `/api/chat/conversations/{id}/state`.

The transcript is authoritative for persisted text. It is not the authority for
typed pending input, approval, action, control, task, or continuation facts.
Those facts come only from the typed actor projection, which is updated by live
custom frames and conditional `/state` reads. A history response must not
overwrite that projection.

The projection recognises the complete public snapshot contract, including
`activeTurn`, `latestTurn`, `recentTerminalTurns`, `activeTask`, `taskStatus`,
`pendingInput`, `pendingApproval`, `pendingActions`, `recentActions`,
`latestInputResolution`,
`latestApprovalResolution`, `latestControlResult`, `latestStepControlResult`,
`recentStepControlResults`, `controlFence`, `continuationAdmission`,
`attentionKind`, `attentionSince`, `activeStepSummary`, `scopeId`,
`stateVersion`, and `progressSequence`. The deployment has
also returned additive `canaryEffectFault`, absent from the pinned canon.
Snapshot decoding therefore preserves recognised fields, ignores unknown
additive fields, and fails closed only at identity, version, and command-verb
boundaries. `stateVersion`, `progressSequence`, and operation generations must
be browser-safe integers; invalid values cannot advance local state.

On mount, reconnect, and after a relevant terminal or control acknowledgement,
the browser conditionally reads `/state?afterStateVersion=<current>`. It treats
`not_modified` as no state change, applies `current` only after validating the
envelope, scope, actor identity, and monotonic version/sequence, reloads on
`reload_required`, and treats `not_found` as a non-adoptable resource result.

## Failure boundaries

The architecture is deliberately fail closed at identity and engine boundaries:

- An unknown, malformed, or wrong-family conversation ID is rejected rather than guessed or forwarded.
- A typed command has one explicit allowlisted verb; unknown fields and verbs are rejected and cannot reach Workflow.
- `RUN_STARTED`, custom-frame payloads, and `/state` must agree on the adopted actor and turn identities. A stream cannot change either identity after adoption.
- State, progress, and operation generations must be safe integers and may advance only monotonically from actor-authored evidence.
- A typed upstream error, transport error, or stale control fence cannot retry through the legacy path.
- Attachments are unsupported for typed text. The browser rejects them at its boundary; NyxID sends no `inputParts`. Aevatar's reference client maps its own attachment affordance to `inputParts` after stripping `surface` and `attachment`, but that behaviour is not part of this contract.
- Action completion cannot report arbitrary resources or secret material.
- Workflow wire replay is retired. Session-only wire captures must not parse, fixture, replay, or label a workflow channel.

Detailed request, stream, and card failure behavior is specified in [Wire contract](02-wire-contract.md), [Stream protocol](03-stream-protocol.md), and [Action cards](04-action-cards.md).
