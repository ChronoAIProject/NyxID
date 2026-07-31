# Adversarial Review: Chat Parity Audit

Reviewed artifact: `docs/chat-aevatar-dev-parity-audit.md` at NyxID
`c6249bc2ed4e4abfa225cc0a3a6bbb5640ee9e45`.

Pinned comparison source: Aevatar `origin/dev` at
`bbd906eb503a126c1a4b6a9ff67952cc819ccdd4`.

This review treats the audit, the NyxID rollup documents, and earlier production
401 observations as claims to test, not as authority. The central D1 premise is
false at the pinned Aevatar revision. There is still a real resource-routing
defect, but it is narrower and different: NyxID routes every ID through the
typed canonical resource endpoints even after merging workflow `chatc-*` rows
into its list.

## 1. D1: existence and fallback behavior

### Verdicts

- **REFUTED -- `/api/chat/conversations/**` exists on no Aevatar branch.** One
  branch is enough to disprove the universal claim, and the exact pinned
  `origin/dev` branch maps all four routes:
  `origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs:24-30`.
  The same pinned host actually mounts them through
  `MapNyxIdChatPublicEndpoints()` at
  `origin/dev:src/Aevatar.Mainnet.Host.Api/Hosting/MainnetHostBuilderExtensions.cs:457-464`.
  The pinned canonical documentation also names this family at
  `origin/dev:docs/canon/chat-api.md:38-43`. D1's "phantom family" conclusion is
  incompatible with the source revision the audit says it inspected.

- **CONFIRMED -- NyxID's list fallback cannot rescue a non-success canonical
  response.** `list_conversations` forwards the canonical request at
  `backend/src/handlers/assistant.rs:414-422` and returns immediately on a
  non-2xx at `backend/src/handlers/assistant.rs:423-425`. The scoped
  `chat-history` fetch is reached only after the canonical body was both
  successful and JSON-parseable, at `backend/src/handlers/assistant.rs:426-465`.
  Therefore the audit's statement about the *control flow* is right, although
  its premise that pinned dev returns a route-level 404 is wrong.

- **REFUTED -- against pinned dev, list/transcript/state/delete all fail because
  the canonical family is absent.** The canonical list is a real typed list. It
  reads the shared history index and filters to the NyxIdChat service kind at
  `origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs:88-108`.
  Canonical typed detail returns `{messages,stateVersion}` at the same file's
  `111-135`; canonical state is implemented at `138-153`; canonical delete is
  implemented at `156-183`.

- **CONFIRMED -- there is nevertheless a workflow-family defect.** NyxID merges
  `chatc-*` rows from scoped history at `backend/src/handlers/assistant.rs:439-465`,
  but its detail, delete, and state handlers unconditionally build typed
  canonical paths at `backend/src/handlers/assistant.rs:486-558`. Canonical
  detail authorizes an ID as `GAgentActor` of
  `NyxIdChatServiceDefaults.GAgentKind` at
  `origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatEndpoints.cs:215-239`.
  The admission implementation resolves that typed registry target and returns
  not-found when it is not registered at
  `origin/dev:src/Aevatar.Studio.Infrastructure/ActorBacked/ActorBackedGAgentRegistryPorts.cs:94-130`.
  A workflow `chatc-*` history row is not thereby a typed NyxIdChat actor. Thus
  a merged workflow row can be listed and then fail when opened, deleted, or
  queried through the typed family. The bug is family confusion, not route
  nonexistence.

- **UNPROVEN -- current production mounting and auth order.** Pinned source
  proves the dev-shaped host contract, not what binary/configuration is live in
  production. A production 401 does not distinguish "route exists behind auth"
  from "global auth rejected before routing." An authenticated successful GET,
  an authenticated endpoint-list capture, or the exact deployed commit and
  host composition would settle production reality.

## 2. Shared Chat History and typed rows

### Verdict

**CONFIRMED.** Typed `nyxid-chat-*` conversations are deliberately written into
the same Chat History projection queried by
`/api/scopes/{scopeId}/chat-history`.

### Writer evidence

- The typed conversation actor initializes a Chat History conversation through
  `IChatHistoryCommandPort.InitializeConversationAsync` at
  `origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatConversationGAgent.cs:238-263`.
  Its outbox uses the typed actor ID as `ConversationId` and `ServiceId`, and
  `NyxIdChatServiceDefaults.GAgentKind` as `ServiceKind`, at the same file's
  `1749-1767`.

- Typed terminal delivery is sent through the same Chat History command port at
  `origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatConversationGAgent.cs:359-384`.
  Another typed completion path writes two stored messages plus typed service
  metadata with `SaveMessagesAsync` at
  `origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatGAgent.cs:499-533`.

