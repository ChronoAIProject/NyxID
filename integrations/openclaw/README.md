# OpenClaw NyxID Integration

`openclaw-plugin-nyxid` packages two OpenClaw extensions together:

- A ClawHub skill at [`skills/nyxid`](./skills/nyxid) for declarative agent usage
- An auth/runtime plugin that logs into NyxID and exchanges delegated proxy tokens

## What It Supports

- OAuth 2.0 Authorization Code + PKCE against NyxID
- Access token refresh using NyxID's `refresh_token` grant
- RFC 8693 token exchange for short-lived delegated proxy access
- `nyxid_list_services` tool for service discovery
- `nyxid_proxy` tool for slug-based proxy requests through NyxID

## Important NyxID Runtime Behavior

- Service discovery uses the user's direct NyxID access token because delegated
  tokens are restricted to proxy, LLM gateway, and delegation refresh endpoints.
- Proxy calls use a delegated token obtained from `POST /oauth/token` with the
  token-exchange grant.
- NyxID's current approval flow is blocking. Proxy requests wait for approval
  and either complete or return `403`; they do not return `202 Accepted`.

## Local Development

```bash
cd integrations/openclaw
npm install
npm test
```

## Publish

```bash
cd integrations/openclaw
npm publish --access public
```

After the npm package is published, publish the skill bundle to ClawHub using
the same `nyxid` slug and point the listing to this directory.
