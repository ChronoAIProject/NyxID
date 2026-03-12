# nyxid-node Agent

`nyxid-node` is a lightweight Rust binary that runs on your infrastructure as a credential node agent. It connects to a NyxID server via WebSocket, receives proxy requests, injects locally stored credentials, and forwards requests to downstream services. Credentials never leave your infrastructure.

---

## Table of Contents

- [Installation](#installation)
- [Registration](#registration)
- [Starting the Agent](#starting-the-agent)
- [Managing Credentials](#managing-credentials)
- [Checking Status](#checking-status)
- [Configuration File](#configuration-file)
- [HMAC Request Signing](#hmac-request-signing)
- [Streaming Proxy Responses](#streaming-proxy-responses)
- [Reconnection and Resilience](#reconnection-and-resilience)
- [Graceful Shutdown](#graceful-shutdown)
- [Security](#security)
- [CLI Reference](#cli-reference)
- [Troubleshooting](#troubleshooting)

---

## Installation

Build from source (requires Rust 2024 edition):

```bash
# From the project root
cargo build --release -p nyxid-node

# Binary is at target/release/nyxid-node
```

Or install directly:

```bash
cargo install --path node-agent
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
nyxid-node register --token nyx_nreg_<64_hex_chars>
```

The agent connects to the NyxID server via WebSocket, exchanges the registration token for a permanent auth token and HMAC signing secret, and saves the encrypted configuration to `~/.nyxid-node/config.toml`.

#### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--token` | (required) | One-time registration token |
| `--url` | `ws://localhost:3001/api/v1/nodes/ws` | WebSocket URL of the NyxID server |
| `--config` | `~/.nyxid-node` | Path to config directory |

For production, use WSS:

```bash
nyxid-node register \
  --token nyx_nreg_... \
  --url wss://auth.example.com/api/v1/nodes/ws
```

On success, the agent prints the node ID and config file path:

```
Node registered successfully.
  Node ID: a1b2c3d4-...
  Config:  /home/user/.nyxid-node/config.toml

Start the agent with:
  nyxid-node start
```

---

## Starting the Agent

```bash
nyxid-node start
```

The agent:

1. Loads the configuration from `~/.nyxid-node/config.toml`
2. Decrypts the auth token and signing secret using the local keyfile
3. Decrypts all stored credentials
4. Connects to the NyxID server via WebSocket
5. Authenticates with its node ID and auth token
6. Begins serving proxy requests and responding to heartbeats

#### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--config` | `~/.nyxid-node` | Path to config directory |
| `--log-level` | `info` | Log level: `trace`, `debug`, `info`, `warn`, `error` |

The agent runs until terminated. Use `--log-level debug` for detailed connection and request logging.

---

## Managing Credentials

Credentials are stored locally in the config file, encrypted with AES-256-GCM. The agent decrypts them at startup and holds them in memory.

### Add a Credential (Header Injection)

```bash
nyxid-node credentials add \
  --service openai \
  --header "Authorization: Bearer sk-proj-..."
```

### Add a Credential (Query Parameter Injection)

```bash
nyxid-node credentials add \
  --service stripe \
  --query-param "api_key=sk_live_..."
```

### List Credentials

```bash
nyxid-node credentials list
```

Output:

```
Configured credentials:
  openai: header (Authorization)
  stripe: query_param (api_key)
```

### Remove a Credential

```bash
nyxid-node credentials remove --service openai
```

### Service Slug Matching

The `--service` value must match the **slug** of the downstream service in NyxID. When a proxy request arrives for a service, the agent looks up credentials by the service slug included in the request.

---

## Checking Status

```bash
nyxid-node status
```

Output:

```
Node Status
  Node ID:     a1b2c3d4-...
  Server:      wss://auth.example.com/api/v1/nodes/ws
  Credentials: 2 configured
    - openai
    - stripe
```

This is a local check only -- it reads the config file but does not connect to the server.

---

## Configuration File

The agent stores its configuration at `~/.nyxid-node/config.toml` (or the path specified by `--config`). The file is created during registration and updated when credentials are added or removed.

### Structure

```toml
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

### Encryption

All sensitive values (auth token, signing secret, credential values) are encrypted with AES-256-GCM using a locally generated 32-byte key stored at `~/.nyxid-node/.keyfile`. The keyfile is created with mode `0600` on Unix systems.

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
2. The agent sends `proxy_response_chunk` messages with base64-encoded data (max 64KB per chunk)
3. The agent sends `proxy_response_end` when the stream completes

NyxID reconstructs the streaming response and forwards it to the client as a standard SSE stream. This enables real-time streaming from LLM APIs (e.g., OpenAI chat completions with `stream=true`) through the node proxy.

---

## Reconnection and Resilience

The agent automatically reconnects on disconnection using exponential backoff:

| Attempt | Delay |
|---------|-------|
| 1 | 100ms |
| 2 | 200ms |
| 3 | 400ms |
| 4 | 800ms |
| ... | Doubles each time |
| Max | 60 seconds |

On a clean disconnect (server-initiated close), the backoff resets to the initial delay. On errors (network failure, auth rejection), the backoff increases.

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

### Local Encryption

- All secrets are encrypted with AES-256-GCM before writing to disk
- The encryption key is a 32-byte random value stored in `~/.nyxid-node/.keyfile`
- The keyfile is created with `O_CREAT | O_EXCL` and mode `0600` (Unix) to prevent race conditions
- Source byte arrays are zeroized after copying to prevent memory leakage
- Decrypted credential values are held in `Zeroizing<String>` wrappers

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
nyxid-node <COMMAND> [OPTIONS]

COMMANDS:
  register      Register this node with a NyxID server
  start         Start the node agent (connect and serve)
  status        Show node connection status
  credentials   Manage local credentials
  version       Show version information

GLOBAL OPTIONS:
  --log-level <LEVEL>   Log level: trace, debug, info, warn, error

REGISTER OPTIONS:
  --token <TOKEN>       One-time registration token (nyx_nreg_...)
  --url <URL>           WebSocket URL of the NyxID server
  --config <PATH>       Path to config directory

START OPTIONS:
  --config <PATH>       Path to config directory

STATUS OPTIONS:
  --config <PATH>       Path to config directory

CREDENTIALS SUBCOMMANDS:
  add     Add a credential for a service
  list    List configured credentials
  remove  Remove a credential for a service

CREDENTIALS ADD OPTIONS:
  --service <SLUG>          Service slug (e.g., "openai")
  --header <HEADER>         Header to inject (e.g., "Authorization: Bearer sk-...")
  --query-param <PARAM>     Query parameter to inject (e.g., "api_key=sk-...")
  --config <PATH>           Path to config directory

CREDENTIALS REMOVE OPTIONS:
  --service <SLUG>          Service slug to remove
  --config <PATH>           Path to config directory
```

---

## Troubleshooting

### "Config error: Failed to read config"

The config file does not exist. Run `nyxid-node register` first.

### "Authentication failed" on start

The auth token may have been rotated. Re-register the node with a new registration token, or update the config with the new token after rotation.

### "No credentials configured for service"

The service slug in the proxy request does not match any entry in the local credential store. Add the credential with:

```bash
nyxid-node credentials add --service <slug> --header "Authorization: Bearer ..."
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
