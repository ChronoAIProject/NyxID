# Assistant Chat Architecture

Last verified against `fcb79b18` (2026-08-01).

## System boundary

The browser assistant is a three-hop system:

```text
React browser
  -> NyxID /api/v1/assistant/**
     -> admin-managed DownstreamService slug "aevatar"
        -> Aevatar /api/chat and conversation-history resources
```

The browser owns presentation state, optimistic turns, local structured blocks, stream consumption, and recovery orchestration. NyxID owns caller authentication, platform-target selection, scope derivation, body reconstruction, identity and capability injection, and response normalization. Aevatar owns the actor or workflow execution, persistent history, state versions, turn identities, and upstream stream semantics.

The route graph is defined in `backend/src/routes.rs:build_router`. The forwarding and resource-family switch are in `backend/src/handlers/assistant.rs`. Request grammar and exact upstream paths are in `backend/src/services/assistant_service.rs`. Browser orchestration is in `frontend/src/lib/assistant/aevatar-transport.ts` and `frontend/src/hooks/use-assistant.ts`.

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

Assistant handlers do not resolve a user-owned `UserService`. They resolve the active admin-managed `DownstreamService` whose slug is `aevatar`, require it not to need a per-user credential, and call the administrative proxy path. This has several consequences:

- The caller cannot choose the upstream base URL.
- A personal or organization service with the same slug does not override the platform target.
- User connection state does not gate assistant access.
- User service scopes, node pins, and agent credential bindings do not select the assistant upstream.
- The upstream URL comes from the service row's `base_url`; NyxID has no assistant-specific Aevatar URL environment variable.

The selection is implemented by `backend/src/handlers/assistant.rs:forward` through `proxy_service::execute_admin_proxy`. The service row must be active and must set `requires_user_credential` to `false`.

## Identity and capability chain

NyxID authenticates the human, selects the platform row, and supplies two different credentials to Aevatar when the row enables them:

1. An identity assertion proves who the human is. `identity_propagation_mode` must be `jwt` or `both`, and `identity_jwt_audience` must match Aevatar's expected audience, currently `urn:aevatar:api`. NyxID injects this assertion as `X-NyxID-Identity-Token`.
2. A delegated capability authorizes Aevatar to call back into NyxID tools for that user. `inject_delegation_token` must be `true`. NyxID injects the capability as `X-NyxID-Delegation-Token`.

The delegated capability scope comes from the service row's `delegation_token_scope` when that scope can call the REST proxy. An empty or LLM-only scope that cannot authorize the required REST surface falls back to `proxy` and emits one warning. The live token lifetime is controlled by `MCP_DELEGATION_TOKEN_TTL_SECS`, currently defaulting to 300 seconds.

The stable service-row intent is therefore:

```text
slug:                         aevatar
active:                       true
requires_user_credential:     false
identity_propagation_mode:    jwt or both
identity_jwt_audience:        urn:aevatar:api
inject_delegation_token:      true
delegation_token_scope:       proxy or another REST-capable scope
forward_access_token:         false
```

Identity and delegated-capability construction live in `backend/src/handlers/proxy.rs`; assistant-specific forwarding and fallback scope selection live in `backend/src/handlers/assistant.rs:resolve_forward_scope` and `build_forward_authorization`.

### Authorization is not caller passthrough

The configured steady state sets `forward_access_token` to `false`. The browser's inbound Authorization value is not copied to Aevatar. NyxID generates the identity assertion and delegated capability from the verified authentication context.

There is a compatibility bridge when `forward_access_token` is enabled on the service row. For a cookie-authenticated session, NyxID mints a standard delegated token and overwrites the upstream `Authorization` header; it does not forward an arbitrary browser-supplied value. A verified bearer caller can retain its verified bearer on that configuration, but the assistant route policy excludes API keys, service accounts, delegated tokens, and relay tokens before the handler runs.

`JWT_ASSISTANT_FORWARD_TTL_SECS`, `crypto/jwt.rs:generate_assistant_forward_access_token`, and the `assistant_forward` claim are compatibility tombstones. They preserve rejection behavior for a prior token shape and do not control live assistant authentication. New code must not depend on them.

