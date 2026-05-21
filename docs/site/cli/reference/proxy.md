---
title: nyxid proxy
description: Reference for nyxid proxy — discover proxyable services and send credential-injected requests through NyxID.
---

`nyxid proxy` sends requests through NyxID to your connected services. NyxID injects the stored credential server-side — the secret never travels from your terminal. For how a proxied request flows, see [The proxy](/docs/shared/concepts/the-proxy).

:::note
Both subcommands accept the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, and `--output table|json`. See [Authenticate](/docs/cli/getting-started/authenticate).
:::

## proxy discover

```bash
nyxid proxy discover
```

List the services you can proxy through (paginated service discovery), with their slugs.

## proxy request

```bash
nyxid proxy request <service> [path] [flags]
```

Send a request to `<service>` (slug or UUID) at `path` (e.g. `models`, `v1/chat/completions`). A `200` carries the upstream's real response body.

- `--method, -m <verb>` — HTTP method (default `GET`).
- `--data, -d <body>` — request body: a JSON string, `@file` to read from a file, or `-` for stdin.
- `--header, -H 'Key:Value'` — extra request headers (repeatable).
- `--stream` — stream the response (SSE, audio/video, large downloads).
- `--by-id` — treat `<service>` as a service ID rather than a slug.
- `--via-service <user-service-id>` — pin a specific service instance when the same slug exists in more than one scope (e.g. personal vs. org). Get the ID from `nyxid service list --output json`.
