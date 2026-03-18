# Background Services

## systemd

Use the template at [`docs/node-agent-install/assets/systemd/nyxid-node.service`](./assets/systemd/nyxid-node.service).

### Placeholder meanings

- `{{RUN_AS_USER}}`: the non-root account that owns the config directory and should run the agent
- `{{WORKING_DIRECTORY}}`: usually the repo root or the directory that contains the installed binary
- `{{BINARY_PATH}}`: absolute path to `nyxid-node`
- `{{CONFIG_DIR}}`: absolute path to the config directory, not the `config.toml` file
- `{{LOG_LEVEL}}`: one of `trace`, `debug`, `info`, `warn`, `error`

### Typical install flow

```bash
sudo cp nyxid-node.service /etc/systemd/system/nyxid-node.service
sudo systemctl daemon-reload
sudo systemctl enable --now nyxid-node.service
sudo systemctl status nyxid-node.service
sudo journalctl -u nyxid-node.service -n 100 --no-pager
```

## launchd

Use the template at [`docs/node-agent-install/assets/launchd/dev.nyxid.node-agent.plist`](./assets/launchd/dev.nyxid.node-agent.plist).

### Placeholder meanings

- `{{LABEL}}`: launchd label, for example `dev.nyxid.node-agent`
- `{{BINARY_PATH}}`: absolute path to `nyxid-node`
- `{{CONFIG_DIR}}`: absolute path to the config directory
- `{{WORKING_DIRECTORY}}`: stable working directory for the process
- `{{LOG_LEVEL}}`: one of `trace`, `debug`, `info`, `warn`, `error`
- `{{STDOUT_PATH}}` and `{{STDERR_PATH}}`: writable log files

### Typical install flow

```bash
mkdir -p ~/Library/LaunchAgents
cp dev.nyxid.node-agent.plist ~/Library/LaunchAgents/
launchctl bootstrap "gui/$(id -u)" ~/Library/LaunchAgents/dev.nyxid.node-agent.plist
launchctl enable "gui/$(id -u)/dev.nyxid.node-agent"
launchctl kickstart -k "gui/$(id -u)/dev.nyxid.node-agent"
launchctl print "gui/$(id -u)/dev.nyxid.node-agent"
```

If the service was already loaded, use `launchctl bootout` on the same label before reloading the updated plist.

## Windows

No Windows service definition is bundled in this repository. If persistence is needed on Windows:

1. Finish the normal install, registration, and credential setup first.
2. Confirm whether Task Scheduler or a Windows service wrapper is preferred.
3. Keep the binary path and config directory explicit so the persistence layer can call:

```powershell
nyxid-node --log-level info start --config C:\path\to\.nyxid-node
```
