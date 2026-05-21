---
title: nyxid catalog
description: Reference for nyxid catalog — browse the services NyxID already knows how to reach and inspect their API endpoints.
---

The catalog is the read-only set of services NyxID already knows how to connect to — base URL, auth method, capabilities, and (where available) an OpenAPI spec. Browse it to find the slug to pass to [`nyxid service add`](/docs/cli/reference/service).

:::note
Every subcommand accepts the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, and `--output table|json`. See [Authenticate](/docs/cli/getting-started/authenticate).
:::

## catalog list

```bash
nyxid catalog list [--all]
```

List connectable catalog entries. `--all` also includes system services (those without user auth).

## catalog show

```bash
nyxid catalog show <slug>
```

Show one entry's detail: base URL, auth method, capabilities, auth notes, known limitations, and required permissions.

## catalog endpoints

```bash
nyxid catalog endpoints <slug>
```

List the API operations parsed from the entry's OpenAPI spec — the operations an AI agent can discover and call through the proxy. (Requires the catalog entry to have an `openapi_spec_url`.)