- Typed lifecycle deletion unregisters the actor and then deletes its shared
  history row at
  `origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatConversationGAgent.cs:483-523`.

### Read-model evidence

- The shared index queries non-deleted `ChatConversationCurrentStateDocument`
  rows by scope, with no service-kind filter, at
  `origin/dev:src/Aevatar.Studio.Infrastructure/ActorBacked/ActorBackedChatHistoryStore.cs:43-94`.
  Therefore typed rows, workflow rows, and any other service kind using this
  model can all appear.

- Shared detail resolves the conversation document and returns ordered stored
  messages plus its projection `StateVersion` at
  `origin/dev:src/Aevatar.Studio.Infrastructure/ActorBacked/ActorBackedChatHistoryStore.cs:97-114`.
  The scoped HTTP response is exactly `{messages,stateVersion}` at
  `origin/dev:src/Aevatar.Studio.Hosting/Controllers/ChatHistoryController.cs:39-55`.

- The strongest corroboration is the typed canonical facade itself: it calls
  the shared `IChatHistoryQueryPort.GetIndexAsync` and then filters by typed
  `ServiceKind` at
  `origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs:88-108`,
  while typed detail reads the same query port at `111-135`.

### Consequence for N1

N1's assertion that shared history covers both ID families is correct, but its
reason for replacing all canonical resources is wrong. The correct mapping is:

| Resource | `chatc-*` workflow row | `nyxid-chat-*` typed row |
|---|---|---|
| Combined list | Fully page scoped `chat-history`, then strictly filter to the two supported families; alternatively fully page and merge the two sources | Same shared index is sufficient because typed rows are present |
| Transcript | Scoped `chat-history/conversations/{id}` (console contract) | Canonical `/api/chat/conversations/{id}` or the same shared detail; canonical preserves typed admission semantics |
| State | No workflow state route; use transcript `stateVersion` | Canonical `/api/chat/conversations/{id}/state` |
| Delete | Scoped history DELETE | Canonical `/api/chat/conversations/{id}` composite lifecycle DELETE |

The typed canonical delete must not be replaced by a manual actor/history pair.
It already calls `NyxIdChatLifecycleFacade.DeleteConversationAsync` at
`origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs:156-183`;
the actor command itself unregisters and deletes history at
`origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatConversationGAgent.cs:495-523`.

## 3. Independent console contract extraction

### Core command contract

- **CONFIRMED -- endpoint and JSON body.** The console posts to `/api/chat`,
  trims `commandId`, `prompt`, and `sessionId`, normalizes the strict
  `conversation` object, and pins `workflow: "studio"` at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatApi.ts:316-397`.

- **CONFIRMED -- headers.** It explicitly sets `Accept: text/event-stream` and
  `Content-Type: application/json` at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatApi.ts:381-397`.
  `authFetch` adds `Authorization: Bearer <accessToken>` when one is available
  at `origin/dev:apps/aevatar-console-web/src/shared/auth/fetch.ts:3-21`.
  The audit's "headers match" should be narrowed: NyxID matches content
  negotiation, while its backend-mediated identity/delegation chain is
  semantically equivalent platform plumbing, not byte-identical console auth.

- **CONFIRMED -- command ID only on create.** The page chooses `undefined` for
  an existing conversation, reuses the create ID for the same failed-create
  prompt, and mints a new one when that prompt changes at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:1232-1237`.
  The request call passes that optional value at `1281-1294`.

- **CONFIRMED -- session semantics, with an omitted NyxID deviation.** A new
  draft gets a new session UUID at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:126-135`.
  Reopening a history row constructs another fresh session UUID at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:868-899`.
  That value remains stable for turns in the active conversation. NyxID instead
  stores and reuses its session ID at
  `frontend/src/lib/assistant/aevatar-transport.ts:2348-2358`, including through
  index merges at `frontend/src/lib/assistant/aevatar-transport.ts:2119-2128`.
  The audit acknowledges this deviation but does not put it in N1-N6. Under an
  "exactly as the console" directive, it needs a decision or a fix.

- **CONFIRMED -- strict conversation input.** The console rejects extra keys,
  permits exactly `{conversationId:null}` for create, and requires a positive
  safe-integer `minimumStateVersion` for continuation at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatApi.ts:316-375`.

### Context, retry, and recovery

