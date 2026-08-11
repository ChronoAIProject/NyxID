# Assistant Chat

Last verified against Aevatar `0a86713671fcf551dc19ad86b1b6aa8ae6cb980b` and the
production typed-chat probe (2026-08-11).

This directory is the canonical specification for the browser assistant chat surface. It describes the contract implemented by NyxID's `/api/v1/assistant/**` routes, the React assistant client, and the Aevatar chat endpoints those routes call.

The live Aevatar contract wins over prose. If the deployed or pinned upstream contract and these documents disagree, fix the code or fix the document; never preserve two competing contracts.

## Reading order

1. [Architecture](01-architecture.md) defines typed `NyxIdChat` as the only Assistant send path, legacy history-only compatibility, ownership, authentication, and the Aevatar cutover gate.
2. [Wire contract](02-wire-contract.md) specifies every browser call, strict typed bodies, legacy read/delete resources, actor `/state`, fences, and retries.
3. [Stream protocol](03-stream-protocol.md) specifies SSE decoding, typed identity adoption, actor-projection convergence, terminal settlement, and transcript boundaries.
4. [Action cards](04-action-cards.md) specifies the v4 action envelope and `service.connect` lifecycle.
5. [Frontend UI](05-frontend-ui.md) specifies rendering, composer, navigation, loading, error, and accessibility behavior.
6. [Actions registry](06-actions-registry.md) specifies the public action manifest consumed by Aevatar composition.
7. [Testing and gaps](07-testing-and-gaps.md) identifies executable coverage, fault-injection controls, and current operational gaps.
8. [Mock scenario interception](mock-scenario-intercept-spec.md) specifies the implemented developer-only scripted-flow interceptor; its accepted adversarial findings are preserved in the [spec review](mock-scenario-intercept-spec.review.md).
9. [Mock scenario implementation plan](mock-scenario-intercept-plan.md) records the ordered work packages and verification gates; its accepted planning findings are preserved in the [plan review](mock-scenario-intercept-plan.review.md).
10. [Smooth text streaming](smooth-streaming.md) specifies the implemented text-reveal pipeline: the PR #1390 pacing controller, stable-prefix Markdown split and streaming caret, plus boundary-safe cuts and adaptive cadence spreading. It consolidates and supersedes the earlier exploratory plan and its adversarial review.

## Scope

The assistant chat surface is the authenticated browser experience at
`/assistant`. A human session calls NyxID, NyxID selects the platform-managed
`aevatar` service, and Aevatar owns typed actor execution and persistent
conversation history. The frontend sends only typed NyxIdChat commands.
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
- `backend/src/services/assistant_service.rs`: identifier validation, exact upstream paths, and typed command parsing/reconstruction.
- `backend/src/handlers/assistant_actions.rs`: public v4 action manifest.
- `backend/src/handlers/proxy.rs`: platform identity, delegation-token, and Authorization injection.
- `frontend/src/lib/assistant/aevatar-transport.ts`: HTTP calls, typed stream interpretation, actor-state reconciliation, retry, and action-card state.
- `frontend/src/lib/assistant/chat-stream-parser.ts`: SSE and AG-UI decoding.
- `frontend/src/hooks/use-assistant.ts`: query ownership, active-turn state, optimistic messages, projection, and cancellation.
- `frontend/src/components/assistant/**`: visible behavior and accessibility semantics.
- `frontend/e2e/**` and assistant unit tests: executable browser and transport contracts.

Upstream claims in this set are verified against Aevatar commit
`0a86713671fcf551dc19ad86b1b6aa8ae6cb980b`. The primary upstream anchors are
`MainnetChatEndpoints.cs`, `NyxIdChatPublicEndpoints.cs`,
`NyxIdChatConversationAguiFrameBuilder.cs`, `NyxIdChatServiceDefaults.cs`, the
NyxID tool-provider options, and Mainnet host configuration.