## Aevatar configuration

Aevatar validates the NyxID identity assertion using:

- `Aevatar:Authentication:NyxIdIdentityAssertion:OidcDiscoveryUrl`;
- `Aevatar:Authentication:NyxIdIdentityAssertion:Issuer`;
- `Aevatar:Authentication:NyxIdIdentityAssertion:ExpectedAudience`; and
- `Aevatar:Authentication:NyxIdIdentityAssertion:MaximumLifetimeSeconds`.

The pinned Mainnet configuration expects `urn:aevatar:api`. NyxID tool callbacks use `Aevatar:NyxId:ApiBaseUrl`. Action-registry composition is controlled by `Aevatar:NyxId:AssistantActions:Enabled`.

The upstream source anchors are `src/Aevatar.Mainnet.Host.Api/appsettings.json`, `src/Aevatar.AI.ToolProviders.NyxId/NyxIdToolOptions.cs`, `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionsOptions.cs`, and `NyxIdAssistantActionRegistryStartup.cs` in Aevatar.

## Two mutually exclusive engines

Aevatar's `POST /api/chat` classifies every request by the presence of a top-level `type` property. This dispatch rule is implemented in upstream `src/Aevatar.Mainnet.Host.Api/Chat/MainnetChatEndpoints.cs:ClassifyRequestAsync`.

| Request shape | Engine | Conversation IDs | NyxID browser route |
| --- | --- | --- | --- |
| top-level `type` present | typed NyxIdChat actor | `nyxid-chat-...` | `POST /api/v1/assistant/chat` |
| no top-level `type` | workflow engine, pinned to Studio | `chatc-...` | `POST /api/v1/assistant/workflow-chat` |

The engines are mutually exclusive for a turn. A request cannot execute the workflow engine and typed actor together. NyxID reinforces this boundary by rebuilding both body families from strict request types: typed commands always include an allowlisted `type`; workflow turns cannot carry `type` and are rebuilt with `workflow: "studio"`.

### Engine selection in the browser

New browser conversations use the Studio workflow route. Before Aevatar assigns a durable ID, the frontend uses a local `workflow-pending-...` placeholder. The placeholder is routing state only and is never sent as an upstream conversation ID.

When the first stream provides `aevatar.chat.context`, the browser adopts the returned `chatc-...` identity and aliases local query, draft, episode, and navigation state to that durable ID. A stale `nyxid-pending-...` identifier is recognized only to handle older saved URLs as not found; it is not created by the current flow.

Existing `nyxid-chat-...` conversations continue through the typed NyxIdChat actor route. Typed action cards and action-result continuations therefore remain available for those histories. Existing `chatc-...` conversations continue through the Studio workflow route with a state-version fence.

The selection and identifier guards live in `frontend/src/lib/assistant/aevatar-transport.ts`, `frontend/src/lib/assistant/transport.ts`, and `backend/src/services/assistant_service.rs`.

## Persistence ownership

Aevatar owns both conversation families but exposes different resources:

- Both families appear in the scoped history index.
- Typed history and delete use `/api/chat/conversations/{id}`.
- Workflow history and delete use `/api/scopes/{scope}/chat-history/conversations/{id}`.
- Typed state is available from the actor state resource.
- Workflow continuation state is recovered from workflow history and stream context rather than the typed state route.

NyxID multiplexes these resources behind one browser API. The frontend treats the server transcript as authoritative for persisted text while retaining local structured action and approval blocks that the text-only history response cannot yet represent.

## Failure boundaries

The architecture is deliberately fail closed at identity and engine boundaries:

- An unknown or malformed conversation prefix is rejected rather than guessed.
- Workflow create and continuation shapes cannot be mixed.
- A workflow continuation without a positive state fence must recover one from history before sending.
- A create stream that does not establish a scoped durable identity enters bounded create recovery; it is not silently converted to a second create.
- A stream cannot change conversation or turn identity after adoption.
- Action completion cannot report arbitrary resources or secret material.

Detailed request, stream, and card failure behavior is specified in [Wire contract](02-wire-contract.md), [Stream protocol](03-stream-protocol.md), and [Action cards](04-action-cards.md).