- **CONFIRMED with nuance -- context parsing.** The stream context parser
  accepts `stateVersion >= 0`, not only a positive value, at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatApi.ts:243-270`.
  Positivity is required only before a continuation. The page blocks a send and
  starts reconciliation if the active conversation lacks that positive fence
  at `origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:1187-1207`.

- **CONFIRMED -- reservation retry.** The helper uses delays `[300,900]`, only
  retries a continuation on HTTP 503 with code
  `CHAT_HISTORY_RESERVATION_UNAVAILABLE`, refreshes history before each retry,
  waits if the projection is below the prior fence, and raises the outgoing
  fence to the maximum at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatApi.ts:478-568`.

- **REFUTED -- create recovery is a single call after normal EOF.** The helper
  polls on 404 at cumulative attempts using delays `0/300/900/1800 ms` at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:288-305`.
  It runs after a normal EOF with no context at `1415-1435`, after an ambiguous
  possibly-accepted error at `1492-1515`, and in the background after an abort
  at `1601-1640`. N2's "call it once" is not console parity.

- **CONFIRMED -- SSE behavior, with a missed detail.** The reader ignores
  `[DONE]`, skips malformed JSON, handles CRLF, and joins multiple `data:` lines
  within one SSE event at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatApi.ts:576-635`.
  The audit mentions the first two behaviors but omits multi-line `data:`
  handling.

### History, model controls, and files

- **CONFIRMED -- cursor paging, no explicit page size.** The initial index path
  has no query, later paths carry only `cursor` at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatHistoryApi.ts:265-268`.
  The console follows every `nextCursor` and rejects loops at the same file's
  `299-326`.

- **CONFIRMED -- JSON Accept on history.** History GET and DELETE calls use
  `Accept: application/json` at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatHistoryApi.ts:13-15`,
  `275-284`, and `353-363`.

- **CONFIRMED -- `llmModel`/`llmRoute` are not sent by the Chat page.** The
  header builder exists at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatConversationConfig.ts:47-62`,
  but a pinned-tree grep for `buildConversationHeaders` shows its use in Studio
  at `origin/dev:apps/aevatar-console-web/src/pages/studio/index.tsx:3779`, not
  in `pages/chat`. Chat history still decodes those fields as display/session
  metadata at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatHistoryApi.ts:128-143`.

- **CONFIRMED -- the audited console Chat page sends no multipart/file input.**
  Its only start path serializes JSON at
  `origin/dev:apps/aevatar-console-web/src/pages/chat/chatApi.ts:377-397`.
  A pinned-tree grep for `FormData`, `multipart`, and file inputs under
  `pages/chat` returns no matches. File inputs exist in Studio surfaces, not
  this page. The upstream workflow model nevertheless supports `InputParts` at
  `origin/dev:src/workflow/Aevatar.Workflow.Infrastructure/CapabilityApi/ChatCapabilityModels.cs:73-85`,
  and Mainnet routes form content to workflow at
  `origin/dev:src/Aevatar.Mainnet.Host.Api/Chat/MainnetChatEndpoints.cs:43-50`.

## 4. Upstream mechanics in section 1.5

### Dispatch and deserialization

- **CONFIRMED -- dispatch is based on presence of `type`.** JSON with no
  `type` is workflow; one of seven exact strings is Assistant; unsupported
  content/type is 400. Multipart always selects workflow. Evidence:
  `origin/dev:src/Aevatar.Mainnet.Host.Api/Chat/MainnetChatEndpoints.cs:24-40`
  and `43-100`.

- **CONFIRMED -- unknown workflow JSON members are rejected.** `HttpChatInput`
  is annotated with `JsonUnmappedMemberHandling.Disallow` at
  `origin/dev:src/workflow/Aevatar.Workflow.Infrastructure/CapabilityApi/ChatCapabilityModels.cs:73-126`.
  The nested `ChatConversationInput` is independently strict at `128-133`.

### Idempotency and reservation

- **CONFIRMED -- request fingerprinting is create-only.** The fingerprint
  material includes normalized conversation, input parts, LLM controls,
  metadata, prompt, scope, session, and source at
  `origin/dev:src/workflow/Aevatar.Workflow.Application.Abstractions/Runs/WorkflowChatHistoryCreateRecoveryModels.cs:64-87`.
  Reservation uses it only for create and uses `string.Empty` for continuation
  at
  `origin/dev:src/workflow/Aevatar.Workflow.Application/Runs/WorkflowChatRunInteractionService.cs:214-255`.

- **CONFIRMED -- continuation delivery identity has the stated form.** It is
  `chat-history-delivery-{actorId}-{commandId}` at the same file's `229-243`.
  If the client omits `commandId`, the server generates one at
  `origin/dev:src/workflow/Aevatar.Workflow.Application/Runs/WorkflowChatRunInteractionService.cs:45-56`.

