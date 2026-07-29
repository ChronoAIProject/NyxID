# macOS Loopback SSH Target

Use a user-owned OpenSSH daemon on a high loopback-only port. This avoids enabling macOS Remote Login on every interface.

## Files

Choose a runner directory such as `$HOME/.local/share/nyxid-codex-runner`, then generate separate host and client keys:

```bash
mkdir -p "$RUNNER_DIR"
chmod 700 "$RUNNER_DIR"
ssh-keygen -q -t ed25519 -N '' -f "$RUNNER_DIR/ssh_host_ed25519_key"
ssh-keygen -q -t ed25519 -N '' -f "$RUNNER_DIR/runner_id_ed25519"
cp "$RUNNER_DIR/runner_id_ed25519.pub" "$RUNNER_DIR/authorized_keys"
chmod 600 "$RUNNER_DIR/ssh_host_ed25519_key" "$RUNNER_DIR/runner_id_ed25519" "$RUNNER_DIR/authorized_keys"
```

Install the wrapper from `forced-command-wrapper.md` as `$RUNNER_DIR/codex-command-wrapper`.

Create `$RUNNER_DIR/sshd_config` with rendered absolute paths and the current Unix user:

```text
Port 2222
ListenAddress 127.0.0.1
AddressFamily inet
HostKey /absolute/runner-dir/ssh_host_ed25519_key
PidFile /absolute/runner-dir/sshd.pid
AuthorizedKeysFile /absolute/runner-dir/authorized_keys

AllowUsers runner-user
AuthenticationMethods publickey
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
PermitRootLogin no
PermitEmptyPasswords no
ForceCommand /absolute/runner-dir/codex-command-wrapper

AllowAgentForwarding no
AllowTcpForwarding no
X11Forwarding no
PermitTunnel no
GatewayPorts no
PermitTTY no
PermitUserEnvironment no
PrintMotd no
PrintLastLog no
LogLevel VERBOSE
```

Validate and start it before adding persistence:

```bash
/usr/sbin/sshd -t -f "$RUNNER_DIR/sshd_config"
/usr/sbin/sshd -f "$RUNNER_DIR/sshd_config" -E "$RUNNER_DIR/sshd.log"
nc -zv 127.0.0.1 2222
```

Persist it with a user LaunchAgent whose program arguments are:

```text
/usr/sbin/sshd
-D
-e
-f
/absolute/runner-dir/sshd_config
```

Set `RunAtLoad` and `KeepAlive` to true, and write stdout/stderr under the runner directory. Validate the plist with `plutil -lint`, then load it with `launchctl bootstrap gui/$(id -u) path/to/plist`.

## Node Allowlist

In the selected node profile, replace the empty SSH target list with:

```toml
[ssh]
max_tunnels = 2
io_timeout_secs = 420

[[ssh.allowed_targets]]
host = "127.0.0.1"
port = 2222
```

Restart only that profile:

```bash
nyxid node daemon restart --profile "$NODE_PROFILE"
```

Use `$RUNNER_DIR/runner_id_ed25519` as `SSH_KEY_FILE` when adding the node-local SSH credential.
