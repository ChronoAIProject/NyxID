---
title: Connect an AI service
description: The full nyxid service flow — catalog, custom endpoints, OAuth and device-code providers, inspecting, rotating, and routing a connection.
---

The [Get Started flow](/docs/cli/getting-started/first-connection) connects one catalog service in four steps. This guide covers the rest of the surface: discovering what's available, wiring up custom and OAuth services, and maintaining a connection over time.

Every connection provisions three records — an [endpoint, a key, and a service](/docs/shared/concepts/endpoints-keys-services) — in a single `nyxid service add`. You rarely touch them individually.

## Browse the catalog

The catalog is the set of services NyxID already knows how to reach (base URL, auth method, OpenAPI spec).

```bash
nyxid catalog list              # connectable services only
nyxid catalog list --all        # also includes system services
nyxid catalog show llm-openai   # base URL, auth, capabilities for one entry
nyxid catalog endpoints llm-openai  # operations parsed from its OpenAPI spec
```

## Add from the catalog

Pass the catalog slug and point NyxID at the credential via an environment variable so the secret never lands in your shell history:

```bash
export OPENAI_API_KEY=sk-...
nyxid service add llm-openai --credential-env OPENAI_API_KEY
```

The CLI prints a `Slug:` line — that's the handle you proxy through. If you already had an `llm-openai`, the new one is suffixed (e.g. `llm-openai-2`).

:::tip
For multi-field credentials like `aws_sigv4`, use `--credential-file <path>` (or `-` for stdin) instead of `--credential-env`.
:::

## Add a custom (non-catalog) endpoint

For a private or in-house API the catalog doesn't list, use `--custom` with an explicit endpoint and auth method:

```bash
nyxid service add --custom \
  --slug my-internal-api \
  --endpoint-url https://internal.example.com \
  --auth-method bearer \
  --auth-key-name Authorization \
  --credential-env MY_API_KEY
```

`--auth-method` accepts `bearer`, `bot_bearer` (Discord-style `Bot ` prefix), `header`, `query`, `path`, `basic`, `body` (inject into the JSON body), or `none` (no credential). `--auth-key-name` is the header/query/field name the credential goes into.

:::note
If the API lives behind a firewall or on localhost, route it through a [credential node](/docs/cli/guides/credential-node) instead — add `--via-node <node-id>` and hold the credential on the node.
:::

## Connect an OAuth or device-code provider

For providers that use OAuth instead of a static key, swap `--credential-env` for a flow flag. The CLI walks you through authorization in the browser:

```bash
nyxid service add api-lark --oauth
nyxid service add llm-openai --device-code   # for headless / no-browser hosts
```

Request extra scopes on top of the provider's defaults with `--scope` (repeatable, comma- or space-separated). For providers that require your own OAuth app (Lark / Feishu / X), supply the client credentials so per-connection token refresh keeps working:

```bash
export LARK_APP_SECRET=...
nyxid service add api-lark --oauth \
  --oauth-client-id cli_xxx \
  --oauth-client-secret-env LARK_APP_SECRET \
  --scope "contact:contact.base:readonly"
```

`--scope` is **additive** — it adds to the provider's default scopes. To change the scopes of a connection you already have (including removing one), use `service scopes` below.

## Change an OAuth connection's scopes

To re-scope an existing OAuth connection, declare the exact set you want with `service scopes <id|slug> --set`:

```bash
# Replace this connection's scopes with exactly this set (adds and/or removes)
nyxid service scopes api-twitter --set "tweet.read,media.write"

# No --set: opens the picker seeded with the connection's current scopes
nyxid service scopes api-twitter

# Headless host: print a URL + code to approve on another device, then exit
nyxid service scopes api-twitter --set "tweet.read,media.write" --no-wait
```

This re-authorizes the **same** connection in place — its agent bindings, node routing, and org sharing are preserved (it never creates a duplicate). Because changing scopes means re-consenting, the provider's authorization screen still opens in a browser, exactly as when you first add the connection; `--no-wait` lets that browser be on a different device.

What you can remove depends on the provider:

| Provider behavior | Removal |
|---|---|
| Has a token-revocation endpoint (X, Google, Discord, Slack, Twitch, Reddit, TikTok) | Removal applies; the old grant is revoked. |
| No revocation endpoint (Facebook, Spotify, LinkedIn, Microsoft, Lark/Feishu) | NyxID uses only the new scopes, but the old grant lingers until you revoke it in the provider's own settings. |
| Grant-level scope union (GitHub) | Granted scopes are locked — re-auth can't narrow them. You can still add scopes. |

> **Note:** dropping `offline.access` ("Stay connected") removes the refresh token, so the connection stops auto-refreshing and will need reconnecting when the access token expires. Keep it unless you mean to.

## Inspect and verify

```bash
nyxid service list                  # all your services + their slugs and IDs
nyxid service show <id>             # one service's full config
nyxid proxy request <slug> models   # a real proxied call — expect HTTP 200
```

The proxy injects the stored credential server-side; it never travels from your terminal. See [the proxy](/docs/shared/concepts/the-proxy) for the full request path.

## Maintain a connection

Find the service ID with `nyxid service list`, then:

```bash
# Rotate the upstream credential
nyxid service rotate-credential <id> --credential-env NEW_KEY_VAR

# Switch routing between a node and direct
nyxid service route <id> --node <node-id>
nyxid service route <id> --direct

# Retarget or relabel
nyxid service update <id> --endpoint-url https://new.example.com --label "New label"

# Remove it
nyxid service delete <id>
```

## Share a service across an org

Add `--org <id|slug|name>` to `service add` (you must be an org admin). Every member then sees the service in their own `nyxid service list` and proxies through it with their own account — see [Create scoped agent keys](/docs/cli/guides/scoped-agent-keys) and the organizations model.

## Next

- [Your first agent call](/docs/ai/getting-started/first-agent-call) — let an AI agent reach this service over MCP.
- [Set up a credential node](/docs/cli/guides/credential-node) — proxy to localhost / firewalled APIs.
- [Command reference: `service`](/docs/cli/reference/service) — every subcommand and flag, in one place.
