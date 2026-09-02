# Assistant Chat

The Aevatar chat contract is pinned in
`tests/fixtures/assistant/aevatar-chat-contract-pin.json`.
Compare watched paths from the effective chat SHA to the live
`feature/integrate` head with:

```bash
python3 scripts/check-aevatar-chat-drift.py \
  --pin tests/fixtures/assistant/aevatar-chat-contract-pin.json \
  --remote https://github.com/aevatarAI/aevatar.git \
  --branch feature/integrate
```

Pin field sources at effective chat SHA `706ea7cab9d1f882e0fb0f034bb338102b6d5d2b`:

- `remote`, `branch`, `remote_head`, `effective_chat_sha`, `watched_paths`: this pin
- `public_commands`: `agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs`
- `internal_actions`: `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs` (`ResolveServiceAccessReview`)
- `context_attachments`: `agents/Aevatar.GAgents.NyxidChat/ConversationContextAttachmentAdmission.cs`, `NyxIdChatEndpoints.Streaming.cs` (`NyxIdChatContextAttachmentDto`, `ToAttachmentAdmissionWireName`), `NyxIdChatLifecycleFacade.cs` (create-only admission)
- `delete`: `agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs` (`HandlePublicDeleteConversationAsync`)
- `keepalive_seconds`: `agents/Aevatar.GAgents.NyxidChat/NyxIdChatEndpoints.Streaming.cs` (`StreamKeepAliveInterval`)
- `action_registry`: `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs` (`SupportedSchemaVersion`, per-action skip on unknown or divergent descriptors)

This directory is the canonical specification for the browser assistant chat surface. It describes the contract implemented by NyxID's `/api/v1/assistant/**` routes, the React assistant client, and the upstream chat endpoints those routes call. The default surface uses Aevatar's durable typed actor. A default-off feature flag can instead select the implemented stateless Direct Chrono-LLM engine for internal testing.

The live Aevatar contract wins over prose. If the deployed or pinned upstream contract and these documents disagree, fix the code or fix the document; never preserve two competing contracts.

## Reading order

1. [Architecture](01-architecture.md) defines the default typed `NyxIdChat` engine, flag-gated stateless Direct engine, legacy history-only compatibility, ownership, authentication, and the Aevatar cutover gate.
2. [Wire contract](02-wire-contract.md) specifies every browser call for both implemented engines, strict bodies, legacy read/delete resources, actor `/state`, fences, and retries.
3. [Direct Chrono-LLM spec](direct-chronollm-spec.md) is the detailed v3.2 contract implemented by the default-off `experimental:direct-chat-engine` surface.
4. [Endpoint selector addendum](direct-chronollm-endpoints-addendum.md) is a proposed spec v4 follow-up. Its consolidated flag, typed endpoint configuration, `chat-config` API, and gear panel are not implemented by the current branch.
5. [Stream protocol](03-stream-protocol.md) specifies SSE decoding, typed identity adoption, actor-projection convergence, terminal settlement, and transcript boundaries.
6. [Action cards](04-action-cards.md) specifies the v4 action envelope and `service.connect` lifecycle.
7. [Frontend UI](05-frontend-ui.md) specifies rendering, composer, navigation, loading, error, and accessibility behavior.
8. [Actions registry](06-actions-registry.md) specifies the public action manifest consumed by Aevatar composition.
9. [Testing and gaps](07-testing-and-gaps.md) identifies executable coverage, fault-injection controls, and current operational gaps.
10. [Mock scenario interception](mock-scenario-intercept-spec.md) preserves the design record of the superseded scripted-flow interceptor; assistant mocks now live at the HTTP boundary.
11. [Mock scenario implementation plan](mock-scenario-intercept-plan.md) preserves the superseded implementation record and its review findings.
12. [Smooth text streaming](smooth-streaming.md) specifies the implemented text-reveal pipeline: the PR #1390 pacing controller, stable-prefix Markdown split and streaming caret, plus boundary-safe cuts and adaptive cadence spreading. It consolidates and supersedes the earlier exploratory plan and its adversarial review.

## Scope

