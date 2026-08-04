---
title: nyxid node
description: Reference for nyxid node — register and run on-premise credential nodes, manage their local credentials, and operate the background agent.
---

`nyxid node` covers both sides of a [credential node](/docs/shared/concepts/credential-nodes): the **user side** (server API calls — mint tokens, list, rotate) and the **agent side** (local commands run on the node machine — register, start, hold credentials). For the setup walkthrough see [Set up a credential node](/docs/cli/guides/credential-node).

:::note
User-side subcommands accept the common flags `--base-url`, `--access-token` / `--access-token-env`, `--profile`, `--output`. Agent-side subcommands accept `--config <dir>` and `--profile <name>` (env `NYXID_PROFILE`) to support multiple node instances on one machine.
:::

## User-side (server API)

- **`node list`** — list your nodes with status (`Online` / `Offline` / `Draining`).
- **`node show <id>`** — node detail and metrics.
- **`node register-token`** — mint a one-time registration token. `--name <name>`, `--org <id|slug|name>` (admin), `--terminal`, `--no-wait`.
- **`node rotate-token <id>`** — rotate the node's auth token + signing secret server-side. `--terminal`, `--no-wait`.
- **`node transfer <id> --to <user-or-org-id>`** — reassign ownership. `--yes` skips confirmation.
- **`node delete <id>`** — deregister a node. `--yes` skips confirmation.

## Agent-side (on the node machine)

- **`node register --token nyx_nreg_... --url <ws-url>`** — redeem a token and store the node's secrets locally. `--keychain` uses the OS keychain instead of an encrypted file; `--config <dir>`.
- **`node start`** — connect and serve in the foreground. `--log-level <level>`.
- **`node agent-status`** — show local config and connection state.
- **`node rekey --auth-token nyx_nauth_... --signing-secret <hex>`** — apply new secrets after a server-side `rotate-token`.
- **`node migrate --to keychain|file`** — move secret storage between the encrypted file and the OS keychain.
- **`node agent-version`** — print the node agent version.

## node credentials (local secrets)

The upstream secrets a node injects, held only on the node:

- **`add --service <slug>`** — add a credential (prompts for the secret). `--header <name>` or `--query-param <name>`, `--url <target>`, `--secret-format raw|bearer|basic`.
- **`add-oauth --service <slug>`** — run an OAuth / device-code flow for the service (`--from-catalog` to pull config from NyxID).
- **`setup --service <slug>`** — auto-resolve a service's credential requirements from the catalog.
- **`list`** — configured credentials. **`pending`** — pushed credentials awaiting acceptance.
- **`accept <slug>`** / **`decline <slug>`** — act on a pushed credential.
- **`remove --service <slug>`** — delete a credential.

## node ssh-credentials (local SSH keys)

Per-principal private keys for `node_key` SSH services (see [Set up an SSH node](/docs/cli/guides/ssh-node)):

- **`add --service <slug> --principal <p> --key-file <pem> --host <h>`** — add a key. `--port`, `--passphrase-env <VAR>`, `--no-pin-host-key`, and `--kex` / `--host-key` / `--cipher` / `--mac` algorithm allowlists.
- **`set-algos`** — set or reset the algorithm allowlists for one credential (`--reset-all`, etc.).
- **`list [--service <slug>]`**, **`show --service --principal`**, **`test --service --principal`**, **`remove --service --principal`**.
- **`prune --stale`** — drop keys for services no longer in node-key mode.

## node daemon (background service)

Run the agent under launchd (macOS) or systemd (Linux):

- **`install`** (`--log-level`, `--force`), **`uninstall`**, **`start`**, **`stop`**, **`restart`**, **`status`**.
- **`logs`** — `--follow` to tail, `--lines <n>` for recent output.

## node docker

Run the agent in a container instead of a native service: **`build`**, **`start`**, **`stop`**, **`restart`**, **`status`**, **`logs`** (`--follow`). Each `--profile` runs as a separate container.

## node openclaw

Manage an OpenClaw gateway connection from the node: **`connect --url <gateway-url> [--token <t>]`**, **`status`**, **`disconnect`**.

:::note
Pushing credential setup metadata *to* a node operator is a separate top-level command: `nyxid node-credential push <node> --slug <s> --injection-method header|query-param|path-prefix --field-name <name>`, plus `node-credential list <node>` and `node-credential cancel <node> <pending-id>`.
:::