- **REFUTED as a safety conclusion -- empty fingerprint does not prove that
  reusing a continuation command ID is replay-safe.** Every continuation
  reservation creates a fresh `TurnId` at
  `origin/dev:agents/Aevatar.GAgents.ChatHistory/ChatTurnHistoryTerminalDeliveryPort.cs:61-64`.
  Reusing a command ID addresses the existing delivery actor, which accepts a
  repeated reservation only when all fields -- including `TurnId` -- match at
  `origin/dev:agents/Aevatar.GAgents.ChatHistory/ChatTurnHistoryDeliveryGAgent.cs:52-62`
  and `432-445`. Thus an identical client request can conflict after acceptance
  even though it avoids the create fingerprint's named 409 path. The audit's
  narrower statement "no create-fingerprint 409" is true; its implied replay
  safety is not.

- **CONFIRMED -- the projection fence is `current < minimum`, plus a missed
  readiness condition.** The actual reader rejects nonpositive minima, returns
  not-ready when `document.StateVersion < minimumStateVersion`, and also returns
  not-ready when the projected execution context has zero messages at
  `origin/dev:src/Aevatar.Studio.Infrastructure/ActorBacked/ProjectionChatConversationContinuationAdmissionReader.cs:20-55`.
  The audit's citation to `StudioWorkspaceGAgent` is the wrong mechanism.

- **CONFIRMED -- missing/nonpositive continuation watermark also maps to the
  reservation error before actor resolution.** Evidence:
  `origin/dev:src/workflow/Aevatar.Workflow.Application/Runs/WorkflowChatRunInteractionService.cs:397-406`.

- **CONFIRMED -- 503 and body code.** `ChatHistoryReservationUnavailable`
  maps to 503 and `CHAT_HISTORY_RESERVATION_UNAVAILABLE` at
  `origin/dev:src/workflow/Aevatar.Workflow.Infrastructure/CapabilityApi/ChatRunStartErrorMapper.cs:9-30`
  and `34-55`. The mapper constructs `{code,message}` at `70-83`; pre-stream
  failures use it at
  `origin/dev:src/workflow/Aevatar.Workflow.Infrastructure/CapabilityApi/ChatEndpoints.cs:348-363`
  and serialize JSON at `974-986`.

## 5. D3/N3: does NyxID lose the 503 code?

### Verdict

**REFUTED.** No backend change is required merely to preserve this code. The
body reaches the current frontend stream-start path.

### Trace

1. Assistant `forward()` calls `execute_admin_proxy` without translating the
   response at `backend/src/handlers/assistant.rs:330-401`.
2. The proxy preserves the upstream status and allowlisted response headers at
   `backend/src/handlers/proxy.rs:2622-2665`. Its buffered path builds the
   response from the original bytes at `backend/src/handlers/proxy.rs:2963-3016`.
   If an unknown-length JSON error takes the streaming branch, the bytes are
   still streamed unchanged; `should_stream_response` is selected at
   `backend/src/handlers/proxy.rs:3399-3425`.
3. The Web Worker reads every non-OK body as text and posts status plus body at
   `frontend/src/lib/assistant/chat-stream.worker.ts:86-117`.
4. The worker client preserves it in both worker mode at
   `frontend/src/lib/assistant/chat-stream-worker-client.ts:181-207` and inline
   fallback mode at `309-332`.
5. `streamStartError` already parses both NyxID and Aevatar envelopes, including
   `{code,message}`, at
   `frontend/src/lib/assistant/aevatar-transport.ts:843-891`.

The actual FE defect is ordering: the retry loop treats every retryable status,
including every 503, as a blind replay before parsing its body at
`frontend/src/lib/assistant/aevatar-transport.ts:2397-2425`. N3 should inspect
the first 503 body, refresh only for the named continuation error, and leave
other 503s alone. Add an end-to-end transport test proving the exact upstream
JSON body survives BE pass-through; do not add redundant backend error wrapping.

## 6. D4/N4: fabricated watermark and liveness

### Verdicts

- **CONFIRMED -- the floor of one exists.** `workflowTurnBody` emits the stored
  positive version or `1` at
  `frontend/src/lib/assistant/aevatar-transport.ts:2318-2339`. A regression test
  explicitly pins that behavior at
  `frontend/src/lib/assistant/aevatar-transport.test.ts:6223-6251`.

