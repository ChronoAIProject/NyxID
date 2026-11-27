---
title: Your first CLI request
description: Check your NyxID identity, find a GitHub connection and read the username of the authorized GitHub account.
---

Start with the released CLI and a NyxID account. Complete the browser-approved login, then run `nyxid whoami` to confirm your identity. [Install the CLI](/docs/cli/getting-started/install) · [Authenticate](/docs/cli/getting-started/authenticate#log-in)

## 1. Set the provider credential

For GitHub OAuth, have the GitHub account you want to authorize ready in your browser. A managed connector uses its configured OAuth app. If your instance needs your own app, register the callback for that instance first. [GitHub app setup and alternatives](/docs/web/getting-started/first-connection#self-host)

## 2. Add the service from the catalog

If you've already connected GitHub, use that connection. Otherwise, connect it in Studio or use the CLI's catalog OAuth workflow. [Connect GitHub in Studio](/docs/web/getting-started/first-connection#2-add-an-ai-service) · [Connect a service from the CLI](/docs/cli/guides/connect-a-service#connect-an-oauth-or-device-code-provider)

## 3. Copy the returned slug

```bash
nyxid whoami
nyxid service list
```

Confirm that the identity belongs to the account you used in Studio. Copy the GitHub connection's slug from the list. The example below uses the usual first OAuth connection.

## 4. Verify with a proxied request

```bash
SERVICE_SLUG='api-github'
nyxid proxy request "$SERVICE_SLUG" user
```

This sends a GET request. Look for the username of the GitHub account you authorized. A profile for the user `octocat` includes these fields.

```json
{"login":"octocat","id":583231}
```

### If it errors

If the connection is missing, check the account, CLI profile and connection owner. For a restricted profile, check its granted services. If an approval is pending, review it in Studio and follow the CLI's instructions for that request ID. For scope errors or provider reauthorization, see [Studio troubleshooting](/docs/web/getting-started/first-connection#troubleshooting).

## Next

See which methods and paths you can call from the CLI, or use the same connection from your AI client. [CLI proxy reference](/docs/cli/reference/proxy) · [Your first agent call](/docs/ai/getting-started/first-agent-call)
