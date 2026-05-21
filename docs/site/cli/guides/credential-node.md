---
title: Set up a credential node
description: Register an on-premise NyxID node, hold credentials locally, and route a service through it to reach localhost or firewalled APIs.
---

A **credential node** is a small agent you run on your own hardware. The upstream credential and the request both stay on your network — NyxID routes the proxied call to the node over an outbound WebSocket, and the node injects the secret locally. Use it for APIs on `localhost`, behind a firewall, or on a private VPC that NyxID's servers can't reach directly. For the model behind this, see [Credential nodes](/docs/shared/concepts/credential-nodes).

This guide assumes the [CLI is installed](/docs/cli/getting-started/install) and you are [logged in](/docs/cli/getting-started/authenticate). Steps 1 and 5 run from your laptop; steps 2–4 run on the machine that will be the node.

## 1. Mint a registration token (laptop)

Registration tokens are one-time and short-lived (default 1h). Minting requires admin role; the token carries the chosen owner at mint time.

```bash
nyxid node register-token --name edge-box
```

This prints a `nyx_nreg_...` token. Add `--org <id|slug|name>` to register the node under an organization you administer. Add `--terminal` to print the token straight to the terminal instead of opening the browser wizard.

## 2. Register the node (on the box)

On the machine that will run the node, redeem the token against your instance's node WebSocket URL:

```bash
nyxid node register \
  --token nyx_nreg_... \
  --url wss://nyx-api.chrono-ai.fun/api/v1/nodes/ws
```

For a self-hosted backend use `ws://localhost:3001/api/v1/nodes/ws`. Registration generates the node's auth token and HMAC signing secret and stores them encrypted under `~/.nyxid-node/`. Add `--keychain` to store secrets in the OS keychain instead of an encrypted file.

## 3. Run the node agent

Foreground, for a quick test:

```bash
nyxid node start
nyxid node agent-status   # local config + connection state
```

For a long-lived background service, install it under your platform's service manager instead:

```bash
nyxid node daemon install
nyxid node daemon start
nyxid node daemon status
nyxid node daemon logs --follow
```

`daemon install` creates a launchd LaunchAgent on macOS or a systemd user unit on Linux. To run more than one node on a single machine, pass `--profile <name>` to every node command — each profile gets its own config directory and service label. There is also a `nyxid node docker` family if you'd rather run the agent in a container.

## 4. Store the credential on the node

The secret lives only on the node — NyxID never sees it. Add it locally, keyed by service slug:

```bash
nyxid node credentials add --service my-internal-api --header Authorization
nyxid node credentials list
```

You are prompted for the secret value (it is not passed on the command line). Use `--query-param <name>` instead of `--header` for query-string auth, and `--secret-format bearer` to have the node prefix the stored value with `Bearer ` on injection (`basic` base64-encodes `user:pass`). To remove one later: `nyxid node credentials remove --service my-internal-api`.

## 5. Route a service through the node (laptop)

Point a service at the node so proxied calls flow NyxID → node → upstream. Either create the service already routed:

```bash
nyxid service add --custom \
  --slug my-internal-api \
  --endpoint-url https://internal.example.com \
  --auth-method bearer \
  --via-node <node-id>
```

…or retarget an existing service:

```bash
nyxid service route <service-id> --node <node-id>   # find IDs with nyxid node list / nyxid service list
nyxid service route <service-id> --direct           # switch back to direct
```

When a service is node-routed, the credential entered at the node (step 4) is the one injected — you don't store it on NyxID.

## 6. Verify

```bash
nyxid proxy request my-internal-api health
```

An `HTTP/1.1 200` means the request reached your upstream through the node. Node-routed audit events are tagged with `"routed_via": "node"` and the `node_id`.

## Maintain a node

```bash
nyxid node list                    # all your nodes + status (Online / Offline / Draining)
nyxid node show <id>               # one node's detail + metrics
nyxid node rotate-token <id>       # rotate auth token + signing secret (server side)
nyxid node delete <id>             # deregister
```

After a server-side rotation, update the running agent with the new values: `nyxid node rekey --auth-token nyx_nauth_... --signing-secret <hex>`.

## Next

- [Set up an SSH node](/docs/cli/guides/ssh-node) — reach SSH hosts behind the same node.
- [`node` command reference](/docs/cli/reference/node) — every node subcommand and flag.
- [Credential nodes](/docs/shared/concepts/credential-nodes) — why the credential and request never leave your network.
