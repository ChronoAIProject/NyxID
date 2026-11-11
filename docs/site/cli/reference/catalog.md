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

## catalog resync-identity

```bash
nyxid catalog resync-identity <service-id-or-slug> [--yes]
```

Overwrite the identity-forwarding settings (identity propagation mode and included user fields, JWT audience, access-token forwarding, delegation-token injection and scope) on every connection to a catalog service with the service's current settings, including connections that were customized. Connections that already match are left unchanged; the command prints how many were checked and how many were updated. It does not change stored credentials or anyone's sign-in with the service.

Only an Admin or the service's creator can run it, from a human account login (not an Agent Key). Catalog Managers cannot. It asks for confirmation unless you pass `--yes`; `--output json` prints the server response (`matched_count`, `changed_count`).