The assistant chat surface is the authenticated browser experience at
`/assistant`. A human session calls NyxID. With the default-off direct flag
disabled, NyxID selects the platform-managed `aevatar` service and Aevatar owns
typed actor execution and persistent conversation history. With the flag
enabled for that user, new drafts and `direct-*` IDs select the stateless Direct
engine and NyxID calls the platform-managed `chrono-llm-public` service instead.
Existing `nyxid-chat-*` and `chatc-*` routes remain on the canonical history
reader even while the flag is enabled.
Existing `chatc-*` rows are historical read/delete compatibility, never an
alternate send, stream, recovery, or control path.

This set does not specify:

- Channel bots, inbound channel events, or asynchronous channel replies. Those are separate webhook and relay surfaces under `/api/v1/channel-*`; see `docs/CHANNEL_BOT_RELAY.md` and `docs/CHANNEL_EVENT_GATEWAY.md`.
- The general downstream proxy at `/api/v1/proxy/**`, including node-routed and WebSocket proxy streaming. See `docs/PROXY_STREAMING_ARCHITECTURE.md` and `docs/NODE_PROXY_ARCHITECTURE.md`.
- The older standalone chatbot products described by `docs/NYXID_CHATBOT_SPEC.md` and `docs/CHATBOT_3RD_PARTY_INTEGRATION_SPEC.md`.
- Oracle browser-worker pools. See `docs/ORACLE_RELAY.md`.

The assistant backend uses shared proxy machinery for identity and delegation injection, but that implementation reuse does not make the assistant a user-selected proxy route. The platform-managed service row, server-derived identity scope, chat-specific body reconstruction, and human-only route policy are part of this contract.

## Source map

The normative implementation anchors are:

- `backend/src/routes.rs`: route placement and authentication policy.
- `backend/src/handlers/assistant.rs`: upstream selection, history multiplexing, request forwarding, and response normalization.
- `backend/src/handlers/assistant_direct.rs`: direct-route flag enforcement, strict body rebuild, rate-limit lifetime, and Chrono-LLM forwarding.
- `backend/src/services/assistant_direct.rs`: direct request grammar, models, prompts, and curated skills.
- `backend/src/services/assistant_service.rs`: identifier validation, exact upstream paths, and typed command parsing/reconstruction.
- `backend/src/handlers/assistant_actions.rs`: public v4 action manifest.
- `backend/src/handlers/proxy.rs`: platform identity, delegation-token, and Authorization injection.
- `frontend/src/lib/assistant/assistant-http.ts`: cookie-authenticated assistant HTTP, attributed 401 handling, HTTP fixture seam, and wire-log capture.
- `frontend/src/lib/assistant/chat-api.ts`: typed command bodies and canonical SSE response access.
- `frontend/src/lib/assistant/sse-frame-normalizer.ts`: incremental SSE framing and backend-frame normalization.
- `frontend/src/lib/assistant/runtime-event-semantics.ts`: console-compatible runtime event accumulation plus NyxID media artifacts.
- `frontend/src/lib/assistant/chat-actor-state.ts`: actor-fact decoding and versioned projection reduction.
- `frontend/src/lib/assistant/chat-history-decoders.ts`: strict index and wrapped transcript decoding.
- `frontend/src/lib/assistant/chat-stream-orchestrator.ts`: identity adoption, deadlines, watchdog, stream settlement, and state refresh.
- `frontend/src/hooks/use-assistant-chat.ts`: per-conversation session ownership, switching, history restore, and controls.
- `frontend/src/components/assistant/chat-message.tsx`: canonical message composition and tail following.
- `frontend/src/lib/assistant/direct-transport.ts` and `frontend/src/hooks/use-assistant-direct.ts`: the memory-only Direct seam.
- `frontend/src/lib/assistant/conversation-ids.ts`: legacy, typed, and Direct prefix routing.
- `frontend/src/components/assistant/**`: visible behavior and accessibility semantics.
- `frontend/e2e/**` and assistant unit tests: executable browser and transport contracts.

Upstream claims in this set are verified against
`tests/fixtures/assistant/aevatar-chat-contract-pin.json`.
The primary anchors are the console's chat page/API, SSE normalizer, runtime
event accumulator, actor-state reducer, history decoders, and the typed actor's
`NyxIdChatSseWriter`, `NyxIdChatProjectionSession`, and
`NyxIdChatCompletionAguiFrameBuilder`.
