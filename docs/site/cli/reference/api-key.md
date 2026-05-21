---
title: nyxid api-key
description: Reference for nyxid api-key — create, scope, bind, and rotate the NyxID API keys that identify your agents.
---

`nyxid api-key` manages NyxID API keys. A key **is** an agent identity: it carries its own service scope, rate limits, credential bindings, and audit attribution. For the procedure see [Create scoped agent keys](/docs/cli/guides/scoped-agent-keys); for the model see [Agent isolation](/docs/shared/concepts/agent-isolation).

:::note
Every subcommand accepts the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, and `--output table|json`. See [Authenticate](/docs/cli/getting-started/authenticate).
:::

## api-key create

```bash
nyxid api-key create --name <name> [flags]
```

Create a key. The `nyxid_ag_...` secret is shown **once**.

- `--scopes "read write proxy"` — space-separated scope subset.
- `--platform <p>` — `claude-code`, `codex`, `cursor`, `openclaw`, `generic` (recorded for audit).
- `--expires-in-days <n>` — auto-expiry (0 = no expiry).
- `--allowed-services <ids>` / `--allowed-nodes <ids>` — comma-separated scope lists.
- `--allow-all-services` / `--allow-all-nodes` — grant blanket access.
- `--callback-url <url>` — relay callback for channel bots.
- `--org <id|slug|name>` — create a key that authenticates as an organization.
- `--terminal` — print the key to the terminal; `--no-wait` — remote-pair mode.

## api-key list

```bash
nyxid api-key list [--org <id|slug|name>]
```

List keys with platform and bindings count. `--org` lists an org's keys (admin only).

## api-key show

```bash
nyxid api-key show <id>
```

Scope, rate limits, and credential bindings for one key.

## api-key update

```bash
nyxid api-key update <id> [flags]
```

- `--name`, `--scopes`.
- `--allowed-services <ids>` + `--allow-all-services false` — restrict to specific services. **Both are required** to enforce a scope.
- `--allowed-nodes <ids>` + `--allow-all-nodes false` — same for nodes.
- `--callback-url <url>` — set (or `""` to clear) the relay callback.

## api-key bind

```bash
nyxid api-key bind <id> --service <slug> [--credential <label>]
```

Bind a per-agent credential override for one service. Omit `--credential` to auto-resolve it from the service. Without a binding, the proxy uses the service's default credential.

## api-key rotate

```bash
nyxid api-key rotate <id> [--terminal | --no-wait]
```

Issue a new secret for the key, keeping its scope. The old secret stops working immediately.

## api-key delete

```bash
nyxid api-key delete <id> [--yes]
```

Revoke a key. `--yes` skips confirmation.
