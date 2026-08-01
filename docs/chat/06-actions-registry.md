# Assistant Actions Registry

Last verified against `f608b33c` (2026-08-01).

`GET /api/v1/assistant/actions` publishes the immutable action vocabulary that Aevatar may compose into typed NyxIdChat turns. It is a public, static JSON endpoint with an exact-path exemption from the global rate limiter. It does not depend on a session, database row, user scope, or model state.

The NyxID source is `backend/src/handlers/assistant_actions.rs`. The route is mounted in the public router by `backend/src/routes.rs:build_router`; the exemption is defined by `backend/src/mw/rate_limit.rs:is_rate_limit_exempt` and does not apply to a longer path with the same prefix.

## Response

The response content type is `application/json`. The current body is equivalent to:

```json
{
  "schema_version": 4,
  "revision": "nyxid-assistant-actions.v4",
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
    }
  ]
}
```

The serialized body is created once through `LazyLock<String>` and reused. The handler has no service/model layer because the manifest is compile-time product metadata.

## Top-level contract

| Field | Value | Meaning |
| --- | --- | --- |
| `schema_version` | `4` | action request/report envelope generation |
| `revision` | `nyxid-assistant-actions.v4` | exact composition snapshot expected by the pinned Aevatar client |
| `actions` | descriptor array | actions that are both shipped by NyxID and executable by this Aevatar composition |

Schema version and revision are independent checks. Aevatar rejects a version mismatch and rejects a revision mismatch. The registry is startup-pinned; a running host does not periodically refresh or replace it.

## `service.connect`

The only shipped descriptor is:

| Property | Value |
| --- | --- |
| `action` | `service.connect` |
| `risk` | `grant` |
| `tier` | `v1` |
| `remember_eligible` | `true` |

The description is composition guidance. It states the trust boundary: NyxID owns authentication modality, consent, and credential storage; the model receives only a safe outcome; the assistant must never request a key, token, or password in chat.

`params_schema` permits one of two strict top-level variants:

- `catalogService`, requiring `serviceSlug` and optionally carrying `requestedScopes`, `viaNodeId`, and `targetOrgId`;
- `customService`, requiring `name`, `endpointUrl`, and `authMethod`, and optionally carrying `authKeyName`, `viaNodeId`, and `targetOrgId`.

The manifest schema defines structure. Aevatar and the browser apply additional semantic bounds, URL normalization, control-identity checks, exact-one-variant rules, secret rejection, and supported-auth-method checks before a request can execute. Those rules are specified in [Action cards](04-action-cards.md).

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

The source accepts an absolute HTTP or HTTPS NyxID base URL. It builds the registry URL from the configured origin and base path, performs one GET with response-header streaming, and enforces a 1 MiB limit from both `Content-Length` and bytes read.

Startup then parses the JSON and validates at least:

- valid JSON object shape;
- `schema_version == 4`;
- `revision == "nyxid-assistant-actions.v4"`;
- `actions` is an array;
- action names, tier, risk, remember policy, description, and parameter-schema shape;
- no duplicate action descriptor;
- supported schema constructs; and
- presence of every action this Aevatar version marks executable, currently `service.connect`.

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
