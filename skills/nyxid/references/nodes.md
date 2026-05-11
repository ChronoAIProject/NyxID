# Node Management and SSH Remote Access

## Table of contents

- [Node Management](#node-management)
  - [Org-owned nodes](#org-owned-nodes)
  - [Two-machine org-node setup](#two-machine-org-node-setup)
  - [Remote credential provisioning](#remote-credential-provisioning)
  - [Transferring an existing personal node to an org](#transferring-an-existing-personal-node-to-an-org)
  - [Setting up a new node](#setting-up-a-new-node)
  - [Managing the node service](#managing-the-node-service)
  - [Managing nodes](#managing-nodes)
- [SSH Remote Access](#ssh-remote-access)

## Node Management

Nodes are for users who do not want their credentials stored on the NyxID server. Instead, credentials stay encrypted on the user's own machine (the node). When a proxy request comes in, NyxID passes it through to the node agent via WebSocket, the node injects the credential locally and forwards the request to the downstream service. The credential never leaves the node.

`Node.user_id` is a polymorphic owner field, matching `UserService`: it can point to a person user or an org user. Do not add or expect a separate node `org_id`.

### Org-owned nodes

Org admins can mint registration tokens for an org:

```bash
# Default — opens the browser wizard (v3.1 DisplayOnce on the new nyx_nreg_…)
nyxid node register-token --org <ID|SLUG|NAME>

# Headless agent — get a machine-readable pairing URL and resume later
nyxid node register-token --org <ID|SLUG|NAME> --no-wait --output json
nyxid pairing resume <pairing_id>
```

The redeemed node belongs to the org. All current org admins can manage it; org members can list it and proxy through org services routed to it. Only org admins can create org-scoped registration tokens, delete org-owned nodes, rotate node auth tokens, or manage node bindings.

`nyxid node list` includes accessible personal and org-owned nodes. `nyxid node show <ID_OR_NAME>` prints owner metadata and the admin list. The API endpoint `GET /api/v1/nodes/{node_id}/admins` returns the users who can manage a node: the personal owner for personal nodes, or all org admins for org-owned nodes.

Audit events for shared node operations include `owner_user_id` when the actor differs from the owner, so org-owned node activity can be attributed to both the actor and owning org.

Registration tokens carry the chosen owner at mint time. Admin role is verified when the token is issued, not when it is redeemed, so a token issued before an admin is revoked can still register a node until the token expires. The default TTL is 1 hour (`NODE_REGISTRATION_TOKEN_TTL_SECS`); delete pending registration tokens for that owner when removing org admins.

### Two-machine org-node setup

For confidential shared infrastructure, keep user login on the admin's laptop and run only node-agent commands on the shared VM.

| Machine | Runs |
|---|---|
| Laptop | `nyxid login`, `nyxid node register-token --org`, `nyxid service add <slug> --org <ORG> --via-node <NODE>` to mint the org-owned `UserService`, all node management ops (`rotate-token`, `transfer`, `delete`, manage bindings) |
| VM | `nyxid node register --token`, `nyxid node credentials add` (purely local), `nyxid node start` / daemon. Never `nyxid login` on the VM. |

The node's local credential store is keyed by service slug (`config.toml: credentials[<slug>]`). The binding to a NyxID `UserService` is established by matching slugs. Both ends must agree on the slug; no user identity passes through the VM.

Fresh shared-box checklist:

1. Admin on laptop: `nyxid node register-token --org <ID|SLUG|NAME>` -> copy the `nyx_nreg_...` token.
2. Operator on VM: `nyxid node register --token nyx_nreg_... --url wss://...` -> token is consumed, node receives its own `nyx_nauth_...` stored in the OS keychain or local secret backend.
3. Admin on laptop: `nyxid service add <slug> --org <ID|SLUG|NAME> --via-node <node-id-or-name>` -> creates the org-owned `UserService` routing through the node.
4. Operator on VM: `nyxid node credentials add --service <slug> --header X-API-Key` (interactive secret prompt) -> credential stored locally, encrypted.
5. Operator on VM: `nyxid node start` (or `nyxid node daemon install` + `start`).
6. Optional: Admin on laptop: `nyxid node transfer <node-id> --to <org-id>` if the node was registered to a person owner first.

When an admin leaves the org, rotate the node's auth token (`nyxid node rotate-token <node-id>`), audit pending registration tokens for that org and revoke them, and audit any per-agent credential bindings created by the leaving admin.

Org membership concepts are in [`organizations.md`](organizations.md). The broader management surface is in [`managing.md`](managing.md).

### Remote credential provisioning

Remote credential provisioning lets an org admin push setup metadata to a node operator without seeing or transmitting the secret value. NyxID stores only the pending credential metadata: node ID, service slug, injection method, field name, optional target URL, label, creator, owner, and expiry. The secret is entered on the VM and stored only in the node's local encrypted credential store.

| Laptop (admin) | VM (operator) |
|---|---|
| `nyxid node-credential push <node-id-or-name> --slug <slug> --injection-method header --field-name X-API-Key [--target-url ...] [--label ...] --output json` |  |
| Relay the slug, injection method, and field name to the operator. Do not send a secret value. | `nyxid node credentials pending` |
|  | Verify the slug, method, field name, and target URL. |
|  | `nyxid node credentials accept <slug>` and enter the secret when prompted. For non-interactive provisioning, use `--value-env <ENVVAR>`. |
| `nyxid node-credential list <node-id-or-name> --output json` |  |
| `nyxid node-credential cancel <node-id-or-name> <pending-id>` if the push is wrong or stale. | `nyxid node credentials decline <slug> --reason "wrong target"` if the operator refuses it. |

Pending credentials expire automatically. The default TTL is 24 hours (`NODE_PENDING_CREDENTIAL_TTL_SECS`). Expired entries are not returned to admins or nodes; create a fresh push if the operator misses the window.

Decline means the VM operator reviewed the metadata and refused to store a local secret for it. It does not remove or modify existing local credentials. Cancel means an admin withdrew the pending push before acceptance; a later VM-side accept for that pending ID fails.

### Transferring an existing personal node to an org

Use `nyxid node transfer <node-id-or-name> --to <org-id>` when a node was registered under a person but should become shared org infrastructure. The caller must have write access to the current owner and be an admin of the destination org.

Transfer changes only server-side ownership. The node auth token is per-node, so the existing VM connection keeps working. Active `NodeServiceBinding` rows and pending credential pushes for the node are deactivated, and any `UserService` with `node_id` set to that node is unrouted unless it already belongs to the new owner. Recreate only the org-owned bindings and credential pushes you still want after the transfer.

### Setting up a new node

Registration must happen before installing the daemon. Credentials can be added before or after starting -- the agent reloads them automatically within 5 seconds.

```bash
# Step 1: Generate a registration token (on any machine with nyxid CLI).
# The wizard mints the nyx_nreg_… and shows it once with click-to-reveal.
# On a headless agent, append --no-wait --output json to get a pairing URL
# you can hand to the user and resume with `nyxid pairing resume <ID>`.
nyxid node register-token

# Step 2: Install nyxid CLI on the target machine
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"

# Step 3: Register the node (--keychain recommended for secure storage)
nyxid node register \
  --token "nyx_nreg_..." \
  --url "wss://<server>/api/v1/nodes/ws" \
  --keychain

# Step 4: Install and start as a background service (recommended)
nyxid node daemon install                              # install as system service
nyxid node daemon start                                # start the service

# Step 5 — CANONICAL for catalog services (llm-openai, llm-anthropic,
# api-github, etc.). One command: it auto-registers the UserService in the
# backend with this node's ID AND prompts for the credential locally.
# DO NOT also run `nyxid service add <slug> --via-node …` — that creates
# a duplicate backend record and the agent will get confused about which
# one to route through.
nyxid node credentials setup --service llm-openai      # agent picks up new credentials automatically

# Step 5 — for CUSTOM endpoints only (anything not in `nyxid catalog list`).
# Custom services have no catalog config, so the backend record must be
# created explicitly first; only then can the credential be stored locally.
nyxid service add --custom --via-node my-node           # creates backend record (prompts for URL, auth, etc.)
nyxid node credentials add --service my-api --header Authorization --secret-format bearer

# Or run in foreground (for debugging)
nyxid node start

# Or run via Docker. The CLI ships a `nyxid node docker` family that wraps
# the underlying `docker build` / `docker run` calls. It mounts the profile's
# config directory automatically and sets a profile-aware container name so
# multiple node instances can run side-by-side on one host.
nyxid node docker build                                # build the image once
nyxid node docker start                                # start container (default profile)
nyxid node docker start --profile edge-tokyo          # start a second profile
nyxid node docker status                               # show container state
nyxid node docker logs --follow                        # tail container logs
nyxid node docker stop                                 # stop and remove container
nyxid node docker restart                              # restart in place
```

> **Decision rule for AI agents** (deterministic — pick one path, don't mix):
>
> Two axes: (a) is the slug in the catalog? (b) is the operator on the SAME machine as the node, or on a different machine (e.g. admin laptop + shared VM)?
>
> | Same machine, catalog slug | → | `nyxid node credentials setup --service <slug>` (one command, auto-registers + stores credential) |
> | Same machine, custom endpoint | → | `nyxid service add --custom --via-node <node>` then `nyxid node credentials add --service <slug> …` |
> | Different machines (laptop + VM), catalog or custom | → | Admin on laptop: `nyxid service add <slug> [--org …] --via-node <node>` (catalog) or `nyxid service add --custom --via-node <node>` (custom). Operator on VM: `nyxid node credentials add --service <slug> …`. **Do NOT run `credentials setup` on the VM** — the service is already registered by the admin, so `setup` would duplicate state. |
>
> Auto-detect cheats:
> - Catalog vs custom: if `nyxid catalog show <slug> --output json` returns metadata, it's catalog. Otherwise custom.
> - Same-machine vs split: if you're on the same shell where the daemon is running, same-machine. If the user is describing a "shared / org-owned / company VM" or asks you to walk them through two machines, split.
> Credentials can be added, updated, or removed while the agent is running. The agent watches the config file and reloads credentials automatically (no restart needed). This works for both native daemons and Docker containers (config is mounted as a volume).
> Docker containers use the file backend (AES-GCM encrypted) -- OS keychain is not available in containers.
> The `nyxid node docker` wrapper accepts `--profile <name>` on every subcommand. Each profile becomes a separate container with its own config volume at `~/.nyxid-node/profiles/<name>/`, so you can run multiple node instances on the same host (one per org / one per environment).

### Managing the node service

```bash
# Background service lifecycle (launchd on macOS, systemd on Linux)
nyxid node daemon install                              # install as system service (auto-starts on login)
nyxid node daemon install --force                      # reinstall / update service config
nyxid node daemon start                                # start the service
nyxid node daemon stop                                 # stop the service
nyxid node daemon restart                              # restart (picks up config changes)
nyxid node daemon status                               # check if installed and running
nyxid node daemon logs                                 # show recent logs (last 50 lines)
nyxid node daemon logs --follow                        # tail logs in real time
nyxid node daemon uninstall                             # remove service (stops first)
```

### Managing nodes

```bash
# nyxid CLI (manage nodes from user side)
nyxid node list --output json                          # list nodes (includes IDs)
nyxid node show <ID_OR_NAME> --output json             # show node details + metrics

# register-token + rotate-token are wizard-capable: default to the bare form
# (auto-picks local-browser vs remote-pairing); use --no-wait --output json
# only when the agent specifically can't hold a subprocess open.
nyxid node register-token                              # opens browser wizard (v3.1)
nyxid node register-token --org <ID|SLUG|NAME>         # org-owned node token (admin only); wizard pre-selects org
nyxid node register-token --name "edge-tokyo" --no-wait --output json   # headless: pairing handoff JSON
nyxid pairing resume <pairing_id>                                       # …then resume once the user finishes
nyxid node rotate-token <ID_OR_NAME>                   # opens browser wizard (shows new auth_token + signing_secret)
nyxid node rotate-token <ID_OR_NAME> --no-wait --output json            # headless: pairing handoff JSON
nyxid pairing resume <pairing_id>                                       # …then resume once the user finishes

nyxid node delete <ID_OR_NAME> --yes                   # delete node
nyxid node transfer <ID_OR_NAME> --to <USER_OR_ORG_ID> # move node ownership, detaches cross-owner routing
nyxid node-credential push <ID_OR_NAME> --slug <SLUG> --injection-method header --field-name X-API-Key --output json
nyxid node-credential list <ID_OR_NAME> --output json  # list pending credential pushes
nyxid node-credential cancel <ID_OR_NAME> <PENDING_ID> # cancel a pending credential push

# nyxid node CLI (run on the node machine)
nyxid node credentials setup --service <SLUG>          # auto-detect and setup (recommended)
nyxid node credentials add --service <SLUG> --header Authorization --secret-format bearer
nyxid node credentials add-oauth --service <SLUG> --from-catalog  # OAuth from node
nyxid node credentials pending                         # list pushed credentials awaiting local acceptance
nyxid node credentials accept <SLUG>                   # enter and store the secret locally, then mark consumed
nyxid node credentials decline <SLUG> --reason "..."   # refuse a pending credential push
nyxid node credentials list                            # list configured credentials
nyxid node credentials remove --service <SLUG>         # remove credential

# SSH node-key credentials (stored only on the node)
nyxid node ssh-credentials add --service <SLUG> --principal <USER> --key-file ~/.ssh/id_ed25519 --host <HOST> --port 22

# Encrypted private key: pass the passphrase via env var (never as a flag value)
PRIVKEY_PASS=... nyxid node ssh-credentials add --service <SLUG> --principal <USER> \
  --key-file ~/.ssh/id_ed25519 --host <HOST> --port 22 --passphrase-env PRIVKEY_PASS

# Skip host-key pinning (TOFU off) -- only when the operator cannot capture the
# fingerprint upfront. Default is to pin; pinning prevents 1012 SshHostKeyMismatch
# regressions when targets rotate keys legitimately, but a missing pin trades that
# safety for first-connect convenience.
nyxid node ssh-credentials add --service <SLUG> --principal <USER> \
  --key-file ~/.ssh/id_ed25519 --host <HOST> --port 22 --no-pin-host-key

# Per-credential SSH algorithm allowlist (for legacy appliances like RouterOS,
# old MikroTik, or OOB management gear that only offer a narrow algorithm set).
# When unset, russh defaults are used. When set, only the listed algorithms are
# proposed for that credential. Each flag accepts a comma-separated list or
# can be repeated. `none` is rejected for kex/cipher/mac; only Ed25519, RSA,
# and ECDSA host-key families are accepted.
nyxid node ssh-credentials add --service routeros --principal admin \
  --key-file ~/.ssh/routeros_admin --host 10.0.0.1 --port 22 \
  --kex diffie-hellman-group-exchange-sha256 \
  --host-key rsa-sha2-256,ssh-rsa \
  --cipher aes256-ctr \
  --mac hmac-sha2-256

# Update or reset the allowlist on an existing credential without re-importing
# the key. `--reset-all` clears every category back to russh defaults; the
# per-category `--reset-<kex|host-key|cipher|mac>` flags clear one at a time.
# Setting a list and resetting the same category in one call is rejected.
nyxid node ssh-credentials set-algos --service routeros --principal admin \
  --cipher aes256-ctr,aes192-ctr
nyxid node ssh-credentials set-algos --service routeros --principal admin --reset-all

nyxid node ssh-credentials list --service <SLUG>
nyxid node ssh-credentials show --service <SLUG> --principal <USER>
nyxid node ssh-credentials test --service <SLUG> --principal <USER>
nyxid node ssh-credentials remove --service <SLUG> --principal <USER>
nyxid node ssh-credentials prune --stale  # all node `--profile <name>` flags also work here
```

> `credentials setup` works for **catalog services**: it auto-detects whether the service needs an API key, OAuth, or gateway URL, guides the user through the right flow, and auto-registers the service in the backend with the node's ID. For **custom endpoints**, use `nyxid service add --custom --via-node <node>` first, then `nyxid node credentials add`.

## SSH Remote Access

All SSH commands accept service ID, slug, or name (auto-resolves). SSH slugs are scoped per-user -- two users can each have an SSH service with the same slug without conflict. MCP SSH tools (`ssh_exec`, `ssh_list`) only see the caller's own services.

```bash
# Personal SSH service
nyxid service add-ssh --label prod-bastion --host 10.0.0.5 --via-node my-laptop \
  --cert-auth --principals ubuntu

# Shared with an org (every member sees this SSH service in their `service list`)
nyxid service add-ssh --label prod-bastion --host 10.0.0.5 --via-node office-node \
  --cert-auth --principals ubuntu \
  --org acme-corp

nyxid ssh exec <SERVICE> --principal ubuntu -- uptime
nyxid ssh exec <SERVICE> --principal ubuntu -- ls -la /var/log
nyxid ssh terminal <SERVICE>                           # auto-resolves principal
nyxid ssh terminal <SERVICE> --principal ubuntu
nyxid ssh issue-cert <SERVICE> --public-key-file ~/.ssh/id_ed25519.pub --principal ubuntu --certificate-file ~/.ssh/id_ed25519-cert.pub
nyxid ssh proxy <SERVICE>                              # ProxyCommand for OpenSSH

# List SSH services
nyxid service list --output json | jq '.keys[] | select(.service_type == "ssh")'

# Show one SSH service (handy when you need its UserService ID for `--via-service` later)
nyxid service show <SERVICE_ID_OR_SLUG> --output json
```

The SSH `--org` behavior matches `nyxid service add --org`: the service is created under the org owner, and members discover it through their own account. See [`organizations.md`](organizations.md#sharing-a-service-with-the-org) for org-scoped service ownership details.

### SSH auth modes

SSH services have an `ssh_auth_mode`:

- `cert`: NyxID issues short-lived SSH certificates. Supports `ssh proxy`, `ssh exec`, browser terminal, and `ssh issue-cert`.
- `node_key`: The node agent authenticates to the target using a node-local private key. Supports `ssh exec` and browser terminal. `ssh proxy` is intentionally unsupported.
- `proxy_only`: NyxID provides only the SSH-over-WebSocket transport.

Create a node-key service:

```bash
nyxid service add-ssh --label routeros --host 10.0.0.1 --port 22 --via-node edge-node --node-key --principals nyxid-ro,nyxid-admin
nyxid node ssh-credentials add --service routeros --principal nyxid-ro --key-file ~/.ssh/routeros_ro --host 10.0.0.1 --port 22
```

One service can have multiple node-local credentials keyed by `(service_slug, principal)`. If exactly one principal is registered locally, `nyxid ssh exec <SERVICE> -- <cmd>` selects it automatically. If two or more are registered, pass `--principal`; otherwise the CLI returns `1014 SshPrincipalAmbiguous`. If the selected node-local credential is missing, the backend returns `1011 SshNodeKeyMissing`.

Convert an existing SSH service:

```bash
nyxid service convert-ssh routeros --to-node-key
nyxid service convert-ssh routeros --to-cert
nyxid service convert-ssh routeros --to-proxy-only
```

After converting away from `node_key`, run `nyxid node ssh-credentials prune --stale` on the node to remove orphaned local SSH keys.

SSH error codes (1011-1015 are reserved for SSH; surface the code and the suggested fix to the user):

| Code | Name | What it means | What to do |
|------|------|---------------|------------|
| 1011 | `SshNodeKeyMissing` | Service is `node_key` but no credential exists for `(service_slug, principal)` on the node | Run `nyxid node ssh-credentials add --service <SLUG> --principal <USER> ...` |
| 1012 | `SshHostKeyMismatch` | Pinned host-key sha256 doesn't match what the target presented | Investigate first (could be a real MITM); if the target legitimately rotated keys, `remove` and re-`add` the credential |
| 1013 | `SshNodeExecChannelClosed` | russh channel error (auth, network, kex, or invalid algorithm config) | Check target reachability and the principal's authorized_keys. For "Key exchange init failed" against legacy appliances (RouterOS, old MikroTik), pin the algorithms with `ssh-credentials add --kex/--host-key/--cipher/--mac` or `set-algos`. Capture `RUST_LOG=russh=trace` if needed. |
| 1014 | `SshPrincipalAmbiguous` | Multiple principals registered locally and `--principal` was not passed | Pass `--principal <USER>` explicitly |
| 1015 | `SshAuthModeUnsupportedForOperation` | e.g. `ssh proxy` against a `node_key` service, or `ssh exec` against `proxy_only` | Use `ssh exec` / browser terminal for `node_key`; convert with `service convert-ssh` if the service should support a different operation set |