- **CONFIRMED -- zero is a normal create context value.** A create has no prior
  `ConversationContext`, so the terminal delivery port emits context
  `stateVersion` zero at
  `origin/dev:agents/Aevatar.GAgents.ChatHistory/ChatTurnHistoryTerminalDeliveryPort.cs:92-105`.
  NyxID discards context versions that are not positive at
  `frontend/src/lib/assistant/aevatar-transport.ts:3184-3189`.

- **CONFIRMED -- NyxID can have no usable transcript watermark immediately
  after create.** It optimistically appends the user message before dispatch at
  `frontend/src/lib/assistant/aevatar-transport.ts:1304-1367`. Post-turn cache
  projection is explicitly best-effort/non-throwing at
  `frontend/src/hooks/use-assistant.ts:101-145` and is scheduled on terminal at
  `frontend/src/hooks/use-assistant.ts:539-573`. A 404 after terminal is treated
  as "no server transcript yet" and falls back to the local mirror at
  `frontend/src/lib/assistant/aevatar-transport.ts:1270-1289`, which still has no
  positive stored watermark. The upstream admission reader also requires at
  least one projected message, not merely version one
  (`ProjectionChatConversationContinuationAdmissionReader.cs:48-54`).

- **UNPROVEN -- N4 as written permanently deadlocks the conversation.** N4 is
  a plan, not code. A later manual retry could re-read after projection catches
  up. But the plan is underspecified enough to create a practical stall: a
  first continuation immediately after create can get a 404/no watermark,
  surface "synchronizing," and have no bounded background reconciliation to
  make the composer ready. Worse, if the check occurs after `sendMessage`'s
  optimistic append, each failed attempt can leave a duplicate local message.

### Required N4 correction

Perform the watermark preflight before optimistic append. Reconcile with
bounded `0/300/900/1800 ms` list/detail polling, require a positive
`stateVersion`, and when a preceding turn ID is known require that turn's
assistant message to be present. The console does exactly this kind of bounded
reconciliation at
`origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:1009-1145` and
blocks sends while pending at `1187-1207`. Expose a retryable synchronizing
state after the bound is exhausted; do not manufacture a fence.

## 7. D5: continuation `commandId`

### Strongest case for keeping it

**CONFIRMED as a motivation, UNPROVEN as a guarantee.** A stable client command
ID addresses the same continuation delivery actor
(`WorkflowChatRunInteractionService.cs:229-253`). If an HTTP failure is known to
have occurred before reservation, reusing that identity could avoid allocating
unbounded delivery identities and could support deduplication if the server had
a complete continuation replay contract. NyxID's current loop also freezes the
body, so the intended use is internally coherent.

### Strongest case against keeping it

**CONFIRMED.** Calvin's directive is exact console parity, and the console
omits the member (`index.tsx:1232-1237`, `1282-1294`). More importantly, the
server does not currently prove the desired guarantee: every continuation
reserve mints a new turn ID (`ChatTurnHistoryTerminalDeliveryPort.cs:61-64`),
while an existing delivery actor accepts replay only when that turn ID and all
other fields match (`ChatTurnHistoryDeliveryGAgent.cs:432-445`). Reusing the
command ID after an accepted-but-truncated stream can therefore conflict rather
than deduplicate. Using a fresh/generated command ID instead would risk a
duplicate turn if NyxID continued ambiguous automatic retries.

The console resolves that tradeoff by not replaying ambiguous accepted
continuations. It retries only the definitive pre-admission reservation 503;
for an ambiguous continuation it tells the user to reload before continuing at
`origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:1517-1526`.

### Recommendation

**Drop `commandId` from continuations.** Remove generic automatic continuation
replay for network errors, arbitrary 5xx, and post-start truncation. Retry only
503 `CHAT_HISTORY_RESERVATION_UNAVAILABLE` after history refresh; that failure
occurs before a stream is accepted. Keep create `commandId` for create identity
and recovery, but also replace NyxID's automatic ambiguous create replay with
the console's recovery lookup. A user-initiated retry of a failed create may
reuse the same create ID and frozen create body.

## 8. Missed findings and observable differences

### Pagination silently truncates history

**CONFIRMED.** The console follows all cursor pages
(`chatHistoryApi.ts:299-326`). NyxID performs one FE list GET at
`frontend/src/lib/assistant/aevatar-transport.ts:1083-1096`, and its list type
does not even retain `nextCursor` at `frontend/src/lib/assistant/aevatar-transport.ts:341-374`.
The BE makes one canonical request and at most one synthetic shared-index
request (`backend/src/handlers/assistant.rs:414-465`). Both upstream lists
default to 50 rows:
`origin/dev:agents/Aevatar.GAgents.NyxidChat/NyxIdChatPublicEndpoints.cs:88-100`
and
`origin/dev:src/Aevatar.Studio.Hosting/Controllers/ChatHistoryController.cs:22-36`.

