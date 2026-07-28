# Linux SSH Target

Prefer a dedicated unprivileged Unix account with one fixed git workspace. Install and authenticate Codex as that account.

## Target Setup

1. Install OpenSSH server and ensure the node can reach the chosen host/port.
2. Generate a dedicated Ed25519 client key on the node machine.
3. Install the wrapper from `forced-command-wrapper.md` at a root-owned or runner-owned path that unrelated users cannot modify.
4. Add the public key to the target account's `authorized_keys` with restrictions:

```text
restrict,command="/absolute/path/to/codex-command-wrapper" ssh-ed25519 PUBLIC_KEY_MATERIAL
```

5. Disable password authentication for the runner account and do not grant sudo.
6. Pin the target host key when running `nyxid node ssh-credentials add`.

For a same-machine node, use `127.0.0.1:22` only when sshd is explicitly configured for the dedicated runner. Otherwise run a separate loopback-only sshd on a high port, following the macOS shape with Linux paths and a systemd user unit.

A minimal `~/.config/systemd/user/aevatar-codex-sshd.service` for the separate daemon is:

```ini
[Unit]
Description=Loopback SSH target for Aevatar codex_exec

[Service]
ExecStart=/usr/sbin/sshd -D -e -f /absolute/runner-dir/sshd_config
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

Load it with:

```bash
systemctl --user daemon-reload
systemctl --user enable --now aevatar-codex-sshd.service
```

Enable user lingering only when the user explicitly needs the node and SSH target online while logged out.

On Linux, `nyxid node register --keychain` also requires a supported secret service. If it is unavailable, use NyxID's encrypted-file backend and keep its passphrase outside agent output; do not silently fall back to plaintext.

## Codex Sandbox Prerequisite

Codex uses Linux sandboxing for model-generated commands. Install `bubblewrap` when using `workspace-write`:

```bash
sudo apt install bubblewrap
```

Do not solve a broken user-namespace/AppArmor setup by switching the wrapper to danger-full-access. Fix the host sandbox or use an externally isolated managed runner.

## Node Allowlist

Add only the selected target to the node profile:

```toml
[ssh]
max_tunnels = 2
io_timeout_secs = 420

[[ssh.allowed_targets]]
host = "127.0.0.1"
port = 22
```

Restart only that node profile after editing it.
