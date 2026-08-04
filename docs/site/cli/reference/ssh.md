---
title: nyxid ssh
description: Reference for nyxid ssh — issue certificates, open tunnels, run remote commands, and open terminals against SSH services brokered by NyxID.
---

`nyxid ssh` is the client side of NyxID's SSH brokering. The service must already exist and route through a node — create one with `nyxid service add-ssh` (see [Set up an SSH node](/docs/cli/guides/ssh-node)). Every subcommand takes a service ID, slug, or name as its first argument.

:::note
Subcommands accept the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, and `--output`. See [Authenticate](/docs/cli/getting-started/authenticate).
:::

## ssh exec

```bash
nyxid ssh exec <service> [--principal <user>] -- <command...>
```

Run a single command on the remote host and stream the result back. Works for both `cert` and `node_key` services. Everything after `--` is the remote command.

## ssh terminal

```bash
nyxid ssh terminal <service> [--principal <user>]
```

Open an interactive terminal over the WebSocket tunnel. Works for both `cert` and `node_key` services.

## ssh proxy

```bash
nyxid ssh proxy <service> [flags]
```

Open an SSH-over-WebSocket tunnel for use as an OpenSSH `ProxyCommand`. **Certificate mode only** — a `node_key` service has no client-held key to present.

- `--issue-certificate` — issue a fresh certificate inline.
- `--principal <user>`, `--public-key-file <path>`, `--certificate-file <path>`, `--ca-public-key-file <path>`.

## ssh issue-cert

```bash
nyxid ssh issue-cert <service> \
  --public-key-file <pub> --principal <user> --certificate-file <out> \
  [--ca-public-key-file <path>]
```

Issue a short-lived SSH user certificate and write it to `--certificate-file`. Use when you want to manage the SSH client invocation yourself.

## ssh config

```bash
nyxid ssh config \
  --host-alias <alias> --base-url <url> --service-id <id> \
  --principal <user> --identity-file <key> --certificate-file <cert> \
  [--access-token-env NYXID_ACCESS_TOKEN] [--ca-public-key-file <path>]
```

Print a ready-to-paste `~/.ssh/config` stanza that wires `nyxid ssh proxy` in as the `ProxyCommand` for a host alias, so plain `ssh <alias>` just works.