This is worse for the typed canonical list because it filters service kind
*after* obtaining a shared raw page but still returns that raw page's cursor
(`NyxIdChatPublicEndpoints.cs:98-108`). NyxID must drain cursors, detect loops,
filter every page, deduplicate, and preserve ordering. A 51+ mixed-row test is
required.

### DELETE semantics are not interchangeable

**CONFIRMED.** Canonical typed DELETE dispatches the composite lifecycle and
returns 202 with a JSON body (`NyxIdChatPublicEndpoints.cs:156-183`). Scoped
history DELETE deletes only the history conversation and returns bare 200
(`ChatHistoryController.cs:73-87`; command implementation at
`ActorBackedChatHistoryStore.cs:200-215`). Using scoped DELETE for a typed row
would leave the actor/registry resource behind.

There is also a NyxID client bug hidden in N1: `apiClient` parses JSON for every
successful status except 204 at `frontend/src/lib/api-client.ts:140-150`.
Therefore a proxied bare `200 OK` from workflow history DELETE can be reported
as a JSON parse failure after the deletion succeeded. The BE should normalize
that success to 204, or the assistant delete helper should accept an empty 2xx
body.

### Create ordering and turn-count reconciliation

**CONFIRMED, omitted from N2.** The console adopts recovery identity, increments
`expectedTurnCount` by one, and stores the returned `turnId` at
`origin/dev:apps/aevatar-console-web/src/pages/chat/index.tsx:307-317`. Stream
context follows the same ordering at `1348-1360`. Reconciliation does not trust
mere index visibility: it verifies an assistant transcript message with that
turn ID, then requires a positive/non-regressing state version at `1069-1081`.

The count itself is consistent with one per turn: the projection sets
`MessageCount = state.Turns.Count` at
`origin/dev:src/Aevatar.Studio.Projection/Projectors/ChatConversationCurrentStateProjector.cs:51-70`.
There is no two-messages-per-turn count bug, but N2 must copy the turn-observed
ordering guard, not merely adopt an ID from the first recovery response.

### Prompt trimming is broader than D6 says

**CONFIRMED, and the audit understates it.** The backend computes `prompt =
request.prompt.trim()` but checks maximum length against the untrimmed original
and serializes the untrimmed original at
`backend/src/services/assistant_service.rs:1111-1158`. It therefore differs
both in bytes and at the maximum-length boundary. N5 must validate
`prompt.chars().count()` and serialize `prompt`, not only change the emitted
field.

### History Accept headers differ

**CONFIRMED.** Console history calls send `Accept: application/json`
(`chatHistoryApi.ts:13-15`, `275-284`, `353-363`). NyxID's generic request
builder defaults only `Content-Type: application/json` at
`frontend/src/lib/api-client.ts:52-80`, and `assistantApi.get` adds no Accept at
`frontend/src/lib/assistant/aevatar-transport.ts:90-96`. This probably does not
change Aevatar's result today, but it is observable and easy to align locally
without modifying the global API client.

### Error envelope handling differs by path

**CONFIRMED.** Stream-start handling correctly accepts Aevatar `{code,message}`
(`aevatar-transport.ts:843-891`). Ordinary resource calls use `apiClient`, which
casts the body to NyxID's `{error,error_code,message}` shape and stores
`response.error_code` at `frontend/src/lib/api-client.ts:9-20`, `85-96`, and
`140-144`. An Aevatar `{code,message}` retains the message and status but loses
the symbolic code on those paths. Recovery and retry tests must include empty
404 bodies and Aevatar-style JSON bodies rather than only NyxID envelopes.

### Session reminting remains an exactness gap

**CONFIRMED.** As shown in task 3, the console remints `sessionId` on history
reopen (`index.tsx:868-899`), while NyxID preserves it in the stored
conversation (`aevatar-transport.ts:2119-2128`, `2348-2358`). The audit calls
this a deliberate deviation but N1-N6 never seek Calvin's approval for it.
Either align it or explicitly narrow "exact parity" to exclude session
correlation semantics.

### NyxID retries more than the console

**CONFIRMED.** NyxID retries network errors and all statuses in
`[408,425,429,500,502,503,504]` at
`frontend/src/lib/assistant/aevatar-transport.ts:196-200` and `2397-2437`, and
can retry a stream after it started. The console's automatic POST retry is only
the named reservation failure; ambiguous create uses recovery and ambiguous
continuation requires reload. N3's statement that creates keep frozen-body
automatic replay is not console parity.

