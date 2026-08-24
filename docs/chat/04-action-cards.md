# Assistant Action Cards

Last verified against `fcb79b18` (2026-08-01).

Assistant action cards are a typed NyxIdChat protocol for asking the browser to perform a sensitive NyxID-owned journey and report only a safe outcome. The model does not receive credentials, raw connector responses, or browser form data.

The current shipped action is `service.connect`. The card can open NyxID's catalog-service or custom-service connection journey, then report a disposition and a safe `UserService` identity to the typed actor.

The rendering reference remains [assistant-action-cards-showcase.html](../assistant-action-cards-showcase.html). The executable contract is implemented by `frontend/src/schemas/assistant-actions.ts`, `frontend/src/lib/assistant/action-registry.ts:resolveAssistantAction`, `frontend/src/lib/assistant/chat-action-validation.ts:validateActionRequest`, `frontend/src/lib/assistant/chat-actor-state.ts:reduceActorFrame`, `frontend/src/components/assistant/blocks/action-card.tsx:ActionCard`, and `backend/src/services/assistant_service.rs`.

## Upstream producer constraint

Action requests arrive only as this AG-UI frame:

```text
CUSTOM name=nyxid.action.request
```

The pinned Aevatar source emits that frame from the typed NyxIdChat conversation
producer. Aevatar's `/api/chat` dispatcher selects that producer only when the
request has a top-level `type`. NyxID sends every browser turn through that typed
producer; a legacy `chatc-*` history row is never an action origin or target.

The upstream anchors are `MainnetChatEndpoints.cs:ClassifyRequestAsync`, `NyxIdChatPublicEndpoints.cs`, and `NyxIdChatConversationAguiFrameBuilder.cs`. Action-result continuation likewise uses the typed `action.continue` command and a `nyxid-chat-...` actor identity. A visible Studio `chatc-...` identity is never substituted as the typed actor ID.

Action cards may be present in typed histories and remain actor-projected while
text history materializes. A transcript reload must not replace the pending
action state with a text-only history response.

## Envelope v4

The custom payload is a strict object:

```json
{
  "schemaVersion": 4,
  "actorId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "originTurnId": "turn-identity",
  "taskId": "task-identity",
  "stepId": "step-identity",
  "actionRequestId": "action-identity",
  "action": "service.connect",
  "params": {
    "catalogService": {
      "serviceSlug": "github",
      "requestedScopes": ["repo"],
      "viaNodeId": "",
      "targetOrgId": ""
    }
  }
}
```

Fields have these meanings:

| Field | Contract |
| --- | --- |
| `schemaVersion` | Integer envelope version. Version 4 is executable; another structurally valid version renders unsupported. Missing defaults to 0. |
| `actorId` | Typed conversation actor that receives `action.continue`. It must be a control identity when nonempty. |
| `originTurnId` | Required turn that issued the request and ordering group for reports. |
| `taskId` | Optional bounded task correlation identity. |
| `stepId` | Optional bounded step correlation identity. |
| `actionRequestId` | Required idempotency and card identity. |
| `action` | Requested verb. `service.connect` is the only executable verb. |
| `params` | One strict resource variant. |

`originTurnId` and `actionRequestId` are 1 through 256 characters and reject whitespace, control characters, `/`, `\`, `?`, and `#`. A nonempty `actorId` uses the same grammar. Optional correlation IDs are bounded to 256 characters. General wire strings are bounded to 4,096 characters.

Unknown object fields are rejected at every strict schema layer. Secret-shaped property names are rejected defensively, including authorization, API-key, token, secret, password, credential, cookie, user-code, and device-code forms. String values resembling `Bearer ...` or a `nyx_...`/`nyxid_...` secret are rejected.

The envelope is an instruction to show a NyxID-controlled consent journey. It is not authority to execute a backend mutation without user interaction.

## `service.connect` parameters

Exactly one of `catalogService` or `customService` must normalize to a supported journey. A payload with neither, both, or invalid normalized values renders as unsupported.

### Catalog service

```json
{
  "catalogService": {
    "serviceSlug": "github",
    "requestedScopes": ["repo", "read:user"],
    "viaNodeId": "node-identity",
    "targetOrgId": "organization-identity"
  }
}
```

`serviceSlug` is required after trimming and must match `[A-Za-z0-9._-]{1,128}` for execution. `requestedScopes` defaults to an empty array, contains at most 64 strings, and limits each entry to 256 characters. `viaNodeId` and `targetOrgId` default to empty and normalize to nullable IDs.

The card shows a short, normalized service label rather than raw model prose. Known slugs receive curated labels; other slugs are humanized and clamped to 32 characters on one line. This prevents a long or sentence-shaped service name from masquerading as NyxID-authored consent copy.

### Custom service

