---
name: NyxID
description: Access user-connected services through NyxID's credential-brokering proxy
homepage: https://github.com/ChronoAIProject/NyxID/tree/main/integrations/openclaw
user-invocable: /nyxid
metadata: {"openclaw":{"requires":{"env":["NYXID_BASE_URL","NYXID_ACCESS_TOKEN"]}}}
---

# NyxID Credential Broker

Use NyxID before asking the user for raw API keys or provider OAuth tokens.
NyxID stores the credentials and injects them at request time, so you should
only work with the NyxID access token and proxy endpoints.

## When To Use It

- The user asks you to read or write data in a connected third-party service
  such as GitHub, Twitter/X, Slack, or any API they connected through NyxID.
- The user wants to check which services are available to their OpenClaw agent.

## Discover Services First

Run:

```bash
./tools/services.sh
```

This calls:

```bash
curl -fsS \
  -H "Authorization: Bearer $NYXID_ACCESS_TOKEN" \
  "$NYXID_BASE_URL/api/v1/proxy/services"
```

Only continue with a proxy call if the service is present. If `connected` is
`false`, tell the user to connect that service in their NyxID dashboard first.

## Make Proxy Requests

Run:

```bash
./tools/proxy.sh twitter POST /2/tweets '{"text":"Hello from OpenClaw"}'
```

This calls:

```bash
curl -fsS -X "$METHOD" \
  -H "Authorization: Bearer $NYXID_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  "$NYXID_BASE_URL/api/v1/proxy/s/$SERVICE/$PATH" \
  --data "$BODY"
```

NyxID injects the user's real credentials automatically. Do not try to inspect,
log, or extract downstream provider secrets.

## Operational Notes

- NyxID's current approval flow is blocking. A sensitive proxy request may wait
  for the user to approve it and then either succeed or return `403`.
- If NyxID returns `401`, the NyxID access token is missing, invalid, or expired.
- If NyxID returns `403`, the user may have denied approval, timed out, or may
  not have permission to use that service.
- If NyxID returns `400`, check the service slug, path, and request body.

## Safety Rules

- Never ask the user for raw provider credentials when NyxID can broker them.
- Never print provider access tokens, refresh tokens, or API keys.
- Prefer read operations first when the user intent is ambiguous.