### Workflow WebSocket twin is unused by the console page

**CONFIRMED.** A pinned-tree grep for `WebSocket`, `/api/ws/chat`, and
`workflow-chat` under `apps/aevatar-console-web/src/pages/chat` returns zero
matches. The page uses only `startChatStream`'s HTTP fetch
(`chatApi.ts:377-397`). Keeping the WS twin out of this parity change is correct.

## 9. N1-N6 as an implementation plan

### N1 -- **REFUTED / REWRITE**

Do not describe canonical resources as nonexistent and do not restore manual
typed dual-delete. Use the family-aware table in task 2. Fully page the shared
index and filter it. Route workflow detail/delete through scoped history; route
typed state/delete through the mounted canonical facade. Handle the workflow
DELETE's empty 200 response. The current backend guard at
`backend/src/services/assistant_service.rs:1307-1359` must change because it
asserts canonical resources and forbids the needed scoped transcript string.
However, the FE guard does **not** enforce the phantom resource family: it only
forbids old per-conversation command suffixes at
`frontend/src/lib/assistant/canonical-command-guard.test.ts:6-35`. The audit's
claim that both guards encode D1 is false.

### N2 -- **CONFIRMED direction, incomplete behavior**

Add the scoped recovery pass-through, but copy the console's 404 polling across
normal EOF, possibly accepted failure, and abort. Validate command IDs and
validate every returned identity before adoption. Then reconcile until the
returned `turnId` appears in transcript with a positive/non-regressing fence.
A single call is insufficient and observably unlike the console.

### N3 -- **CONFIRMED direction, wrong backend premise and retry scope**

The body code already survives the BE. Change FE decision order and test it.
Retry only a continuation with the exact 503 code after transcript refresh.
Do not retain blind create replay under a "console parity" label; use create
recovery. Do not automatically replay an ambiguous accepted continuation.

### N4 -- **CONFIRMED goal, liveness steps missing**

Remove the floor, but add bounded pre-send reconciliation before optimistic
append. Include the normal create-context-zero/no-history-row window and the
upstream zero-projected-messages condition in tests. Ensure exhausted sync is a
retryable state and that retrying does not duplicate the local user message.

### N5 -- **CONFIRMED trim, REFUTED command-ID recommendation**

Trim before both length validation and serialization. Omit continuation
`commandId` at both layers: the FE currently includes it at
`frontend/src/lib/assistant/aevatar-transport.ts:2318-2339`, and the BE currently
always inserts or generates one at
`backend/src/services/assistant_service.rs:1146-1158`. Preserve it for create.

### N6 -- **REFUTED as written**

Do not add a correction saying `/api/chat/conversations/**` is nonexistent.
Correct the documents by separating typed canonical resources from workflow
Chat History resources. `docs/assistant-network-flows.md:74-101` is broadly
right for typed commands/resources but wrong when row 2 is applied to merged
`chatc-*` rows. `docs/chat-canonical-api-migration.md:58-61` also falsely says
assistant transport sources are wizard bundle sources and mandates a rebuild.
The parity audit itself must be corrected before another document points to it.

### Required guard and test additions

- Add backend upstream-stub tests for every ID family: list pagination,
  transcript, state, DELETE status/body, create recovery, and exact upstream
  path. Unit-testing string builders alone will not catch handler fallback or
  empty-body behavior.
- Add exact create and continuation JSON key-set fixtures derived from console
  `chatApi.test.ts`; assert continuation omits `commandId` and every unknown
  member.
- Add stream-start cases for named reservation 503, other 503, 500, empty body,
  malformed body, refresh 404/5xx, projection-below-fence, and abort.
- Add recovery cases for normal EOF, ambiguous failure, abort/background
  recovery, repeated 404 then success, identity mismatch, and turn-not-yet-in-
  transcript.
- Add a 51+ mixed-family pagination case and cursor-loop rejection.
- Existing Playwright is not an upstream contract gate. Its config explicitly
  says it runs the real UI against `MockAssistantTransport` with no backend at
  `frontend/playwright.config.ts:3-15`; its helper always opens `?mock=1` at
  `frontend/e2e/helpers.ts:27-49`. Keep those UI tests, but add a wire-level
  browser/BE/Aevatar-stub spec or equivalent handler integration suite.

### Wizard bundle conclusion

