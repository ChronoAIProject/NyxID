---
name: nyxid-node-install
description: Use when you need to install, register, repair, or validate the NyxID `nyxid-node` credential agent on a machine. Covers choosing `cargo install --path node-agent` vs `cargo build --release -p nyxid-node`, selecting file vs keychain storage, adding credentials with the prompted `nyxid-node credentials add` flow, and verifying the node is online.
---

# NyxID Node Install

Use this skill when an AI agent is responsible for setting up or repairing `nyxid-node` on a user-controlled machine.

## Workflow

1. Confirm the operating context.
- If you are working inside this repository checkout, prefer `cargo install --path node-agent` when the user wants `nyxid-node` available on `PATH`.
- Use `cargo build --release -p nyxid-node` when the user wants a repo-local build artifact at `target/release/nyxid-node`.
- Collect the NyxID WebSocket URL and one-time registration token before starting registration. The built-in development default is `ws://localhost:3001/api/v1/nodes/ws`.

2. Choose the storage backend before registration.
- Default to file storage on servers, containers, CI, and headless Linux.
- Use `--keychain` only when the host has a working desktop keychain service such as macOS Keychain, Windows Credential Manager, or Linux Secret Service.

3. Install the binary.

```bash
cargo install --path node-agent
```

Alternative repo-local build:

```bash
cargo build --release -p nyxid-node
```

4. Register the node.

```bash
nyxid-node register --token nyx_nreg_... --url wss://auth.example.com/api/v1/nodes/ws
```

- Add `--keychain` when the chosen backend is the OS keychain.
- Registration writes `config.toml`, stores the auth token and signing secret, and prints the node ID.

5. Add credentials without putting secrets on the command line.

Bearer header:

```bash
nyxid-node credentials add --service openai --header Authorization --secret-format bearer
```

Basic auth header:

```bash
nyxid-node credentials add --service github --header Authorization --secret-format basic
```

Raw header:

```bash
nyxid-node credentials add --service resend --header X-API-Key
```

Query parameter:

```bash
nyxid-node credentials add --service stripe --query-param api_key
```

- The CLI prompts securely for the value. Do not use legacy inline `Name: value` or `name=value` forms unless the user explicitly accepts shell-history exposure.
- Match `--service` to the NyxID downstream service slug exactly.
- For `--secret-format basic`, the prompt input must be `username:password`.

6. Validate the setup.

```bash
nyxid-node status
nyxid-node start --log-level info
```

- `status` is local-only. After starting the process, confirm in the NyxID UI that the node is online and bind the intended services.
- Use `--log-level debug` when investigating connection or credential issues.

7. Use the least-destructive repair path.
- Use `nyxid-node rekey --auth-token ... --signing-secret ...` after server-side credential rotation.
- Use `nyxid-node migrate --to keychain` or `--to file` when changing storage backend.
- Re-register only when the config is missing or the node can no longer authenticate with its existing auth material.

## Guardrails

- Never echo or paste API keys into the shell unless the user explicitly asks for inline secrets.
- Treat keychain support as unavailable on headless Linux unless you can verify a working Secret Service daemon.
- Prefer `wss://` for production and `ws://localhost:3001/api/v1/nodes/ws` for local development.

## References

- Full user guide: `docs/NYXID_NODE.md`
- Server-side setup and node binding flow: `docs/NODE_PROXY.md`
