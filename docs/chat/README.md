# Assistant Chat

Last verified against `1a7f6f6b` (2026-08-04).

This directory is the canonical specification for the browser assistant chat surface. It describes the contract implemented by NyxID's `/api/v1/assistant/**` routes, the React assistant client, and the Aevatar chat endpoints those routes call.

The live Aevatar contract wins over prose. If the deployed or pinned upstream contract and these documents disagree, fix the code or fix the document; never preserve two competing contracts.

## Reading order

1. [Architecture](01-architecture.md) explains ownership, authentication, configuration, and the two Aevatar engines.
2. [Wire contract](02-wire-contract.md) specifies every browser call, the body NyxID rebuilds, history resources, fences, retries, and create recovery.
3. [Stream protocol](03-stream-protocol.md) specifies SSE decoding, consumed AG-UI events, context adoption, terminal settlement, and transcript projection.
4. [Action cards](04-action-cards.md) specifies the v4 action envelope and `service.connect` lifecycle.
5. [Frontend UI](05-frontend-ui.md) specifies rendering, composer, navigation, loading, error, and accessibility behavior.
6. [Actions registry](06-actions-registry.md) specifies the public action manifest consumed by Aevatar composition.
7. [Testing and gaps](07-testing-and-gaps.md) identifies executable coverage, fault-injection controls, and current operational gaps.
8. [Mock scenario interception](mock-scenario-intercept-spec.md) specifies the implemented developer-only scripted-flow interceptor; its accepted adversarial findings are preserved in the [spec review](mock-scenario-intercept-spec.review.md).
9. [Mock scenario implementation plan](mock-scenario-intercept-plan.md) records the ordered work packages and verification gates; its accepted planning findings are preserved in the [plan review](mock-scenario-intercept-plan.review.md).
10. [Smooth text streaming](smooth-streaming.md) specifies the implemented text-reveal pipeline: the PR #1390 pacing controller, stable-prefix Markdown split and streaming caret, plus boundary-safe cuts and adaptive cadence spreading. It consolidates and supersedes the earlier exploratory plan and its adversarial review.

## Scope

The assistant chat surface is the authenticated browser experience at `/assistant`. A human session calls NyxID, NyxID selects the platform-managed `aevatar` service, and Aevatar owns chat execution and persistent conversation history. The frontend consumes typed NyxIdChat and Studio workflow streams through one transport abstraction.

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
- `backend/src/services/assistant_service.rs`: identifier validation, exact upstream paths, typed command parsing, and Studio body reconstruction.
- `backend/src/handlers/assistant_actions.rs`: public v4 action manifest.
- `backend/src/handlers/proxy.rs`: platform identity, delegation-token, and Authorization injection.
- `frontend/src/lib/assistant/aevatar-transport.ts`: HTTP calls, stream interpretation, fences, retry, recovery, and action-card state.
- `frontend/src/lib/assistant/chat-stream-parser.ts`: SSE and AG-UI decoding.
- `frontend/src/hooks/use-assistant.ts`: query ownership, active-turn state, optimistic messages, projection, and cancellation.
- `frontend/src/components/assistant/**`: visible behavior and accessibility semantics.
- `frontend/e2e/**` and assistant unit tests: executable browser and transport contracts.

Upstream claims in this set are verified against Aevatar commit `bbd906eb503a126c1a4b6a9ff67952cc819ccdd4`. The primary upstream anchors are `MainnetChatEndpoints.cs`, `NyxIdChatPublicEndpoints.cs`, `NyxIdChatConversationAguiFrameBuilder.cs`, the NyxID tool-provider options, and Mainnet host configuration.
