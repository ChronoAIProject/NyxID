# Assistant Actions Registry

Last verified against the `service.reauthorize` implementation (2026-08-17).

`GET /api/v1/assistant/actions` publishes the immutable action vocabulary that Aevatar may compose into typed NyxIdChat turns. It is a public, static JSON endpoint with an exact-path exemption from the global rate limiter. It does not depend on a session, database row, user scope, or model state.

The NyxID source is `backend/src/handlers/assistant_actions.rs`. The route is mounted in the public router by `backend/src/routes.rs:build_router`; the exemption is defined by `backend/src/mw/rate_limit.rs:is_rate_limit_exempt` and does not apply to a longer path with the same prefix.

## Response

The response content type is `application/json`. The current body is equivalent to:

```json
{
  "schema_version": 4,
  "revision": "nyxid-assistant-actions.v8",
  "actions": [
    {
      "action": "service.connect",
      "description": "Ask the user's browser to connect a service through NyxID. Use when a task needs a catalog service (by slug) or a custom HTTPS endpoint that the user has not connected yet. NyxID owns the entire journey - auth modality, consent copy, and credential storage - and reports back only completion or decline with a safe resource reference. Never ask the user for keys, tokens, or passwords in chat.",
      "params_schema": {
        "oneOf": [
          {
            "type": "object",
            "additionalProperties": false,
            "required": ["catalogService"],
            "properties": {
              "catalogService": {
                "type": "object",
                "additionalProperties": false,
                "required": ["serviceSlug"],
                "properties": {
                  "serviceSlug": { "type": "string" },
                  "requestedScopes": {
                    "type": "array",
                    "items": { "type": "string" }
                  },
                  "viaNodeId": { "type": "string" },
                  "targetOrgId": { "type": "string" }
                }
              }
            }
          },
          {
            "type": "object",
            "additionalProperties": false,
            "required": ["customService"],
            "properties": {
              "customService": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "endpointUrl", "authMethod"],
                "properties": {
                  "name": { "type": "string" },
                  "endpointUrl": { "type": "string" },
                  "authMethod": { "type": "string" },
                  "authKeyName": { "type": "string" },
                  "viaNodeId": { "type": "string" },
                  "targetOrgId": { "type": "string" }
                }
              }
            }
          }
        ]
      },
      "risk": "grant",
      "tier": "v1",
      "remember_eligible": true
    },
    {
      "action": "service.reauthorize",
      "description": "Ask the user's browser to re-authorize an existing connected service and review its requested scopes. Use when a task needs permissions that the referenced user service does not currently grant. NyxID owns the authorization journey and credential storage, and reports only a safe user-service reference. Never ask the user for keys, tokens, passwords, or authorization codes in chat.",
      "params_schema": {
        "type": "object",
        "additionalProperties": false,
        "required": ["userServiceId", "requestedScopes"],
        "properties": {
          "userServiceId": { "type": "string" },
          "requestedScopes": {
            "type": "array",
            "items": { "type": "string" }
          }
        }
      },
      "risk": "grant",
      "tier": "v1",
      "remember_eligible": false
    },
    {
      "action": "key.create",
      "description": "Ask the user's browser to create a scoped NyxID API key for the named platform and allowed services. Use when the user wants a new agent identity bounded to specific user-service IDs. NyxID owns key creation and one-time key display, and reports only a safe key reference. Never request, expose, or repeat key material in chat.",
      "params_schema": {
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "platform", "allowedServiceIds"],
        "properties": {
          "name": { "type": "string" },
          "platform": { "type": "string" },
          "allowedServiceIds": {
            "type": "array",
            "minItems": 1,
            "maxItems": 64,
            "uniqueItems": true,
            "items": { "type": "string" }
          }
        }
      },
      "risk": "grant",
      "tier": "v1",
      "remember_eligible": false
    },
    {
      "action": "key.rotate",
      "description": "Ask the user's browser to rotate one exact NyxID API key. Use when the user needs a replacement credential for the identified key. NyxID commits an authoritative predecessor-successor relation, displays replacement key material once in the browser, and reports only the replacement key reference. Never request, expose, or repeat key material in chat.",
      "params_schema": {
        "type": "object",
        "additionalProperties": false,
        "required": ["keyId"],
        "properties": {
          "keyId": { "type": "string" }
        }
      },
      "risk": "grant",
      "tier": "v1",
      "remember_eligible": false
    }
  ]
}
```

The default (no-query) serialized body is created once through `LazyLock<String>` and reused. Per-revision compositions are pre-serialized the same way. The handler has no service/model layer because the manifest is compile-time product metadata.