```json
{
  "customService": {
    "name": "Internal search",
    "endpointUrl": "https://search.example.test/api/",
    "authMethod": "header",
    "authKeyName": "X-API-Key",
    "viaNodeId": "node-identity",
    "targetOrgId": "organization-identity"
  }
}
```

`name` and `endpointUrl` are required after trimming. The normalized endpoint must:

- parse as an absolute URL;
- use HTTPS;
- have a hostname;
- contain no username or password;
- contain no query; and
- contain no fragment.

`authMethod` defaults to `none` and is one of `bearer`, `header`, `query`, `path`, `basic`, `body`, or `none`. `authKeyName`, when nonblank, must be 1 through 256 HTTP token characters. `viaNodeId` and `targetOrgId` normalize like the catalog variant.

The card does not accept a key value, token, password, Authorization header, or raw credential. Credential entry and storage occur inside the NyxID-owned connection UI.

## Validation outcomes

The browser uses two validation levels so a malformed request cannot execute but a user can still release a typed actor waiting on a recognizable request.

### Executable request

A request is executable only when:

- the full strict envelope parses;
- no secret-shaped key or value is present;
- `schemaVersion` is 4;
- `action` is `service.connect`; and
- exactly one parameter variant normalizes to a supported connection journey.

It creates a card in `pending` state with the resolved catalog or custom journey.

### Recoverable unsupported request

If full parsing fails but the object has a valid `actionRequestId` and `originTurnId`, `recoverUnsupportedAssistantActionRequest` discards every untrusted or failed field and creates a non-executable fallback card. The fallback keeps only the two identities required to send a decline, a bounded action label when safe, an integer schema version when present, and a valid optional actor ID. Parameters become empty.

This card has status `unsupported`. Its only useful disposition is decline/failure; it cannot open a connection journey.

### Ignored request

If either required control identity cannot be recovered, no card is created. The browser cannot safely identify or report the request, so inventing an ID would violate idempotency and actor ordering.

These rules fail closed without trapping the user behind every syntactically recognizable future-version request.

## Card state machine

`ActionCardStatus` has these values:

| Status | Meaning | User action |
| --- | --- | --- |
| `pending` | valid request awaiting a decision | connect or decline |
| `in_progress` | NyxID connection journey is open or running | journey controls |
| `blocked` | local journey/reporting could not continue safely | decline/fail, or wait for exact reissue |
| `completed` | local action completed and completion is queued or delivered | none |
| `conflicted` | the same request ID arrived with changed committed details | none; first request remains authoritative |
| `declined` | user declined and report is queued or delivered | none |
| `failed` | failed/cancelled/expired report is queued or delivered | none |
| `unsupported` | request is not executable by this client | decline |

`setActionCardInProgress` moves `pending` to `in_progress` and can return it to `pending`. It does nothing for blocked or terminal states. `blockActionCard` can block pending, in-progress, unsupported, or otherwise nonterminal cards, but cannot overwrite completed, conflicted, declined, or failed state.

The card remains interactive after the origin run reaches its normal terminal. It is not treated as an open model approval gate; the browser action has its own lifecycle.

## Idempotency and conflict rules

`actionRequestId` is the stable identity. The actor projection retains both the committed card fields and the validated original request parameters.

### Exact duplicate

The same ID with the same committed fields and fingerprint is idempotent. It reuses the existing block rather than appending another card. Pending, in-progress, and terminal cards retain their state.

### Changed duplicate

The same ID with a changed action, origin, actor, task, step, normalized parameters, or request fingerprint conflicts with the committed request. If the card is not terminal, the browser changes it to `conflicted`, disables execution, and displays:

```text
This action request was reissued with conflicting details. NyxID kept the first request and disabled this card.
```

The first committed request remains authoritative. The changed payload is never used to open a journey or construct a report. If the existing card is already completed, conflicted, declined, or failed, a later changed duplicate cannot rewrite that terminal outcome.

### Blocked reissue

An exact reissue of a `blocked` card re-arms it. If the request still resolves to a shipped journey, it returns to `pending` and clears the blocked note. If the client no longer supports it, it becomes `unsupported` and preserves a composed explanation.

A changed blocked reissue conflicts; it does not re-arm. This preserves first-commit semantics while allowing the typed actor to repeat the identical request after the environment changes.

Implementation: `frontend/src/lib/assistant/chat-actor-state.ts:decodeActorFrame`,
`applyActionRequest`, and `actionIdentityMatches`.

## Connection journey

The action descriptor produces NyxID-authored title, body, and call-to-action copy. For `service.connect` the body states that NyxID brokers access and keeps the credential out of the model.

The catalog journey selects an existing catalog service by normalized slug and may carry requested scopes, node preference, and target organization. The custom journey opens a controlled service-definition path with a validated HTTPS endpoint and auth method. The visible card does not submit its untrusted model payload directly to a generic API; the UI maps the resolved variant to explicit NyxID form inputs.

