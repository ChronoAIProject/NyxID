# NyxID OpenClaw Integration

`openclaw-plugin-nyxid` lets OpenClaw agents discover and call user-connected services through NyxID's credential brokering proxy.

## What it supports

- OAuth 2.0 + PKCE login against NyxID
- Refresh token handling
- RFC 8693 token exchange when a confidential client is configured
- Direct proxy access with an existing NyxID access token
- Direct proxy access with a NyxID API key for self-hosted installs
- A ClawHub-ready `nyxid` skill with helper scripts

## Default base URL

The default hosted NyxID base URL for this integration is:

`https://nyx-api.chrono-ai.fun`

## Configuration

```json
{
  "plugins": {
    "nyxid": {
      "enabled": true,
      "baseUrl": "https://nyx-api.chrono-ai.fun",
      "clientId": "your-nyxid-client-id",
      "clientSecret": "your-nyxid-client-secret",
      "delegationScopes": "proxy:*"
    }
  }
}
```

## Auth modes

### OAuth mode

Provide `baseUrl` and `clientId`. Add `clientSecret` if you want the plugin to perform RFC 8693 token exchange for delegated proxy calls.

### API key mode

Set either:

- `NYXID_API_KEY`
- `plugins.nyxid.apiKey`

You can also provide a NyxID bearer token through `NYXID_ACCESS_TOKEN`.

When the plugin has an API key but not an OAuth client, it calls the proxy directly with `X-API-Key`.

## Skill helpers

- `skills/nyxid/tools/services.sh`: list available proxy services
- `skills/nyxid/tools/proxy.sh`: send a proxied request through NyxID

Both scripts accept either `NYXID_ACCESS_TOKEN` or `NYXID_API_KEY`.