### Revision negotiation

`GET /api/v1/assistant/actions?revision=<r>`:

| Request | Response |
| --- | --- |
| No `revision` query param | Latest body, byte-identical to the default composition (`schema_version` 4, `revision` `nyxid-assistant-actions.v8`, including dormant Wave-2 descriptors). Current consumers are unchanged. |
| Known `<r>` | A composition whose `revision` field is exactly `<r>` and whose `actions` are **that revision's action-name set**, serialized with **the `params_schema` Aevatar's `ValidatePinnedContract` selects for that revision**. Aevatar does per-revision schema selection, then `JsonNode.DeepEquals` against the chosen compiled constant — a local "current bytes only" rule cannot override the deployed validator. Descriptions use the current live text (Aevatar does not pin descriptions). Schema overrides live in the revision map (`PARAMS_SCHEMA_OVERRIDES_BY_REVISION`); a future split is a data row, not a compose-path branch. For `key.create` the split is identical on `origin/dev` and `origin/feature/integrate`: **v4, v5** → `KeyCreateParamsSchema` (no `minItems` / `maxItems` / `uniqueItems` on `allowedServiceIds`); **v6, v7, v8** → `LeastScopeKeyCreateParamsSchema` (the current live descriptor). v4 pins only `service.connect`, so `key.create` is not served there at all. The live consumer ask is `?revision={SupportedRegistryRevision}` (v7 on `origin/dev`, v8 on `origin/feature/integrate`); both of those select the least-scope schema. |
| Unknown `<r>` | `404` with the stable `not_found` / `1003` error body. No manifest fields. |
| Malformed `<r>` (more than 128 characters, or any control character) | `400` with the stable `validation_error` / `1008` error body. |

Known revisions are the static `revision → action-name set` map mirroring Aevatar `PinnedActionsByRevision` (`origin/dev` accepts `{v4, v5, v6, v7}`; `origin/feature/integrate` accepts those plus `v8`). v7's set is `{service.connect, key.create, key.rotate}` — not a prefix of the current default array (`service.reauthorize` sits at index 1). `aevatar-nyxid-actions.v1` is Aevatar's own composition namespace, not a NyxID-published revision; NyxID returns 404 for it. The checked-in file `tests/fixtures/assistant/aevatar-pinned-actions-by-revision.json` is a **manually synced mirror** of both `PinnedActionsByRevision` and `ExecutableActionsByRevision` per lineage; CI does not read the Aevatar clone, so a green test is not evidence that upstream has not moved.

The rate-limit exemption is exact-path (`/api/v1/assistant/actions`). Query strings do not change the path.

#### Published-revision contract

- **Immutable.** Once a revision is published, its action-name set and the `params_schema` served for each pinned action are never edited. A live-descriptor schema change is a new revision plus a historical override row, not a silent rewrite of an old composition.
- **Monotone (each ⊆ the next), waves only append.** A new wave adds names under the current string as dormant descriptors, then a later revision-bump PR adds a new map entry whose set is the previous pin plus those names. Historical exception, frozen in Aevatar's pin map and not to be repeated: v5 (`WaveOneDraft`) listed `service.reauthorize` and `key.rotate`, then v6 (`LeastScope`) dropped them. From v6 onward the served sets are append-only.
- **Dormant descriptors** (today: the 12 Wave-2 verbs) appear only in the default latest body. They are absent from every pinned revision set until a future revision pins them. No wave PR bumps `ASSISTANT_ACTIONS_REVISION`.

### Upstream consumer ask (Aevatar)

Startup fetch should request:

```text
{Aevatar:NyxId:ApiBaseUrl}/api/v1/assistant/actions?revision={SupportedRegistryRevision}
```

On `404` (older NyxID without negotiation), fall back to the bare path and existing set-membership validation. One URL-builder change plus fallback; NyxID and Aevatar can then deploy independently.

## Top-level contract

| Field | Value | Meaning |
| --- | --- | --- |
| `schema_version` | `4` | action request/report envelope generation |
| `revision` | `nyxid-assistant-actions.v8` | exact composition snapshot expected by the pinned Aevatar client |
| `actions` | descriptor array | actions that are both shipped by NyxID and executable by this Aevatar composition |

Schema version and revision are independent checks. Aevatar rejects a version mismatch and rejects a revision mismatch. The registry is startup-pinned; a running host does not periodically refresh or replace it.

## Shipped actions

The registry ships these descriptors:

