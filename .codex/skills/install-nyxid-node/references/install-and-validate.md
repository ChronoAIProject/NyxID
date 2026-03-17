# Install And Validate

## Source Of Truth

- CLI behavior: `docs/NYXID_NODE.md`
- Node routing and bindings: `docs/NODE_PROXY.md`
- Supported flags: `node-agent/src/cli.rs`

## Install Commands

### Install Onto PATH With Cargo

```bash
cargo install --path node-agent
```

Use this when the repo checkout is available and the target host should get `nyxid-node` on `PATH` through Cargo's bin directory.

### Build A Release Binary

```bash
cargo build --release -p nyxid-node
```

Result:

- Binary path: `target/release/nyxid-node`

Use this when you need an explicit artifact to copy into `/usr/local/bin`, a service directory, or another managed location.

## Registration Commands

### File Backend

```bash
nyxid-node register \
  --token "$REGISTRATION_TOKEN" \
  --url "$WS_URL"
```

### Keychain Backend

```bash
nyxid-node register \
  --token "$REGISTRATION_TOKEN" \
  --url "$WS_URL" \
  --keychain
```

### Custom Config Directory

```bash
nyxid-node register \
  --token "$REGISTRATION_TOKEN" \
  --url "$WS_URL" \
  --config "$CONFIG_DIR"
```

## Credential Commands

### Header Injection With Bearer Formatting

```bash
nyxid-node credentials add \
  --service openai \
  --header Authorization \
  --secret-format bearer
```

### Query Parameter Injection

```bash
nyxid-node credentials add \
  --service stripe \
  --query-param api_key
```

### Inspect Existing Credentials

```bash
nyxid-node credentials list
```

## Validation Checklist

1. Confirm the binary is callable:

```bash
nyxid-node version
```

2. Confirm local config is readable:

```bash
nyxid-node status
```

3. If a custom config directory is in use, repeat with `--config "$CONFIG_DIR"`.
4. If a background service was installed, inspect its status and recent logs.
5. Confirm the node is shown as online in the NyxID UI or via the node APIs.

## Repair Checklist

If the node is already installed:

1. Inspect `nyxid-node status` before editing anything.
2. Check whether the storage backend is `file` or `keychain`.
3. List existing credentials before replacing them.
4. If the server rotated the auth token, use `nyxid-node rekey`.
5. If the storage backend needs to change, use `nyxid-node migrate --to keychain` or `--to file`.
