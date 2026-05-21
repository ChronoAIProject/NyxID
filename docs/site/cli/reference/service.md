---
title: nyxid service
description: Reference for nyxid service — add, inspect, route, and rotate the external services you proxy through NyxID.
---

`nyxid service` manages your connected external services. Each `add` provisions an [endpoint, a key, and a service](/docs/shared/concepts/endpoints-keys-services) in one step. For the end-to-end walkthrough see [Connect an AI service](/docs/cli/guides/connect-a-service).

:::note
Every subcommand accepts the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, and `--output table|json`. See [Authenticate](/docs/cli/getting-started/authenticate).
:::

## service add

```bash
nyxid service add [SLUG] [flags]
```

Add a service from the catalog (`SLUG`) or a fully custom endpoint (`--custom`). Provisions endpoint + credential + routing.

- `--custom` — define a non-catalog endpoint (use with `--endpoint-url`, `--auth-method`).
- `--slug <slug>` — custom slug for the new service (auto-derived if omitted; must be unique per user).
- `--endpoint-url <url>` — target base URL (override or custom).
- `--label <text>` — display label.
- `--auth-method <m>` — `bearer`, `bot_bearer`, `header`, `query`, `path`, `basic`, `body`, or `none`.
- `--auth-key-name <name>` — header/query/body field the credential goes into (e.g. `Authorization`, `X-API-Key`).
- `--credential-env <VAR>` — read the secret from an environment variable (preferred).
- `--credential-file <path>` — read credential bytes from a file (`-` for stdin); use for multi-field credentials like `aws_sigv4`.
- `--oauth` / `--device-code` — authenticate via OAuth or device-code flow instead of a static key.
- `--scope <scopes>` — extra OAuth scopes on top of the provider's defaults (repeatable).
- `--oauth-client-id` / `--oauth-client-secret-env` — bring-your-own OAuth app credentials (Lark / Feishu / X).
- `--via-node <node-id>` — route through a [credential node](/docs/cli/guides/credential-node).
- `--openapi-spec-url <url>` — OpenAPI spec for endpoint discovery (`""` opts out of the catalog default).
- `--org <id|slug|name>` — create the service under an organization (admin only).
- `--terminal` — skip the browser wizard; `--no-wait` — remote-pair mode (resume with `nyxid pairing resume`).

## service add-ssh

```bash
nyxid service add-ssh --label <text> --host <host> --via-node <node-id> [flags]
```

Create an SSH service (always node-routed). See [Set up an SSH node](/docs/cli/guides/ssh-node).

- `--cert-auth` — NyxID-issued certificate mode (mutually exclusive with `--node-key`).
- `--node-key` — node-local private-key mode.
- `--principals <list>` — comma-separated SSH principals.
- `--port <n>` — SSH port (default 22). `--ttl <min>` — certificate lifetime (cert mode, default 30).
- `--org <id|slug|name>` — create under an organization.

## service convert-ssh

```bash
nyxid service convert-ssh <slug> [--to-cert | --to-node-key | --to-proxy-only]
```

Switch an SSH service between its three auth modes.

## service list

```bash
nyxid service list
```

List all your services with their slugs and IDs. Use `--output json` to grab UUIDs for scripting.

## service show

```bash
nyxid service show <id>
```

Full config for one service.

## service update

```bash
nyxid service update <id> [flags]
```

- `--label`, `--endpoint-url`, `--openapi-spec-url` (`""` clears).
- `--node-id <id>` / `--no-node` — set or remove node routing.
- `--active` / `--inactive` — toggle service state.
- `--default-header 'name=value[:overridable]'` — set default injected headers (repeatable; replaces the current set). `--clear-default-headers` removes them.
- `--ws-frame-preset <name>` / `--ws-frame-clear` — WebSocket auth-frame rules.

## service rotate-credential

```bash
nyxid service rotate-credential <id> --credential-env <NEW_VAR>
```

Replace the upstream credential for a service.

## service route

```bash
nyxid service route <id> [--node <node-id> | --direct]
```

Switch a service between node-routed and direct.

## service credentials

```bash
nyxid service credentials <slug> --client-id-env <VAR> --client-secret-env <VAR>
```

Set the OAuth client credentials for a service's provider.

## service delete

```bash
nyxid service delete <id> [--yes]
```

Remove a service. `--yes` skips the confirmation prompt.