| Action | Parameters | Risk | Remember eligible |
| --- | --- | --- | --- |
| `service.connect` | strict catalog-service or custom-service variant | `grant` | `true` |
| `service.reauthorize` | exact `userServiceId` plus nonempty normalized unique `requestedScopes` enforced by the consumer and browser | `grant` | `false` |
| `key.create` | `name`, `platform`, exact nonempty unique `allowedServiceIds` | `grant` | `false` |
| `key.rotate` | exact predecessor `keyId` | `grant` | `false` |

The description is composition guidance. It states the trust boundary: NyxID owns authentication modality, consent, and credential storage; the model receives only a safe outcome; the assistant must never request a key, token, or password in chat.

`params_schema` permits one of two strict top-level variants:

- `catalogService`, requiring `serviceSlug` and optionally carrying `requestedScopes`, `viaNodeId`, and `targetOrgId`;
- `customService`, requiring `name`, `endpointUrl`, and `authMethod`, and optionally carrying `authKeyName`, `viaNodeId`, and `targetOrgId`.

The manifest schema defines structure. Aevatar and the browser apply additional semantic bounds, URL normalization, control-identity checks, exact-one-variant rules, secret rejection, and supported-auth-method checks before a request can execute. Those rules are specified in [Action cards](04-action-cards.md).

### Where the browser is stricter than the published schema

The manifest is the contract, and it is pinned byte-for-byte: Aevatar compares
each published `params_schema` with `JsonNode.DeepEquals`, so a refinement
added to the manifest to describe a browser-side rule would fail the pin and
disable the whole registry. The rules below therefore live only in the browser
and are invisible from the manifest. They are recorded here because a request
that satisfies the published schema can still be refused.

| Parameter | Published schema | Browser rule (`frontend/src/schemas/assistant-actions.ts`) |
| --- | --- | --- |
| `requestedScopes[]` | `{"type": "string"}`, and the array may be empty | non-empty, `<= 256` chars, already trimmed, and matching `/^[A-Za-z0-9._:\/~+*=-]+$/` |
| `requestedScopes` | array of strings | 1-64 entries, no duplicates |