**REFUTED -- the claim that `frontend/src/lib/assistant/**` is currently in the
wizard module graph.** The authoritative committed graph is
`cli/src/wizard/bundle-meta/index.manifest:1-89`; it contains no
`frontend/src/lib/assistant/*` path. It does contain a few shared assistant
stores and `schemas/assistant-wire-log.ts`, which is a different statement.
The graph is generated from Rollup module IDs at
`frontend/vite-plugins/wizard-manifest.ts:8-42` and the freshness test hashes
exactly that graph plus five extras at
`cli/tests/wizard_bundle_freshness.rs:19-25`, `46-76`.

CI conservatively triggers the wizard freshness job for every `frontend/**`
change at `.github/workflows/ci.yml:120-133`, then runs the source-closure test
at `.github/workflows/ci.yml:331-345`. This review ran
`cargo test -p nyxid-cli --test wizard_bundle_freshness`; it passed. Changes
limited to `aevatar-transport.ts`, its tests, or other absent assistant modules
do not require a wizard rebuild. If implementation instead changes a manifest
member such as `frontend/src/lib/api-client.ts`, follow
`CONTRIBUTING.md:181-192` and run/commit `npm --prefix frontend run
build:wizard` output.

## Recommended changes to the audit document

| Audit location | Verdict | Required correction |
|---|---|---|
| Section 1.5 / D1 | REFUTED | Replace "no branch / phantom family" with proof that the typed canonical family is mounted on pinned dev; describe the actual `chatc-*` family-routing defect |
| Section 1.3 | CONFIRMED, incomplete | State that shared history includes typed rows because typed writers initialize/save there; document full cursor paging and JSON Accept |
| Section 1.4 create recovery | REFUTED | Replace "calls once" with 0/300/900/1800 polling on normal EOF, ambiguous failure, and abort, followed by transcript reconciliation |
| Section 1.4 reservation retry | CONFIRMED | Keep the named 503 behavior; add that zero-message projection is also not ready |
| Section 1.5 fingerprint conclusion | REFUTED in part | Keep create-only/empty-fingerprint facts but remove the implication that same-command continuation replay is safe; cite fresh `TurnId` conflict mechanics |
| Section 2.1 headers | REFUTED in part | Say content negotiation matches; describe NyxID auth as semantically mediated rather than byte-identical Bearer behavior |
| D3 / N3 BE requirement | REFUTED | Record the existing status/body pass-through and move the fix to FE retry decision ordering |
| D4 / N4 | CONFIRMED, incomplete | Add pre-optimistic bounded reconciliation, create-context-zero race, expected-turn observation, and retryable sync state |
| D5 / N5 | REFUTED recommendation | Omit continuation command IDs and remove ambiguous automatic replay; keep create ID plus recovery |
| D6 / N5 | CONFIRMED, incomplete | Trim before both max-length validation and serialization |
| N1 typed delete | REFUTED | Use canonical composite typed DELETE; never manual dual-delete or history-only delete for typed rows |
| N1 workflow delete | CONFIRMED gap | Normalize bare 200 to 204 or teach the FE delete helper to accept an empty successful body |
| N1 list | CONFIRMED gap | Drain every cursor page, reject loops, filter both supported families, dedupe, and test 51+ mixed rows |
| N2 | CONFIRMED direction, incomplete | Add polling/recovery entry points, identity validation, expected-turn ordering, and final reconciliation |
| N6 | REFUTED | Do not point docs to the false phantom-route claim; document typed canonical vs workflow scoped resource ownership |
| Exactness scope | CONFIRMED gap | Resolve NyxID session-ID persistence across reopen and the broader automatic retry policy |
| Verification plan | CONFIRMED gap | Add Aevatar-stub handler/wire tests; current Playwright mock suite cannot verify upstream routes, bodies, headers, or recovery |
| Wizard note | REFUTED | Assistant transport is absent from the committed wizard graph; run freshness, rebuild only if an actual manifest member changes |
| Deployment statement | UNPROVEN | Keep live production contract as an open item until an authenticated capture or exact deployed revision is available |

## Summary

The audit found real workflow-chat parity gaps, but its central explanation is disproved by the pinned upstream: `/api/chat/conversations/**` exists, is mounted, and is the canonical typed NyxIdChat resource family. The repair should be family-aware rather than a wholesale rollback: fully page shared history, use scoped resources for workflow `chatc-*`, retain canonical composite resources for typed `nyxid-chat-*`, add the console's real recovery/reconciliation behavior, preserve and inspect 503 bodies in the FE, remove fabricated watermarks and ambiguous POST replay, and omit continuation `commandId`. The implementation plan must also cover empty workflow DELETE bodies, session reminting, prompt-length trimming, cursor truncation, upstream-stub tests, and conditional rather than mandatory wizard rebuilding; production behavior remains unproven until there is an authenticated live capture or an exact deployed revision.
