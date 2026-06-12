---
title: Connect aevatar
description: Add a self-hosted aevatar runtime to NyxID and route Codex, Cursor, and channel traffic through the existing slug proxy.
---

aevatar is a self-hosted downstream runtime in the NyxID catalog. It uses the same service model as other AI services: a user supplies the runtime URL and bearer token, NyxID stores the credential, and clients call the runtime through `/api/v1/proxy/s/aevatar/*`.

NyxID does not add a dedicated `/api/v1/llm/aevatar/*` namespace. The catalog entry exists so clients can discover one documented contract instead of hard-coding ad hoc routing.

## Add the service

Store the aevatar bearer token in an environment variable, then add the catalog service with your runtime URL:

```bash
export AEVATAR_TOKEN="aevatar-runtime-token"

nyxid service add aevatar \
  --endpoint-url "https://aevatar.example.com" \
  --credential-env AEVATAR_TOKEN \
  --label "Aevatar"
```

The command creates a UserEndpoint, UserApiKey, and UserService. The service slug is `aevatar` unless another active service already uses that slug, in which case the CLI reports the generated slug such as `aevatar-2`.

## Call the runtime

Use the normal slug proxy route:

```bash
curl -X POST "https://nyxid.example.com/api/v1/proxy/s/aevatar/v1/responses" \
  -H "X-API-Key: $NYXID_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"aevatar-default","input":"Handle this event","stream":true}'
```

The outbound request to aevatar receives:

| Header | Value |
|---|---|
| `Authorization` | `Bearer <stored-aevatar-token>` |
| `X-NyxID-Identity-Token` | Short-lived RS256 JWT with the NyxID user ID and email |

The raw NyxID caller access token is not forwarded.

## Identity JWT

The catalog entry enables `identity_propagation_mode="jwt"` and includes the user ID and email. aevatar should verify `X-NyxID-Identity-Token` against NyxID's JWKS and treat it as a short-lived assertion for the proxied request.

By default the JWT audience follows the connected endpoint URL. If your aevatar deployment expects a different audience, update the service's identity settings after creation.

## Streaming

The catalog advertises streaming support. If the aevatar runtime returns Server-Sent Events, NyxID streams the response through the proxy without buffering.

Use `stream: true` or the equivalent aevatar request option when the runtime supports SSE:

```bash
nyxid proxy request aevatar v1/responses \
  -m POST --stream \
  -d '{"model":"aevatar-default","input":"Stream this","stream":true}'
```

## Delegation

`X-NyxID-Delegation-Token` is disabled by default for aevatar. The catalog reserves `delegation_token_scope="llm:proxy"` so an operator can explicitly enable delegation later after the aevatar runtime verifies that it can consume and refresh NyxID delegation tokens.

When delegation is disabled, aevatar should use its normal runtime credentials and the identity JWT above. It should not expect callback authority to NyxID APIs from the default catalog contract.

## Scope Rules

Clients calling `/api/v1/proxy/s/aevatar/*` need standard REST proxy access: `proxy` or `proxy:*`. The LLM-only `llm:proxy` scope does not grant REST proxy access.

For scoped agent keys, allow the concrete `UserService.id` created by `nyxid service add aevatar`. NyxID enforces service allowlists after resolving the slug, so the catalog slug itself is not an access-control entry.
