---
name: install-nyxid-node
description: Install, register, and operationalize the NyxID Node Agent (`nyxid-node`) on a user workstation or server. Use when Codex needs to set up, repair, or migrate a NyxID credential node, choose between file and keychain secret storage, add service credentials, or configure the agent to stay running via systemd or launchd.
---

# Install NyxID Node

Implement end-to-end setup of `nyxid-node` on the target host. Treat [`docs/NYXID_NODE.md`](../../../docs/NYXID_NODE.md) as the CLI and local-operations source of truth, and treat [`docs/NODE_PROXY.md`](../../../docs/NODE_PROXY.md) as the server-side routing and binding source of truth. Check [`node-agent/src/cli.rs`](../../../node-agent/src/cli.rs) and [`node-agent/src/main.rs`](../../../node-agent/src/main.rs) before inventing flags or behavior.

## Collect Inputs First

Before you touch the host, confirm these inputs:

- Target OS and service manager (`systemd`, `launchd`, neither)
- Whether Rust and Cargo are already installed
- Desired install mode: keep binary in the repo build output, install with `cargo install --path node-agent`, or copy the release binary to an explicit location
- NyxID WebSocket URL
- One-time registration token (`nyx_nreg_...`)
- Preferred config directory if the default `~/.nyxid-node` should not be used
- Secret storage backend: default to `file`; use `keychain` only when an interactive OS keychain is known to be available
- Service slugs and injection method for each credential to be stored

Do not ask for secrets earlier than necessary. Gather enough context to choose commands, then request the registration token and credential values right before you use them.

## Inspect Before Installing

Start by learning whether this is a fresh install or a repair:

1. Check for an existing config directory and `config.toml`.
2. Run `nyxid-node version` if the binary is already present.
3. If a config already exists, run `nyxid-node status` and inspect the configured storage backend before overwriting anything.
4. On Linux, determine whether `systemctl` is available. On macOS, determine whether `launchctl` is available.

If the host already has a working node, treat the task as repair or migration instead of reinstalling blindly.

## Install Workflow

### 1. Build or install the binary

Default to one of these supported flows:

- `cargo install --path node-agent`
  Use when the current checkout is trusted and the user wants `nyxid-node` on `PATH`.
- `cargo build --release -p nyxid-node`
  Use when the user wants a portable binary or does not want Cargo to manage the install location. The binary will be at `target/release/nyxid-node`.

If you copy the release binary into a system location such as `/usr/local/bin`, confirm the destination and keep the copied path consistent with any service unit you later install.

### 2. Choose the storage backend deliberately

- Use the default file backend on headless Linux hosts, CI-like environments, containers, or any machine where a desktop keychain may not exist.
- Use `--keychain` only when the host is expected to support macOS Keychain, Windows Credential Manager, or Linux Secret Service and the user wants secrets kept out of `config.toml`.
- If a keychain-backed registration fails during preflight, fall back to the file backend instead of retrying the same broken path.

### 3. Register the node

Use `wss://.../api/v1/nodes/ws` for production and internet-reachable NyxID servers. Use the default `ws://localhost:3001/api/v1/nodes/ws` only for local development.

Registration command shape:

```bash
nyxid-node register \
  --token "$REGISTRATION_TOKEN" \
  --url "$WS_URL"
```

Add these flags only when needed:

- `--keychain`
- `--config /custom/config/dir`

After registration:

- Capture the reported node ID, storage backend, and config path.
- Keep the generated config and backend secrets in place.
- Do not expose the auth token or signing secret in shell history or logs.

### 4. Add local service credentials

Prefer the secure prompt-based form over inline secrets:

```bash
nyxid-node credentials add \
  --service openai \
  --header Authorization \
  --secret-format bearer
```

Use query parameter injection only when the downstream service actually expects it:

```bash
nyxid-node credentials add \
  --service stripe \
  --query-param api_key
```

Rules:

- The `--service` value must match the NyxID downstream service slug.
- Avoid inline secret values unless the user explicitly wants automation and accepts shell-history exposure.
- When repairing an existing node, list credentials first so you do not silently replace the wrong service entry.

### 5. Make the agent persistent

If the user wants the node to survive reboots, install a service definition:

- Linux with `systemd`: use [`assets/systemd/nyxid-node.service`](./assets/systemd/nyxid-node.service)
- macOS with `launchd`: use [`assets/launchd/dev.nyxid.node-agent.plist`](./assets/launchd/dev.nyxid.node-agent.plist)

Replace the placeholders before loading the unit. Keep the `BINARY_PATH`, `CONFIG_DIR`, and `WORKING_DIRECTORY` aligned with the install method you chose earlier.

On Windows, keep the install to the binary, registration, and credential flow unless the user explicitly asks for persistence. No Windows service template is bundled here; use the platform-native scheduler or service manager only after confirming the user's preference.

If neither `systemd` nor `launchd` is available, call out that a foreground process or terminal multiplexer is only a fallback, not a durable install.

## Validate Before You Finish

Run the smallest set of checks that prove the install is real:

1. `nyxid-node version`
2. `nyxid-node status`
3. Service-manager status if you installed one
4. A log check if startup fails
5. Confirmation in the NyxID UI or API that the node is online

Use [`references/install-and-validate.md`](./references/install-and-validate.md) for command patterns and [`references/background-services.md`](./references/background-services.md) for service-manager steps.

## Troubleshooting Rules

- `Registration failed`: suspect an expired token or wrong WebSocket URL first.
- Keychain errors on Linux servers: switch to the file backend unless the host clearly has Secret Service configured.
- `status` fails because `config.toml` is missing: registration never completed successfully or the wrong config directory is being used.
- The node stays offline after a service install: inspect the service logs, confirm outbound WebSocket reachability, and verify the stored auth token was not rotated on the server.
- Requests fail after install: confirm the stored service slug and credential injection method match the bound downstream service.

## Boundaries

- Do not invent unsupported CLI flags or config fields.
- Do not edit the NyxID server to compensate for a host-local install problem unless the evidence points to a server-side bug.
- Do not destroy an existing config or stored credentials without explicit user approval.