On successful connection, the journey returns the created or selected `UserService` ID. That safe identity is the only completion resource reported for the shipped action. If the user cancels or declines, no resource is needed.

## Report envelope

The browser reports one or more decisions through typed `action.continue`:

```json
{
  "type": "action.continue",
  "conversationId": "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
  "clientRequestId": "delivery-identity",
  "originTurnId": "turn-identity",
  "actions": [
    {
      "actionRequestId": "action-identity",
      "originTurnId": "turn-identity",
      "disposition": "completed",
      "resource": {
        "userService": {
          "userServiceId": "service-identity"
        }
      }
    }
  ]
}
```

Allowed dispositions are:

- `completed`;
- `declined`;
- `failed`;
- `cancelled`; and
- `expired`.

The browser card maps `completed` to completed, `declined` to declined, and the remaining failure-like dispositions to failed.

All report objects are strict. The outer and inner `originTurnId` values must match. An action request ID can appear only once in a batch. The browser validates a nonempty batch before changing card state. The backend additionally caps a batch at 64 reports.

Reports in one batch must belong to the same typed conversation actor. The transport uses each card's `actorId`; if that is absent, it may use the stored durable ID only when it is a typed `nyxid-chat-...` conversation. It never sends a workflow `chatc-...` ID as `conversationId` for `action.continue`. If no typed actor is known, the batch is blocked and no POST occurs.

An empty `actions` array is a distinct wake command and is allowed only for a typed conversation.

## Safe resources

The schema recognizes these strict one-variant resource references:

```json
{ "userService": { "userServiceId": "..." } }
{ "key": { "keyId": "..." } }
{ "node": { "nodeId": "..." } }
{ "serviceAccount": { "serviceAccountId": "..." } }
{ "developerApp": { "clientId": "..." } }
{ "device": { "deviceId": "..." } }
```

Each object has exactly one variant and each payload has exactly one valid control identity. The builder copies only the allowlisted variant and ID into the wire body.

Every `completed` report must carry exactly one of these allowlisted resource variants. A completed report with no resource, multiple variants, an unknown variant, an extra payload member, an invalid control identity, or secret-shaped material is rejected by `backend/src/services/assistant_service.rs:parse_assistant_chat_command`.

Declined, failed, cancelled, and expired reports may omit the resource.

## Blocked and conflicted reporting

A `blocked` card cannot report `completed`. A `conflicted` card cannot be continued at all. These restrictions prevent a completed local side effect from being attributed to a request whose active wire identity or parameters are no longer trustworthy.

If a local service was already connected before the card became blocked or conflicted, the UI records:

```text
A service was connected in NyxID, but this action request could not notify the assistant. Review it in AI Services.
```

For a conflict, this is composed with the first-request conflict note. The service remains visible and manageable in NyxID, but the client does not forge a successful report.

Decline or failure remains the escape path for blocked and unsupported cards when their actor identity is valid.

## Delivery and settlement

The browser sends one selected outcome through a typed `action.continue`
command containing one report. Dispatch requires the current typed actor
identity and a positive state version; no report is redirected to another
engine. The report receives a fresh `clientRequestId`, which is also the
NyxID/Aevatar idempotency key.

The browser does not queue or automatically replay a failed report. A
pre-stream HTTP or network failure rejects the operation, restores the card to
an actionable state, and lets the user retry. Once the continuation has adopted
its authoritative actor and turn identity, a later stream failure settles as a
visible turn error without resubmitting the report.

The actor's projected action summary is authoritative for terminal card state.
Its latest committed report maps `completed`, `declined`, and failure-like
dispositions to the corresponding settled card. Until that fact arrives, the
request remains pending; a locally blocked journey is held only as a
conversation-local presentation override.

Implementation: `frontend/src/hooks/use-assistant-chat-controls.ts:reportAction`,
`frontend/src/lib/assistant/chat-stream-orchestrator.ts:runChatStream`, and
`frontend/src/lib/assistant/chat-action-presentation.ts:actionSummaryBlock`.

## Security and fail-closed rules

The complete action path maintains these invariants:

- The frame carries no credential value.
- Strict parsing and secret-pattern checks run before action resolution.
- Model-supplied strings do not become NyxID consent prose without normalization and bounds.
- Only a supported v4 action opens a journey.
- A future or malformed recognizable action can be declined but not executed.
- An unidentifiable malformed action is ignored rather than assigned a local wire identity.
- The first committed request wins for each `actionRequestId`.
- Changed duplicates cannot mutate a pending or terminal card into a different action.
- Completed `service.connect` reports contain only the safe `UserService` ID.
- Report DTOs are rebuilt from allowlisted fields in both frontend and backend.
- Reports return to the typed actor that issued the request, never to a workflow conversation guessed from UI context.
- A blocked/conflicted completed side effect is surfaced for manual management but not falsely reported.
- Raw form values, upstream connector bodies, Authorization headers, and secrets are never projected into the transcript or report.