RFC 6749 §3.3 permits every printable ASCII character except space, `"` and
`\` in a scope token, so the character class is narrower than the standard
allows. No provider in the current catalog trips it -- Google, GitHub, Slack,
Lark, Microsoft, Zoom, HubSpot and Atlassian scopes all pass -- but a
conforming scope outside it parses at Aevatar and then degrades to
`{variant: "unknown"}` in the browser, which renders "Unsupported action
request" without saying which scope was rejected.

If a provider ever needs a character outside that class, widen the browser
regex; do not widen the manifest.

### Authorization evidence reads

Postcondition verification reads evidence from dedicated projections rather
than from the full detail responses:

| Verb | Evidence read |
| --- | --- |
| `service.reauthorize` | `GET /api/v1/keys/{id}/authorization` |
| `key.create`, `key.rotate` | `GET /api/v1/api-keys/{id}/authorization` |

Both return exactly the properties the reader consumes and nothing else. The
full `/keys/{id}` and `/api-keys/{id}` responses remain unchanged and are still
the documented detail contract for the dashboard, the CLI and external
consumers; they are simply not safe to use as evidence, because they carry
user-controlled free text (a service label, a custom header value, the
supported `Bearer ${credential}` WS auth template, a key description) that the
reader's secret-shape scan cannot distinguish from a real credential. A
consumer still reading evidence from the detail routes will reject any service
or key configured that way, permanently and silently.

`key.create` executes through `POST /api/v1/assistant/actions/key-create`.
The effect reserves one key UUID in a durable, secret-free action receipt,
validates the exact personal service set, and makes exact retries return only
the committed key ID. `key.rotate` uses the matching `key-rotate` route and
reserves one successor UUID before entering the transactional rotation path.
The authoritative successor read exposes `created_at`, exact
`rotation_predecessor_id`, positive `state_version`, and `updated_at` without
key material. Both browser journeys display a newly committed secret once and
submit only `{ "key": { "keyId": "..." } }` to Aevatar.

## ADR: Wave-0 G1 resolved as fork (b)

G1 of the assistant support contract is resolved as **fork (b): extended REST**. Wave-1 postconditions shipped as hardened REST evidence projections (`GET /keys/{id}/authorization`, `GET /api-keys/{id}/authorization`) plus delegated `account:read` GET admission, consumed by Aevatar's REST reader (`NyxIdAssistantToolSource` / `NyxIdApiClient`). The MCP `nyx__*` G2 read built-ins (`nyx__list_api_keys`, `nyx__readiness`, and similar) will not be built. The contract §6 matrix "mechanism" column re-targets to the registered REST reads and projections documented above. Aevatar wave issues must not target MCP-client read tools that will never exist. Class-P (proxy execution in chat) and uncovered Class-R parity reads remain gapped; that is Wave-0 product debt owned by Aevatar's mechanism decision and does not gate Wave 2/3/4 postconditions.

## `params_schema` loader grammar

Aevatar accepts this restricted recursive schema grammar:

- a `oneOf` node with a nonempty array whose branches are recursively valid;
- an `object` node with literal `"additionalProperties": false` and a `properties` object whose values are recursively valid;
- an `array` node with a recursively valid `items` schema; or
- a `string` node.

Every node without `oneOf` requires a string `type` of at most 32 characters. Only `object`, `array`, and `string` are supported. `required` arrays are used when validating request parameters. Other keywords such as `format`, `enum`, and `minLength` are ignored rather than rejected, so the manifest must not imply that Aevatar enforces them.

Every property name at every depth is normalized by retaining only ASCII alphanumeric characters and lowercasing. A normalized name is forbidden when it equals one of:

```text
token tokens accesstoken refreshtoken authorization cookie cookies secret secrets
clientsecret password passphrase usercode devicecode rawbody rawupstreambody
credential credentials
```

Manifest root and descriptor fields use snake_case, including `schema_version`, `params_schema`, and `remember_eligible`. Parameter property names use the typed wire contract's camelCase, including `catalogService` and `serviceSlug`. This asymmetry is deliberate.

Implementation: upstream `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs:ValidateSchemaNode` and `NyxIdActionSecretPolicy.cs:ValidateFieldName`; NyxID mirrors the grammar and normalized forbidden set in `backend/src/handlers/assistant_actions.rs` tests.

## Aevatar startup consumption

When `Aevatar:NyxId:AssistantActions:Enabled` is `true`, Aevatar registers a startup service that fetches:

```text
{Aevatar:NyxId:ApiBaseUrl}/api/v1/assistant/actions
```

Once the consumer-ask above lands, that fetch appends `?revision={SupportedRegistryRevision}` and falls back to this bare path on 404.

The source accepts an absolute HTTP or HTTPS NyxID base URL. It builds the registry URL from the configured origin and base path, performs one GET with response-header streaming, and enforces a 1 MiB limit from both `Content-Length` and bytes read.

Startup then parses the JSON and validates at least:

- valid JSON object shape;
- `schema_version == 4`;
- `revision == "nyxid-assistant-actions.v8"`;
- `actions` is an array;
- action names, tier, risk, remember policy, description, and parameter-schema shape;
- no duplicate action descriptor;
- supported schema constructs; and
- presence of every action this Aevatar version marks executable: `service.connect`, `service.reauthorize`, `key.create`, and `key.rotate` for the v8 composition.

Enabled startup fails if fetch, size, JSON, schema, revision, or required-action validation fails. The host therefore does not accept typed chat work with a partially loaded or ambiguous registry.

When the feature is disabled, Aevatar injects an immutable empty registry instead of running the fetch service. No action is executable, and attempts to resolve action requests fail closed as unsupported.

The upstream implementation anchors are:

- `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistryStartup.cs`;
- `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs`;
- `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionsOptions.cs`; and
- `agents/Aevatar.GAgents.NyxidChat/ServiceCollectionExtensions.cs`.

## Policy ownership

Risk and remember eligibility come from the pinned registry definition. An action request cannot override them. Aevatar rejects caller-supplied risk or remember-policy values during request validation.

Unknown manifest actions that are not compiled into Aevatar do not become executable merely because NyxID publishes them. Conversely, Aevatar refuses startup if an action it requires for the current executable set is missing. Shipping a new action therefore requires coordinated support in:

- NyxID's static manifest;
- Aevatar's compiled action contract and typed producer;
- the browser envelope schema and journey registry;
- the backend report allowlist and postcondition resource; and
- action-card tests across registry, frame, UI, and continuation delivery.

Changing only prose or only the manifest is insufficient.

## Security properties

The registry contains descriptions and JSON Schema only. It contains no user data, credentials, service tokens, delegated capabilities, internal connector results, or mutable policy.

The executable path preserves these boundaries:

- Registry composition tells the model which safe action it may request.
- The custom frame carries typed parameters but no credential value.
- The browser owns consent and credential entry.
- The browser reports an allowlisted disposition and safe resource ID.
- Aevatar validates the report against the typed actor action state.

The public endpoint is therefore safe to fetch during Aevatar startup without user authentication. Its publication does not authorize an action or grant access to any user resource.
