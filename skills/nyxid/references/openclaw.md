# OpenClaw Integration

```bash
nyxid openclaw setup --url http://localhost:18789   # CLI prompts for token securely
```

**For OpenClaw users:** After installing or updating this skill, start a new chat to activate it. If the gateway isn't installed as a background service yet, set it up so it stays running and restarts automatically:

```bash
openclaw gateway status    # check if already running as a service
openclaw gateway install   # install as system service (systemd on Linux, launchd on macOS)
openclaw gateway start     # start the service
```

Without this, restarting the gateway (`openclaw gateway restart`) will shut it down and it won't come back up on its own.

**Transport selection.** `llm-openclaw` supports both HTTP proxy (`POST /v1/chat/completions`, etc.) and WebSocket passthrough (the OpenClaw CLI's native `connect` + `chat.send` flow). Check `nyxid proxy discover --output json` — the entry exposes `"streaming_supported": true` and `"websocket_supported": true`. Use `wss://<nyxid-host>/api/v1/proxy/s/llm-openclaw` with a `Bearer` token for the WebSocket path. If it's node-routed and the WS upgrade times out, update the node agent and restart its daemon (older agents pre-date WS proxy support).

**Scope and routing headers.** The service-level default-header flag closes the workflow described in NyxID#161 -- agents no longer need to remember `x-openclaw-scopes` per call. Set it once on the UserService and every call carries it automatically:

```bash
nyxid service update <user-service-id> --default-header 'x-openclaw-scopes=operator.read,operator.write'
```
