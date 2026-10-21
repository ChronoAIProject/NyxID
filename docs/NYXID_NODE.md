# nyxid node Agent

> The node agent is part of the `nyxid` CLI. All commands below use `nyxid node <command>`. The former standalone `nyxid-node` binary has been removed.

The `nyxid node` subcommand is a lightweight credential node agent built into the `nyxid` CLI. It runs on your infrastructure, connects to a NyxID server via WebSocket, receives proxy requests, injects locally stored credentials, and forwards requests to downstream services. Credentials never leave your infrastructure.

---

## Table of Contents

- [Installation](#installation)
- [Registration](#registration)
- [Starting the Agent](#starting-the-agent)
- [Managing Credentials](#managing-credentials)
- [Checking Status](#checking-status)
- [Secret Storage Backends](#secret-storage-backends)
- [Migrating Storage Backends](#migrating-storage-backends)
- [Configuration File](#configuration-file)
- [HMAC Request Signing](#hmac-request-signing)
- [Streaming Proxy Responses](#streaming-proxy-responses)
- [Serving SSH Tunnels](#serving-ssh-tunnels)
- [Reconnection and Resilience](#reconnection-and-resilience)
- [Graceful Shutdown](#graceful-shutdown)
- [Security](#security)
- [CLI Reference](#cli-reference)
- [Troubleshooting](#troubleshooting)

---

## Installation

The node agent is included in the `nyxid` CLI. Build from source (requires Rust 2024 edition):

```bash
# From the project root
cargo build --release -p nyxid-cli

# Binary is at target/release/nyxid
```

Or install directly:

```bash
cargo install --path cli
```

---

## Registration

Before starting the agent, register it with your NyxID server using a one-time registration token.

### Step 1: Create a Registration Token

In the NyxID dashboard, go to **Credential Nodes** and click **Register Node**. Or use the API:

```bash
curl -X POST https://your-nyxid-server/api/v1/nodes/register-token \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "my-server"}'
```

The response includes a `nyx_nreg_...` token that expires after 1 hour (configurable).

### Step 2: Register the Agent

```bash
nyxid node register --token nyx_nreg_<64_hex_chars>
```

The agent connects to the NyxID server via WebSocket, exchanges the registration token for a permanent auth token and HMAC signing secret, and saves the encrypted configuration to `~/.nyxid-node/config.toml`.

#### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--token` | (required) | One-time registration token |
| `--url` | `ws://localhost:3001/api/v1/nodes/ws` | WebSocket URL of the NyxID server |
| `--config` | `~/.nyxid-node` | Path to config directory |
| `--keychain` | `false` | Store secrets in the OS keychain instead of encrypted file |

For production, use WSS:

```bash
nyxid node register \
  --token nyx_nreg_... \
  --url wss://auth.example.com/api/v1/nodes/ws
```

To use the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service) instead of file-based encryption:

```bash
nyxid node register \
  --token nyx_nreg_... \
  --url wss://auth.example.com/api/v1/nodes/ws \
  --keychain
```

On success, the agent prints the node ID, storage backend, and config file path:

```
Node registered successfully.
  Node ID:  a1b2c3d4-...
  Storage:  file
  Config:   /home/user/.nyxid-node/config.toml

Start the agent with:
  nyxid node start
```

---

## Starting the Agent

```bash
nyxid node start
```

The agent:

1. Loads the configuration from `~/.nyxid-node/config.toml`
2. Loads the auth token and signing secret from the configured storage backend (file or OS keychain)
3. Loads all stored credentials from the configured backend
4. Connects to the NyxID server via WebSocket
5. Authenticates with its node ID and auth token
6. Begins serving proxy requests, SSH tunnel requests, and responding to heartbeats

#### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--config` | `~/.nyxid-node` | Path to config directory |
| `--log-level` | `info` | Log level: `trace`, `debug`, `info`, `warn`, `error` |

The agent runs until terminated. Use `--log-level debug` for detailed connection and request logging.

---

## Managing Credentials

Credentials are stored locally using the configured storage backend -- either AES-256-GCM encrypted in the config file (default) or in the OS keychain. The agent loads them at startup and holds decrypted values in memory.

### Preferred: Setup from the NyxID Catalog

For catalog-backed services, the easiest path is:

```bash
nyxid node credentials setup --service llm-openai
```

`credentials setup` fetches the service metadata from NyxID, detects whether the service needs an API key, bearer token, gateway URL, device-code OAuth, or authorization-code OAuth, and stores the resulting credential locally on the node.

For node-routed provider-backed services, the OAuth token stays on the node. NyxID stores only the routed AI service metadata and does not keep the provider credential.

### Add a Credential (Header Injection)

```bash
nyxid node credentials add \
  --service openai \
  --header "Authorization: Bearer sk-proj-..."
```

### Add a Credential (Query Parameter Injection)

```bash
nyxid node credentials add \
  --service stripe \
  --query-param "api_key=sk_live_..."
```

### List Credentials

```bash
nyxid node credentials list
```

Output:

```
Configured credentials:
  openai: header (Authorization)
  stripe: query_param (api_key)
```

### Remove a Credential

```bash
nyxid node credentials remove --service openai
```

### Service Slug Matching

The `--service` value must match the **slug** of the downstream service in NyxID. When a proxy request arrives for a service, the agent looks up credentials by the service slug included in the request.

---

## Checking Status

```bash
nyxid node status
```

Output:

```
Node Status
  Node ID:     a1b2c3d4-...
  Server:      wss://auth.example.com/api/v1/nodes/ws
  Storage:     file
  Credentials: 2 configured
    - openai
    - stripe
```

This is a local check only -- it reads the config file but does not connect to the server.

---

## Secret Storage Backends

The agent supports two backends for storing secrets (auth token, signing secret, credential values):

### File Backend (default)

Secrets are encrypted with AES-256-GCM and stored in `config.toml`. A 32-byte encryption key is generated at `~/.nyxid-node/.keyfile` (mode `0600`). This works on all platforms including headless servers and Docker containers.

```bash
nyxid node register --token nyx_nreg_...
```

### Keychain Backend

Secrets are stored in the OS keychain:

- **macOS**: Keychain (via Security Framework)
- **Windows**: Credential Manager
- **Linux**: Secret Service D-Bus API (GNOME Keyring, KDE Wallet)

The TOML config file retains only non-secret metadata (server URL, node ID, injection method, header/param names). No encrypted values are written to disk.

```bash
nyxid node register --token nyx_nreg_... --keychain
```

Keychain entries use `nyxid-node` as the service name, with `{node_id}/auth_token`, `{node_id}/signing_secret`, and `{node_id}/cred/{service_slug}` as account identifiers. Multiple nodes on the same machine do not collide.

For an interactive desktop installation, select `--keychain` explicitly and use
that supervisor user's native service: a macOS LaunchAgent in the Aqua login
session or a Linux systemd user unit connected to its own `%t/bus`, ordered after
`dbus.socket`. An unlocked desktop key service must be available to that user.
The supervisor owns the key-service session; agent and browser jobs never inherit
its bus address or credential environment.

On headless Linux and separated root-supervisor machines, the default file backend
works without a desktop key service. Keep the supervisor's config directory private
and the worker identities distinct. A file-backed node does not contact Secret
Service just to prove file custody. A locked or unavailable configured keychain
refuses secret access; the daemon never silently switches storage, creates a new
key, or treats a missing object as an access denial. To change an existing backend,
use the explicit stopped-daemon migration below.

Existing installed service definitions keep working. To opt into the new native
key-service session binding, explicitly reinstall after upgrading (with the same
`--profile` and `--config`):

```bash
nyxid node daemon install --force --profile work
nyxid node daemon restart --profile work
```

Only install/reinstall renders the new definition. Start does not rewrite it, and
migration accepts both the original and the new canonical definitions.

### Credential custody and worker identities

A per-user keychain does not isolate programs sharing its OS identity. The trusted
credential supervisor must remain outside the command, secure-browser and developer-
browser identities. On supported separated Linux nodes, the node makes real opens
of its config and keyfile and the supported sibling profiles' keyfiles under each
worker UID/GID and its selected supplementary groups. Keychain-backed nodes also
create a public synthetic entry in the supervisor's actual OS key service, prove
that it exists there, and attempt retrieval from every worker identity. Probe
entries use `{node_id}/object/v1/custody/*`; they contain no node secret and are
retired after the observation. The helper never reads actual credential values.

File custody checks the complete supplied path before resolving aliases: every
directory traversed, including parents inside intermediate symlink targets, must
belong to root or the supervisor UID and disallow group/other writes unless
sticky. The selected node directory must belong to the supervisor and disallow
other writers; config/key files must be private regular files. Trusted platform
aliases such as macOS `/var` and `/tmp` remain supported. A private destination
does not make an alias under a writable non-sticky parent safe. Custody retains
the same path spelling as later config readers and revalidates that traversal;
canonicalizing only the custody path would hide those parents. Root and processes
sharing the supervisor UID remain trusted and can change accepted paths.

Only an expected OS permission error establishes denial. Successful access is
reported as accessible; missing paths/objects, unavailable services, unfamiliar
provider errors, invalid identities and timeouts remain unknown. Diagnostics contain
fixed reasons and per-role metadata, never provider bodies or secret identifiers.
The cache is separate for each context's actual identities: initial/context probes
and identity changes refresh it, and a bounded monitor repeats every 60 seconds.
Execution reads the cache; it does not launch a probe on each request. Expired
observations cannot authorize separated execution. Machine status includes
`credential_custody` outcomes for command, browser and developer roles; stale
snapshots are unknown until the next daemon profile publication. The CLI helper
executable must also be reachable by every worker identity: a binary installed
inside a supervisor-only home cannot establish custody. Install it at the shared,
administrator-controlled executable location used by the separated setup.

Shared legacy and non-separated machines retain their existing execution behavior
with a downgraded custody claim and an explicit warning that agent commands or
browsers may access node tokens, signing secrets and stored credentials. A separated
context instead refuses unverified custody with `node_custody_unverified` (12419)
before a worker or browser effect. Signed job cancellation, turn stops and desktop
close remain available when custody becomes unverified, with their existing
signature, authority and scope checks. This is OS identity separation, with the shared
kernel, supervisor and privileged-user limits described in the machine docs; it is
not VM isolation. Independently configured profiles outside the supported default
or machine-profile roots remain the operator's responsibility.

Custody adds a prerequisite to separation evidence v4. Upgrade both backends and
nodes before separated mode becomes available again: old backends refuse v4 node
evidence, and new backends refuse v3 node evidence. Missing/null/false custody never
becomes a successful observation. Existing `shared_legacy` assignments retain their
stored mode; a failed separated request never falls back to shared legacy.

After the backend upgrade, existing separated assignments pause until their nodes
report supported v4 evidence. Their stored mode and capabilities stay unchanged;
discovery and settings retain them with an unavailable reason. Upgrade and drain
every backend replica before relying on this admission gate: older replicas may
still trust the old support advertisement for an existing separated assignment.

---

## Migrating Storage Backends

To migrate an existing node from file-based storage to OS keychain (or vice versa):

```bash
# Migrate from file to keychain
nyxid node migrate --to keychain

# Migrate from keychain back to file
nyxid node migrate --to file
```

The `migrate` command:

1. Reads all secrets (auth token, signing secret, all credential values) from the current backend
2. Writes them to the target backend
3. Updates `storage_backend` in the config file
4. Saves the updated config
5. Removes the old secrets from the previous backend

After migration, restart the agent to use the new backend. If saving the updated config fails, the agent keeps using the source backend and does not delete the source secrets. If cleanup of the previous backend fails after the save succeeds, the migration still completes and prints warnings so you can remove the stale secrets manually.

---

## Configuration File

The agent stores its configuration at `~/.nyxid-node/config.toml` (or the path specified by `--config`). The file is created during registration and updated when credentials are added or removed.

### Structure

#### File Backend

```toml
storage_backend = "file"

[server]
url = "wss://auth.example.com/api/v1/nodes/ws"

[node]
id = "a1b2c3d4-..."
auth_token_encrypted = "<base64>"

[signing]
shared_secret_encrypted = "<base64>"

[credentials.openai]
injection_method = "header"
header_name = "Authorization"
header_value_encrypted = "<base64>"

[credentials.stripe]
injection_method = "query_param"
param_name = "api_key"
param_value_encrypted = "<base64>"
```

#### Keychain Backend

When using the keychain backend, encrypted values are omitted from the config. Only non-secret metadata is stored:

```toml
storage_backend = "keychain"

[server]
url = "wss://auth.example.com/api/v1/nodes/ws"

[node]
id = "a1b2c3d4-..."
auth_token_encrypted = ""

[signing]
shared_secret_encrypted = ""

[credentials.openai]
injection_method = "header"
header_name = "Authorization"

[credentials.stripe]
injection_method = "query_param"
param_name = "api_key"
```

> **Backwards compatibility:** Existing config files without a `storage_backend` field default to `"file"`.

### File-Backend Encryption

When using the file backend, all sensitive values are encrypted with AES-256-GCM using a locally generated 32-byte key stored at `~/.nyxid-node/.keyfile`. The keyfile is created with mode `0600` on Unix systems.

Each encrypted value is stored as base64-encoded `nonce (12 bytes) || ciphertext`. Different nonces are used for each encryption operation, so the same plaintext produces different ciphertext.

### File Permissions

On Unix systems, the config file is written atomically (write to temp file with mode `0600`, then rename) to avoid a window where the file has default permissions.

---

## HMAC Request Signing

When HMAC signing is enabled on the NyxID server (default: enabled), proxy requests sent to the agent include an HMAC-SHA256 signature. The agent verifies this signature to ensure request integrity and authenticity.

### How It Works

1. During registration, the server generates a shared HMAC secret and returns it to the agent
2. The server signs each proxy request with the shared secret
3. The agent verifies the signature before executing the request
4. Requests with invalid signatures are rejected with HTTP 403

### Signed Fields

The HMAC message is computed as:

```
{timestamp}\n{nonce}\n{method}\n{path}\n{query}\n{body_base64}
```

The signature is a hex-encoded HMAC-SHA256 digest.

### Replay Protection

The agent maintains a replay guard that:

- Rejects requests with timestamps older than 5 minutes (`MAX_TIMESTAMP_SKEW_SECS = 300`)
- Rejects duplicate nonces within the skew window
- Caps the nonce set at 10,000 entries to bound memory usage

---

## Streaming Proxy Responses

The agent supports streaming proxy responses for SSE (Server-Sent Events) endpoints. When the downstream service returns `Content-Type: text/event-stream`, the agent streams the response back to NyxID in chunks instead of buffering the entire response.

### Streaming Protocol

1. The agent sends `proxy_response_start` with status and headers
2. The agent sends streaming data chunks, preferably as WebSocket binary frames with a 36-byte `request_id` prefix; when the server does not advertise `auth_ok.capabilities.proxy_binary_chunks`, it falls back to legacy `proxy_response_chunk` JSON messages with base64-encoded data
3. The agent sends `proxy_response_end` when the stream completes

NyxID reconstructs the streaming response and forwards it to the client as a standard SSE stream. This enables real-time streaming from LLM APIs (e.g., OpenAI chat completions with `stream=true`) through the node proxy.

---

## Serving SSH Tunnels

The agent also participates in NyxID's SSH-over-WebSocket flow when a bound service has SSH tunneling enabled.

### SSH Tunnel Flow

1. NyxID sends `ssh_tunnel_open` with a `session_id`, `host`, and `port`
2. The agent opens a TCP connection to `host:port` from the node's network
3. The agent acknowledges success with `ssh_tunnel_opened`
4. SSH payload bytes move in both directions through `ssh_tunnel_data`
5. Either side ends the session with `ssh_tunnel_close` or `ssh_tunnel_closed`

### Operational Notes

- No SSH private keys are stored on the node for this feature; the node only bridges TCP
- The target SSH service must be reachable from the node host
- If the TCP connect attempt fails, the agent returns `ssh_tunnel_closed` with an error payload and NyxID records an SSH connect failure audit event

For end-user setup, certificate issuance, and OpenSSH `ProxyCommand` examples, see [SSH_TUNNELING.md](./SSH_TUNNELING.md).

---

## Reconnection and Resilience

The agent automatically reconnects on disconnection using exponential backoff:

| Attempt | Jittered delay |
|---------|----------------|
| 1 | 1–2 seconds |
| 2 | 2–4 seconds |
| 3 | 4–8 seconds |
| 4 | 8–16 seconds |
| 5 | 16–32 seconds |
| Later | 30–60 seconds |

Every disconnect triggers a delayed retry, including clean server closes and
errors. The backoff resets only after at least 60 seconds of authenticated
service; a briefly successful handshake does not reset it. Shutdown interrupts
the retry sleep immediately.

After a backend crash/restart, the previous generation's owner lease can remain
live for up to `NODE_OWNER_LEASE_TTL_SECS` (default 90s). The server rejects such
attempts with WebSocket Close `4008` before `auth_ok`; the agent retries with
backoff until ownership is available. The node auth token is long-lived and
is not a JWT; an unrelated `Token expired` JWT log does not diagnose this lease
rejection.

The agent handles the full reconnection lifecycle:

1. Establish WebSocket connection
2. Send `auth` message with stored node ID and auth token
3. Wait for `auth_ok` response
4. Set up writer task for outgoing messages
5. Enter the main reader loop for heartbeats and proxy requests

---

## Graceful Shutdown

The agent handles `SIGINT` (Ctrl+C) and `SIGTERM` gracefully:

1. Stop accepting new proxy requests
2. Wait up to 30 seconds for in-flight requests to complete
3. Force shutdown if requests remain after the deadline

In-flight requests are tracked with an atomic counter that increments when a request starts and decrements when it completes.

---

## Security

### Secret Storage

- **File backend:** All secrets are encrypted with AES-256-GCM before writing to disk. The encryption key is a 32-byte random value stored in `~/.nyxid-node/.keyfile`, created with `O_CREAT | O_EXCL` and mode `0600` (Unix) to prevent race conditions. Source byte arrays are zeroized after copying.
- **Keychain backend:** Secrets are stored in the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service). No encrypted values or keyfile are written to disk.
- Decrypted credential values are held in `Zeroizing<String>` wrappers regardless of backend

### Token Security

- Auth tokens (`nyx_nauth_...`) are 32 bytes of cryptographic randomness
- Tokens are encrypted at rest in the config file
- The NyxID server stores only SHA-256 hashes of tokens
- Token rotation invalidates the old token immediately

### Network Security

- Use `wss://` (WebSocket over TLS) in production
- Auth tokens are transmitted in WebSocket messages, not URL parameters
- HMAC signing prevents request tampering in transit

### Credential Isolation

- Credentials are stored only on the node -- they never transit the NyxID server
- The agent injects credentials into outgoing requests locally
- Header injection overwrites the specified header; query parameter injection appends to the URL

---

## CLI Reference

```
nyxid node <COMMAND> [OPTIONS]

COMMANDS:
  register      Register this node with a NyxID server
  start         Start the node agent (connect and serve)
  status        Show node connection status
  rekey         Update auth token and signing secret after server-side rotation
  credentials   Manage local credentials
  openclaw      OpenClaw convenience commands
  migrate       Migrate secret storage between backends
  version       Show version information

GLOBAL OPTIONS:
  --log-level <LEVEL>   Log level: trace, debug, info, warn, error

REGISTER OPTIONS:
  --token <TOKEN>       One-time registration token (nyx_nreg_...)
  --url <URL>           WebSocket URL of the NyxID server
  --config <PATH>       Path to config directory
  --keychain            Store secrets in OS keychain instead of encrypted file

START OPTIONS:
  --config <PATH>       Path to config directory

STATUS OPTIONS:
  --config <PATH>       Path to config directory

REKEY OPTIONS:
  --auth-token <TOKEN>      New auth token (nyx_nauth_...)
  --signing-secret <HEX>    New HMAC signing secret (64 hex chars)
  --config <PATH>           Path to config directory

CREDENTIALS SUBCOMMANDS:
  setup      Set up a catalog-backed service locally (preferred)
  add        Add a credential for a service
  add-oauth  Run a local OAuth flow for a service
  list       List configured credentials
  remove     Remove a credential for a service

CREDENTIALS SETUP OPTIONS:
  --service <SLUG>          Catalog service slug (e.g., "llm-openai")
  --api-url <URL>           NyxID base URL (auto-derived from node config when omitted)
  --access-token <TOKEN>    NyxID access token (or use `nyxid login` / NYXID_ACCESS_TOKEN)
  --config <PATH>           Path to config directory

CREDENTIALS ADD OPTIONS:
  --service <SLUG>          Service slug (e.g., "openai")
  --header <HEADER>         Header to inject (e.g., "Authorization: Bearer sk-...")
  --query-param <PARAM>     Query parameter to inject (e.g., "api_key=sk-...")
  --config <PATH>           Path to config directory

CREDENTIALS ADD-OAUTH OPTIONS:
  --service <SLUG>          Service slug
  --from-catalog            Fetch OAuth metadata from the NyxID catalog
  --client-id <ID>          OAuth client ID (or use catalog-provided shared client ID)
  --client-secret <SECRET>  OAuth client secret when required
  --authorization-url <URL> OAuth authorization URL for manual setup
  --token-url <URL>         OAuth token URL for manual setup
  --device-code-url <URL>   OAuth device-code URL for manual setup
  --config <PATH>           Path to config directory

OPENCLAW SUBCOMMANDS:
  connect     Store the OpenClaw credential locally and optionally create/check the routed AI service in NyxID
  status      Show local OpenClaw credential status
  disconnect  Remove the local OpenClaw credential

CREDENTIALS REMOVE OPTIONS:
  --service <SLUG>          Service slug to remove
  --config <PATH>           Path to config directory

MIGRATE OPTIONS:
  --to <BACKEND>            Target backend: "keychain" or "file"
  --config <PATH>           Path to config directory
```

---

## Troubleshooting

### "Config error: Failed to read config"

The config file does not exist. Run `nyxid node register` first.

### "Authentication failed" on start

The auth token may have been rotated. Re-register the node with a new registration token, or update the config with the new token after rotation.

### "No credentials configured for service"

The service slug in the proxy request does not match any entry in the local credential store. Add the credential with:

```bash
nyxid node credentials add --service <slug> --header "Authorization: Bearer ..."
```

### Agent keeps reconnecting

Check the logs for the specific error. Common causes:

- **"Failed to connect"**: The NyxID server is unreachable. Verify the `--url` or the `server.url` in config.toml.
- **"Authentication failed"**: The auth token is invalid or has been rotated.
- **"Connection closed during auth"**: The server rejected the connection (max connections reached, or the node was deleted).

### HMAC signature verification failed

The signing secret may be out of sync. Rotate the node's token from the NyxID dashboard to generate a new auth token and signing secret, then re-register.

### Streaming responses not working

Streaming is automatic when the downstream service returns `Content-Type: text/event-stream`. Verify the downstream service is configured correctly and the proxy request includes appropriate headers (e.g., `Accept: text/event-stream`).

## Machine access for NyxBot and specialists

Ask NyxBot to set up a machine. It returns an owner-only review page with one
copyable command and watches until the node connects. The page offers this
computer, a VM or Docker, independent command/file/computer capabilities, an
organization owner when you administer one, and an optional specialist grant.
Keep the command on that page: never paste its registration token into chat.
A VM or container is recommended. Agents act with their OS user's full access,
and content they encounter can contain prompt injection. Workspace roots
constrain file tools and command working directories; they do not sandbox a
shell command.

On a supported Linux host, install Chromium and the desktop prerequisites below.
Follow the [credential custody executable prerequisite](#credential-custody-and-worker-identities):
install `nyxid` in an administrator-controlled location every worker identity can
execute, such as `/usr/local/bin/nyxid`, rather than inside a supervisor-private
home. Then run:

```sh
sudo nyxid node setup --machine --computer --separate-users
```

Without a token this prints an expiring pairing code and link. Open the link
and explicitly confirm the hostname, OS, IP and capabilities, or tell NyxBot
only the short code and approve its action card. Setup registers the node,
installs the pinned computer driver when requested, then installs/starts the
profile's launchd or systemd daemon. `--profile NAME` isolates node identities.
`--root DIR` can be repeated; the default workspace is `~/nyxid-workspace`.
`--token` is supported for the page-generated command. Existing `register` and
`daemon install` commands still work.

New machine setups record `automatic` intent for new agent assignments. Setup
confirms Linux Landlock ABI 6 or newer, the root supervisor and three distinct
non-root users, the command seccomp filter, actual Landlock enforcement, and
the configured Chromium's namespace/PID/network/seccomp sandbox under both
browser users. A configured username or kernel version alone is insufficient.
The browser probe uses a fresh profile and the actual executable, including its
AppArmor attachment; Ubuntu's user-namespace restriction stays enabled.

Evidence v4 holds the launched package browser with a pidfd and observes live
sandboxed descendants through kernel PPid ancestry, all UID/GID values, NNP,
additional seccomp filters, deeper NSpid membership, different UID/GID maps and
a different namespace-specific net/dev inode. Process names and renderer-like
arguments never establish ancestry. These are observations, not attestation of
descendant executables or protection from browser/supervisor/kernel compromise.
The probe needs neither DevTools nor a tracing capability. A container hiding
required observations reports `separated_browser_sandbox_unobservable_in_container`
and `managed_browser_sandbox: false`; use supported native Linux or a separate
machine container/VM per agent instead of adding capabilities. It also
positively verifies the secure policy's refusal of remote debugging and absence
of a listener, and requires `credential_custody: true` from actual dropped-identity
access checks against protected credential storage. Missing or failed custody
returns `node_custody_unverified`; repair protected storage and rerun setup or
the probe. Container preflight uses disposable public sentinels without creating
node keys; registered native probes check actual node and sibling-profile
custody. v1/v2/v3 and unknown reports cannot authorize new automatic/required
assignments; existing stored assignments retain their mode.

Linux setup detects the resolved browser and its package metadata before
installing policies. Supported locations are Debian Chromium's
`/etc/chromium/policies/managed`, the actual Ubuntu Snap Chromium binary's
`/etc/chromium-browser/policies/managed`, and Google Chrome's
`/etc/opt/chrome/policies/managed`. Unknown executable/package layouts stop with
`managed_browser_policy_layout_unknown`; install a supported browser package
and rerun setup. Setup never installs a policy-directory alias. A policy file
alone is insufficient: `separated_browser_policy_not_effective` means the real
secure browser did not prove its policy blocks DevTools. Reinstall managed
policies for that executable and rerun; keep `RemoteDebuggingAllowed=false`.

The setup page carries the reviewed choice into native `--separation` arguments
or Docker's `NYXID_MACHINE_SEPARATION` environment variable. It preserves the
capabilities you selected; it does not silently enable a browser to make probes
pass. On a fresh Linux VM, install the managed browser and desktop prerequisites,
select Browser if you intend to grant it, and run the generated native setup
command with `sudo` and `--separate-users` to provision the required OS identities.
For example, an explicitly required browser/files setup is:

```sh
sudo nyxid node setup --files --browser --separate-users --separation require-separated
```

| Setup option | New-assignment intent |
|---|---|
| Omitted on a new setup, or `--separation automatic` | Choose separated only after every supported mechanism is confirmed; otherwise explain the reason and offer an informed legacy choice when stdin and stderr are terminals. |
| `--separation require-separated` | Require confirmed separation; never fall back to shared access. |
| `--separation shared-legacy` | Explicitly accept shared legacy access after reviewing the warning below. |

On macOS or another unsupported configuration, automatic setup prints the
observed reason and the full warning below, then asks
`Continue with shared legacy mode? [y/N]` when stdin **and** stderr are terminals.
Only `y` records an explicit `shared_legacy` owner choice. No, an empty answer
or EOF stops setup with a nonzero exit and retains automatic intent.
`require-separated` never prompts. Noninteractive automatic setup also stops;
its error names the reason and the exact `--separation shared-legacy` flag.
Known missing prerequisites are handled before pairing. To deliberately
use a single-user host in a script, run `nyxid node setup --machine --computer --separation
shared-legacy`. **Shared legacy access gives agent commands the full access of
their OS user. A misbehaving or prompt-injected agent may read node tokens,
signing secrets and stored credentials accessible to that user, including the
node config and local credential store. Workspace roots constrain file tools
and working directories, not the shell. Supported separated contexts still
share the host kernel and retain kernel-exploit and supervisor-compromise risks;
they are not a VM boundary.** Use a separate machine container or VM per agent
for stronger isolation; containers also share their host kernel.
Supported `separated` mode retains the residual
risks in [MACHINE_AGENT_ISOLATION.md](MACHINE_AGENT_ISOLATION.md).

Probes have a five-second bound. `separated_probe_timeout` means the host did not
confirm support in time; a cold Chromium start can cause this. Rerun setup when
the host is less busy. Setup kills probe helpers and reaps the parent after
success, failure or timeout. Failed automatic/required intent remains recorded
so a retry cannot silently become legacy consent, and setup does not start the
daemon after failed verification. Before registration, the private
`machine-separation-choice.json` file retains pending intent without creating
registration credentials. After registration, intent lives in `config.toml`.

Rerunning setup against an enabled older machine preserves its absent intent
unless `--separation` is supplied. Stored agent assignments always retain their
mode. A selection records intent for future assignments; owner authorization
and the existing feature gates still apply. `machine status` shows the recorded
intent and cached probe reason without launching a browser. Upgrade the server
to the W9.4 selection contract before relying on a node's advertised intent;
older servers do not enforce the new selection field.

After successful setup, `[machine].separation_choice` in the profile's
`config.toml` stores `automatic`, `require_separated` or `shared_legacy` (the CLI
uses hyphens). An older profile with no value stays unselected on an omitted-flag
rerun. Before relying on these defaults, upgrade and drain every server replica,
then deploy the matching UI and CLI/container versions. The server's durable
W9.4 marker preserves only pre-cutover implicit NyxBot reachability under live
ACLs; a newly added machine with no choice requires owner selection. Existing
legacy assignments continue normally, and node intent never rewrites them.

Assistant → Machines distinguishes confirmed support, unmet prerequisites and
unknown evidence. A new or old support boolean alone is insufficient for a new
separated assignment. Node settings say **Sharing set per agent** because one
node can have legacy and separated assignments at the same time. Review the
agent's **Shared legacy**, **Separate workspace and browser**, or **Owner
selection required** label for its effective mode. Saved-login fills still bind
to their approved context and profile generation; revocation quarantines the old
profile before reuse. These controls do not establish VM-level isolation or move
the credential service outside agent OS identities; see the [residuals and
mechanisms](MACHINE_AGENT_ISOLATION.md#what-separated-does-and-does-not-guarantee).

```sh
nyxid node machine enable                    # commands and files
nyxid node machine enable --computer         # independent computer capability
nyxid node machine enable --shell --files --root /srv/workspace
nyxid node machine disable --computer
nyxid node machine disable --all
nyxid node machine status
```

Capabilities default off and the local configuration is authoritative. Changes
apply on daemon restart or reconnect. Shell/computer processes may run as root
only with explicit `--allow-root`. Commands have a clean environment, no TTY,
a process group killed on timeout/cancel, four concurrent jobs by default, a
one-hour timeout ceiling and bounded output. Foreground results preserve the
beginning and end of large output with byte totals; background output paginates
by absolute offset from a bounded ring. Both share the configured memory cap.
Background jobs survive socket reconnects, but a daemon restart ends them.

Computer use runs the MIT cua driver **0.31.0** (`cua-driver-rs-v0.31.0`; authoritative pin: `cli/src/node/machine/cua.rs`), verified against the release
hashes in `cli/resources/cua/release.json`, through MCP stdio with telemetry
disabled. NyxID never installs the AGPL perception extension. `--cua-driver PATH`
uses an existing binary after checking its version. Standard mode may require
human consent for some actions. `--computer-mode unrestricted` is explicit on
hosts; it is the machine container default. On macOS, grant Screen Recording
and Accessibility for the app launching the node, then restart it. The direct
MCP driver's TCC attribution belongs to its launching app (for example Terminal).
`machine status` and node details report both grants; a successful tool listing
alone does not mean capture/input permissions are available. Captures that need
a temporary filename use a private 128 MiB RAM volume, removed when the driver
stops. If that volume cannot be created, computer use fails without writing a
frame to disk. Linux capture stays in memory.

### Container and separated Linux VM

The published image is
`ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:<server-version>`. The Assistant → Machines setup page
reads the server release from public config and provides the complete, pinned
`docker run`. The CLI `nyxid node docker … --machine` defaults to `latest`;
use the setup page command to pin the deployed server release. See
[container upgrades](MACHINE_NODES.md#rollout-and-container-upgrades) before updating. Persist both `/workspace` and `/var/lib/nyxid-machine`; without
`NYXID_NODE_TOKEN` the first boot prints a pairing code in its logs. It contains
Chromium, Xvfb, Openbox, git, curl, Python, Node.js, ripgrep and build tools.
The supervisor drops privileges: shell/file tools use `agent`, while Chromium
and cua use `browser`. The browser profile is private; X authentication is
unavailable to `agent`. Chromium retains its renderer sandbox through user and
PID namespaces. The generated Docker command supplies the shipped seccomp
profile with `--security-opt seccomp=...`, allowing those namespaces without
added capabilities. Do not add `--no-sandbox`; see the sandbox instructions
below for the equivalent manual command.

For the same OS-user separation on a Linux VM:

```sh
sudo nyxid node setup --machine --computer --separate-users --root /srv/workspace
```

Setup creates profile-specific users and a system supervisor service. The
supervisor running as root does not authorize commands as root: children drop
to the configured users. Per-agent separation additionally requires the live
capability checks above. Single-user setups require explicit `--separation
shared-legacy`; this flag does not remove browser/command OS-user separation
from an existing `--separate-users` installation.

### Connected services, files and desktop

Each command receives `NYXID_GATEWAY_URL` on loopback and a fresh, job-limited
`NYXID_GATEWAY_TOKEN`. Calls to `/s/{slug}/{path}` travel over the node socket
and run through NyxID's live proxy authorization, approval, billing and audit
pipeline. Declare the needed services in `nyx__machine_exec.services`; omitted
means none. SDK base/key variables point there only for those declarations.
For example, curl the GitHub API at `$NYXID_GATEWAY_URL/s/api-github/...` with
the local bearer token. No connected-service credential is sent to the machine.

After declaring the connected GitHub service, plain `git clone`, `fetch`, `pull`
and `push` use it
through smart HTTP. Per-process `GIT_CONFIG_*` rewrites HTTPS and `git@github.com:`
URLs and adds only the local token; global/repository configuration and remotes
retain the original URL. Git traffic streams with a route-specific 16 GiB
upload limit and a one-hour response-header allowance for receive-pack; stream
idle limits and the command's own timeout still apply. Set `timeout_secs` to a
suitable value for large clones/pushes (the command default is 120 seconds).
With no GitHub connection, git goes direct. An ungranted,
disabled, approval-required or expired service returns a NyxID error.

Conversation attachments can stream to the workspace. Verified PNG/JPEG/GIF/WebP
files up to 5 MiB can be shared back, at most eight images per turn. Computer
screenshots use the same owner-only attachment path, while the agent receives
accessibility text. Clipboard file/image inputs are staged from the agent's
readable workspace (5 MiB cap); cua cannot use them to open private browser
files. Screenshot output-file options are disabled.

The live desktop opens in NyxBot or from node details. Multiple owner viewers
can watch; one takes control. Taking control cancels agent jobs and blocks
computer, shell and file operations on that machine. Hand back with an optional
note to wake the waiting thread. Owner frames and input are never persisted or
returned to the model. Human capture uses X11 on Linux and ScreenCaptureKit on
macOS; Linux owner input uses XTest, independently of capture and agent cua
calls. macOS owner input uses a separate cua session. Idle sessions close.

### Recovering machine authority

Each new machine tool call automatically acquires live authority bound to the
connected daemon's current runtime and current permissions. Reading capabilities
or `machine_list` only observes state; it is **not an authority refresh**. Updating
or restarting changes the runtime, and an old lease must never be renewed into
that new runtime.

For repeated `machine_authority_stale` failures or pending permission changes:

1. Stop the affected turn if a command's outcome is ambiguous. Inspect activity
   and, when safe, use a read-only check to establish whether the command already
   changed anything. Never blindly replay an ambiguous command.
2. Update the node to a version with clock-safe authority timing, reconnect it,
   and confirm the expected machine/runtime in Machine activity and daemon logs.
   A browserless shared shell/files container needs no browser to recover.
3. Synchronize UTC on both server and node hosts. A Docker container shares its
   host's clock: fix the **Docker host clock**, rather than trying to set time
   inside the container. Authority timing permits at most five seconds of clock
   difference and fails closed beyond that bound.
4. Inspect the error's `reason` and `request_id` when present, plus Machine
   activity operation/activity/job IDs, runtime ID, assignment revision and UTC
   time. Refresh machine status. Distinguish earlier commands stopping from
   acknowledgement still pending and browser cleanup still pending. A displayed
   expiry time only bounds execution; cleanup requires actual acknowledgement.
   Offline acknowledgement may remain pending after earlier commands stop.
5. Make a **fresh harmless tool call** (for example a bounded file listing within
   allowed roots) after the required permission/cleanup state permits it. This
   call acquires fresh authority automatically. If it fails, retain its reason
   and request ID for diagnosis; do not reuse an old acknowledgement or lease.

Useful fixed diagnostics include `authority_clock_skew` (check host clocks),
`authority_lifetime_invalid` (check signing/issuance timing), `authority_expired`
(check delay/backpressure), `authority_runtime_mismatch` (check current daemon
connection/profile), `authority_revision_stale` (check current permissions),
`authority_binding_invalid`/`authority_context_mismatch` (check UUID/identity and
context bindings), `authority_store_unavailable` (check daemon-owned private
state directory, permissions and free disk), and `context_quarantine_pending`
(restore cleanup on the node). Invalid UUIDs, malformed identities, expired
source/funding authority and private-store failures can also refuse execution;
not every stale-authority error proves a runtime mismatch. A malformed store can
prevent daemon startup before any tool response. Check daemon logs rather than
repeatedly retrying commands. Capacity refusal is a separate busy outcome.

**Never delete `machine-authority-v2.json`, `machine-authority-v3.json` or context
journals to recover access.** Never lower revisions, manually clear database
pending/cleanup flags, bypass quarantine, or replay old leases. These files and
rows prevent old permissions and cancelled work from returning. Repair ownership,
permissions or disk availability without discarding their security state. Share
only metadata diagnostics; do not dump configuration, keys, command output or
credentials into support logs.

For pending rows without a deadline, the first committed observation starts a single conservative
47-second execution recovery window, initialized and checked against MongoDB's
clock rather than individual application replicas. A null/malformed recorded deadline requires
administrator inspection. Older nodes without timing support require an update
and real acknowledgement. See [authority timing and pending states](MACHINE_NODES.md#per-agent-capabilities-m12)
for protocol details and rolling-upgrade limits.

### Saved website logins

Save labels, exact HTTPS origins, usernames, passwords and optional TOTP on the
human-only Saved logins page. Secrets are write-only and envelope-encrypted.
NyxBot can use the owner's logins; specialists need explicit grants. Guests
never receive machine or saved-login tools.

Filling works only in node-managed Chromium with admin-installed policies and
the signed NyxID native-messaging extension. Policies disable DevTools,
`javascript:` URLs, password saving and autofill. The extension checks the
focused origin and input type, inserts with browser editing events and pins
password fields against reveal toggles until submit/navigation. The native
socket is inaccessible to the agent OS user on separated machines. Values and
common encodings are scrubbed from machine text, file and job outputs.

Setup requests administrator access once for the policies. Declining (or using
`--skip-browser-policy`) leaves filling unavailable and preserves other machine
features. On single-user hosts, filling is additionally off until the owner
acknowledges the warning and enables it in Assistant → Machines settings: agent commands run
as the same user as the browser and could read typed values. Prefer the
container or a separated VM. Only the human owner can change this setting or
`machine_confirm` (`none`, `changes`, `all`). A login can also require a card
for every sign-in.

The approved website and managed browser profile are trusted recipients; a
website that deliberately re-displays a password as text could make it visible
on screen, so use owner takeover for the most sensitive accounts.

Interactive TTY machine sessions and Windows containers are follow-ups.
See [MACHINE_NODES.md](MACHINE_NODES.md) for the authority model, protocol,
acceptance coverage and repeatable performance checks.

### Machine gateway declarations and isolation

Declare connected services on each command, for example:

```json
{"machine":"dev","command":"git clone https://github.com/owner/private.git","services":["api-github"]}
```

Only declared, still-accessible services are available to that job. Omission
means no service access. Git and SDK environment is set only for declarations;
plain git works after declaring the connected host. Undeclared calls return an
HTTP error instructing the agent to declare the missing service. Confirmation
cards and machine audit metadata list declarations. SDK variables come from
catalog `inference.wire_protocol`; git origins and Basic usernames come from
`git_http` metadata (GitHub seeds use `https://github.com` / `x-access-token`).
The signed environment spec contains local gateway paths, never provider
credentials. Both gateway hops preserve Content-Encoding and Content-Length,
so SDKs and git decode compressed responses normally while bodies stream.

Prefer the machine container or a VM installed with `--separate-users`. On a
single-user machine with shell enabled, commands can read the node's config,
stored credentials, signing secret and node token. Workspace limits apply to
file tools and working directories, not the shell. Setup/status and the Nodes
page show this warning; Machines keeps a Not isolated badge. The owner may proceed.

Chromium runs with its renderer sandbox enabled. Docker's default seccomp
profile blocks Chromium's namespace setup; use the versioned profile shipped at
`cli/resources/machine-container/seccomp.json`:

```sh
docker run --rm --shm-size=1g \
  --security-opt seccomp=cli/resources/machine-container/seccomp.json \
  ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:latest
```

The setup page downloads this profile in its single copyable command, and
`nyxid node docker start --machine` writes the embedded profile automatically.
It accepts `--separation automatic|require-separated|shared-legacy` and forwards
the choice as `NYXID_MACHINE_SEPARATION` to setup in the container. An omitted
choice on a new container means automatic. Before creating a new container, the
CLI runs the real probes in a disposable copy of the machine image with the
same seccomp settings and no node identity or workspace volumes. Failed
automatic selection offers the same default-No prompt on a terminal; scripts
must explicitly pass `--separation shared-legacy` to accept the warning.
Required separation always fails if support is unavailable. Setup checks again
after provisioning. Existing containers and stored assignments keep their
recorded choice; change an existing container's intent through `node setup`
inside it rather than passing a new choice to Docker start/restart.

The Docker CLI uses self-hosted registration and sets
`NYXID_MACHINE_MANAGED=0`, including when its image reference is locally tagged
with the node-only image. It forwards `NYXID_NODE_URL`, `NYXID_NODE_TOKEN` and
configured CA certificates only to the selected node container. The probe gets
none of these variables or certificate mounts. Image-declared anonymous volumes
may be created for the probe; Docker removes them with its `--rm` container.
Refused selection creates no named identity or workspace volume.

For a profile named `work`, the machine container is `nyxid-node-machine-work`.
Its named volumes are `nyxid-node-machine-work-state` at
`/var/lib/nyxid-machine` and `nyxid-node-machine-work-workspace` at `/workspace`.
The recorded choice is `[machine].separation_choice` in
`/var/lib/nyxid-machine/node/config.toml`; the CLI does not mount or modify the
host's `~/.nyxid-node/profiles/work` profile. `docker stop --machine` retains the
stopped container so a subsequent start preserves its selection and identity.
To retire it, remove that container explicitly; its named volumes remain until
you explicitly remove them. If you remove the container but keep its volumes,
the next start performs fresh preflight and requires an explicit legacy choice
on an unsupported host before it can reopen a legacy identity.

For a self-hosted browserless node-only image, explicitly set
`NYXID_MACHINE_MANAGED=0` and `NYXID_MACHINE_SEPARATION=shared-legacy` after
reviewing the warning above. Missing selection defaults to automatic and
refuses before registration because this image has no managed Chromium;
`require-separated` also refuses. Unknown or empty selections are rejected.
Existing node-only config volumes retain their stored intent. **A separate
container per agent is the recommended isolation model for node-only
containers**; containers share the host kernel and do not provide a VM boundary.
Full machine images also read `NYXID_MACHINE_SEPARATION` during first setup.

The profile adds clone/unshare/setns to the Moby default; no additional
capabilities are needed. Agent commands, file workers, cua and Chromium inherit
NoNewPrivs. The container test verifies nested renderer namespaces, seccomp and
NoNewPrivs; Chromium never receives `--no-sandbox`.

Owner live view uses X11/XFixes capture and independent XTest input on Linux,
and ScreenCaptureKit with a separate human cua input session on macOS. Agent
observations/actions remain on cua. At 30 Hz the encoder compares 64-pixel tiles
and sends a JPEG dirty rectangle only when pixels change. Sequence-bound deltas
recover with a full frame after loss; idle bandwidth is zero. Pixel buffers stay
in memory and only reach owner browser sockets. Takeover cancels in-flight
agent operations immediately, including file transfers, without capture locks.
See [validation and measurements](MACHINE_NODES.md#validation-and-measurements) for measured frame rate,
input latency, takeover latency and the exact macOS benchmark command.
