# SSH Node-Key Auth Mode

NyxID SSH services support three auth modes:

| Mode | Value | Supported operations |
|---|---|---|
| Certificate | `cert` | `ssh proxy`, `ssh exec`, browser terminal, `ssh issue-cert` |
| Node key | `node_key` | `ssh exec`, browser terminal |
| Proxy only | `proxy_only` | `ssh proxy` transport only |

`node_key` keeps the target SSH private key on the node host. The NyxID server forwards only the service slug, SSH principal, command or shell request, and signed routing metadata over the node WebSocket. The node agent looks up a local encrypted entry keyed by `(service_slug, principal)` and authenticates to the downstream SSH target with `russh`.

## Breaking Change for Node Operators

This feature branch widens the HMAC envelopes for cert-mode `ssh exec` and SSH browser terminal open frames. After deploying a backend that includes this change, node operators must upgrade the node binary before cert-mode `ssh exec` and SSH web terminal sessions will work:

```bash
nyxid node daemon stop
# reinstall or replace the nyxid binary with the matching release
nyxid node daemon start
```

`ssh proxy`, HTTP and WebSocket proxying, node-key SSH exec (`ssh_node_exec_open`), and node heartbeat frames are unaffected by this HMAC cutover. SSH browser terminals use the widened `web_terminal_open` HMAC envelope in every auth mode.

## Create a Node-Key SSH Service

Create or convert the service from a logged-in operator/admin machine:

```bash
nyxid service add-ssh routeros \
  --host 10.0.0.1 \
  --port 22 \
  --via-node edge-node \
  --node-key \
  --principals nyxid-ro,nyxid-admin
```

Then provision credentials on the node machine:

```bash
nyxid node ssh-credentials add \
  --service routeros \
  --principal nyxid-ro \
  --key-file ~/.ssh/routeros_ro \
  --host 10.0.0.1 \
  --port 22
```

One service can hold multiple principals with separate private keys:

```bash
nyxid node ssh-credentials add --service routeros --principal nyxid-admin --key-file ~/.ssh/routeros_admin --host 10.0.0.1 --port 22
nyxid node ssh-credentials list --service routeros
```

The node config stores only non-secret metadata in a flat `ssh_keys` list. Private keys and optional passphrases are stored through the node secret backend using per-entry secret names. Existing API credential entries are unchanged; operators do not need to re-register nodes or re-enter existing node credentials.

## Execute Commands

When a service has one local principal, the CLI can infer it:

```bash
nyxid ssh exec routeros -- /system/resource/print
```

When two or more principals are registered for the same service, select one explicitly:

```bash
nyxid ssh exec routeros --principal nyxid-ro -- /system/resource/print
```

If no local credential exists for the selected principal, the backend returns `1011 SshNodeKeyMissing`. If more than one local principal exists and the caller omits `--principal`, the CLI returns `1014 SshPrincipalAmbiguous` and lists the available principals.

`nyxid ssh proxy` is intentionally unsupported for node-key services and returns `1015 SshAuthModeUnsupportedForOperation`. Use `nyxid ssh exec` or the browser terminal for node-key services.

## Convert Modes

Use the conversion helper for existing SSH services:

```bash
nyxid service convert-ssh routeros --to-node-key
nyxid service convert-ssh routeros --to-cert
nyxid service convert-ssh routeros --to-proxy-only
```

Converting to `node_key` does not check whether a node-local key has already been provisioned because the target node may be offline. `ssh exec` returns `1011 SshNodeKeyMissing` until an operator provisions at least one credential with `nyxid node ssh-credentials add`.

Converting away from `node_key` leaves local node-key entries in the node store. They are marked stale by server-side state and can be removed on the node with:

```bash
nyxid node ssh-credentials prune --stale
```

Every conversion emits audit event `service.ssh_auth_mode_changed` with `{from, to, actor}`.

## Host-Key Pinning

By default, `nyxid node ssh-credentials add` scans the target SSH host key and stores its SHA-256 fingerprint. The node agent verifies that fingerprint before authenticating and returns `1012 SshHostKeyMismatch` if the target presents a different host key.

Use `--no-pin-host-key` only when the operator accepts the risk of trusting any host key for that entry:

```bash
nyxid node ssh-credentials add --service routeros --principal nyxid-ro --key-file ~/.ssh/routeros_ro --host 10.0.0.1 --no-pin-host-key
```

## Per-Credential Algorithm Allowlist

Each credential can pin its own ordered KEX, host-key, cipher, and MAC allowlists. When unset, russh defaults are used; when set, only the listed algorithms are proposed for that credential's connections. The lists apply to `ssh exec`, browser terminal, and the `ssh-credentials test` no-op handshake.

Use this when russh's default negotiation does not converge against a legacy appliance (e.g. MikroTik RouterOS strong-crypto, older OOB management cards). Pinning gives a deterministic set so `1013 SshNodeExecChannelClosed` is no longer ambiguous between "no algorithm overlap" and "auth or network failure".

```bash
nyxid node ssh-credentials add \
  --service routeros --principal nyxid-admin \
  --key-file ~/.ssh/routeros_admin --host 10.0.0.1 --port 22 \
  --kex diffie-hellman-group-exchange-sha256 \
  --host-key rsa-sha2-256,ssh-rsa \
  --cipher aes256-ctr \
  --mac hmac-sha2-256
```

Update or reset an existing credential without re-importing the key:

```bash
# Replace just the cipher list
nyxid node ssh-credentials set-algos --service routeros --principal nyxid-admin \
  --cipher aes256-ctr,aes192-ctr

# Drop one category back to russh defaults
nyxid node ssh-credentials set-algos --service routeros --principal nyxid-admin --reset-mac

# Drop everything back to russh defaults
nyxid node ssh-credentials set-algos --service routeros --principal nyxid-admin --reset-all
```

Validation rules enforced at write time and at connect time:

- Each flag accepts comma-separated values or can be repeated.
- Empty lists are rejected — omit the flag instead.
- `none` is rejected for kex/cipher/mac (cleartext transport is not configurable).
- Host-key entries must resolve to the Ed25519, RSA, or ECDSA family. DSA and security-key (`sk-*`) algorithms are rejected.
- The OpenSSH `kex-strict-c-v00@openssh.com` and `ext-info-c` markers are auto-appended after resolution; an operator cannot disable them via the allowlist and cannot pass them as input.
- The pinned host-key SHA-256 fingerprint (set by `--no-pin-host-key`'s default) is enforced independently of the algorithm list, so a weakened algorithm allowlist does not weaken MITM protection.

The `show` subcommand prints the current allowlist per category, or `default` when unset:

```bash
nyxid node ssh-credentials show --service routeros --principal nyxid-admin
```

## Node WebSocket Frames

Node-key command execution uses:

- `ssh_node_exec_open` from NyxID to node with `request_id`, `service_slug`, `principal`, `command`, and `timeout_secs`.
- `ssh_node_exec_data` from node to NyxID for stdout/stderr chunks.
- `ssh_node_exec_close` from node to NyxID for normal completion.
- `ssh_node_exec_error` from node to NyxID for missing keys, host-key mismatch, or SSH channel errors.

The browser terminal reuses `web_terminal_open` with `auth_mode: "node_key"` and the same `web_terminal_data`, `web_terminal_resize`, and `web_terminal_closed` stream frames as cert-mode terminals.
