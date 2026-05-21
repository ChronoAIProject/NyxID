---
title: Set up an SSH node
description: Reach SSH hosts through a credential node — with NyxID-issued certificates or node-local private keys — and run commands or open a terminal without distributing keys.
---

NyxID brokers SSH the same way it brokers HTTP: a [credential node](/docs/cli/guides/credential-node) sits next to your hosts, and you reach them through it without ever distributing private keys or certificates to the people (or agents) doing the work. An SSH service picks one of three auth modes:

- **`cert`** — NyxID issues a short-lived SSH user certificate per session. The CA is trusted by your hosts; no static keys live anywhere.
- **`node_key`** — the private key lives only in the node's encrypted store, keyed by `(service, principal)`. NyxID and clients never see it.
- **`proxy_only`** — NyxID only tunnels the TCP stream; auth happens entirely at your client.

This guide assumes you already have a node registered and running ([Set up a credential node](/docs/cli/guides/credential-node)). Every SSH service must route through a node, so `--via-node` is required.

## Create the SSH service

Pick the auth mode with `--cert-auth` or `--node-key` (omit both for `proxy_only`):

```bash
# Certificate mode — NyxID's CA signs a fresh cert each session
nyxid service add-ssh \
  --label "prod-web" \
  --host 10.0.0.5 \
  --via-node <node-id> \
  --cert-auth \
  --principals deploy \
  --ttl 30

# Node-key mode — the private key never leaves the node
nyxid service add-ssh \
  --label "prod-db" \
  --host 10.0.0.6 \
  --via-node <node-id> \
  --node-key \
  --principals postgres
```

`--ttl` sets the certificate lifetime in minutes (cert mode only; default 30). Add `--org <id|slug|name>` to create an org-owned SSH service that members reach through their own accounts.

## Node-key mode: store the key on the node

In `node_key` mode the private key is held on the node, not on NyxID. On the node machine, add it per principal:

```bash
nyxid node ssh-credentials add \
  --service prod-db \
  --principal postgres \
  --key-file ./postgres_ed25519 \
  --host 10.0.0.6
```

The target host key is pinned by default (pass `--no-pin-host-key` to opt out). Use `--passphrase-env <VAR>` for an encrypted key, and the `--kex` / `--host-key` / `--cipher` / `--mac` allowlists to constrain the SSH algorithms. Inspect and test entries with:

```bash
nyxid node ssh-credentials list
nyxid node ssh-credentials test --service prod-db --principal postgres
```

## Connect

For both `cert` and `node_key` modes, the simplest paths are remote exec and the interactive terminal:

```bash
nyxid ssh exec prod-web --principal deploy -- uptime
nyxid ssh terminal prod-web --principal deploy
```

For full OpenSSH-client integration in **cert mode**, open a tunnel as a `ProxyCommand` (this issues a certificate inline):

```bash
nyxid ssh proxy prod-web \
  --issue-certificate \
  --principal deploy \
  --public-key-file ~/.ssh/id_ed25519.pub \
  --certificate-file /tmp/prod-web-cert.pub
```

`nyxid ssh config` prints a ready-to-paste `~/.ssh/config` stanza wiring that `ProxyCommand` to a host alias.

:::warning
`nyxid ssh proxy` is **not** supported for `node_key` services — there is no client-held key to present. Use `nyxid ssh exec` or `nyxid ssh terminal` for node-key hosts.
:::

## Change auth mode later

```bash
nyxid service convert-ssh prod-web --to-node-key    # cert  → node_key
nyxid service convert-ssh prod-db  --to-cert        # node_key → cert
nyxid service convert-ssh prod-web --to-proxy-only  # → proxy_only
```

After converting *to* `node_key`, add the node-local key (above) before connecting. After converting *away* from it, prune the now-unused key on the node: `nyxid node ssh-credentials prune --stale`.

## Next

- [`ssh` command reference](/docs/cli/reference/ssh) — issue-cert, proxy, config, exec, terminal.
- [`node` command reference](/docs/cli/reference/node) — `ssh-credentials` and the rest of the node surface.
- [Credential nodes](/docs/shared/concepts/credential-nodes) — the routing model SSH shares with HTTP.
