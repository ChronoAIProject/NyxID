---
title: Your first CLI request
description: Check your NyxID identity, find a GitHub connection and read the username of the authorized GitHub account.
---

[Install the CLI](/docs/cli/getting-started/install), then [authenticate](/docs/cli/getting-started/authenticate#log-in). You need a NyxID account and a connected GitHub service. If the account is new, [connect GitHub in Studio](/docs/web/getting-started/first-connection#connect-a-service).

## Check your account and list connections

```bash
nyxid whoami
nyxid service list
```

Confirm that the identity is the one you used in Studio. Look for the GitHub connection in the list and copy its slug. If it is missing, check the account, CLI profile, and the service owner in Studio. When using a restricted CLI profile, check the selected key's access to the connection.

## Read your GitHub profile

Set `SERVICE_SLUG` to the exact slug of your connection. The example uses the usual first GitHub OAuth connection. The request uses HTTP GET.

```bash
SERVICE_SLUG='api-github'
nyxid proxy request "$SERVICE_SLUG" user
```

Look for your GitHub login in the JSON response, for example:

```json
{"login":"octocat","id":583231}
```

If NyxID returns 403, check the profile's proxy access and allowed services. If GitHub returns 401, reauthorize the GitHub connection in Studio, then repeat the same read. [CLI proxy reference](/docs/cli/reference/proxy) · [Connect another service from the CLI](/docs/cli/guides/connect-a-service)
