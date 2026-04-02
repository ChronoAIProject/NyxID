# NyxID

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Turn your localhost into an MCP Server.**

NyxID is an open-source Agent Connectivity Gateway. It lets your AI agents
(Claude Code, Cursor, n8n) reach any API you have, public or private,
and handles all the credentials so your agent never sees a raw key.

```
Claude Code / Cursor / n8n
         |
         v
      NyxID (cloud gateway)
         |
    +----+----+
    v    v    v
 Public  Internal  localhost
  APIs    APIs     services
```

NyxID proxies requests, injects credentials automatically, punches through
NAT to reach your local services, and wraps any REST API as MCP tools.

## Quick Start

### 1. Sign up and add your APIs

Go to the [NyxID console](https://auth.nyxid.dev), create an account,
and add the API credentials you want your agents to use.

### 2. Install the CLI

```bash
# macOS / Linux
cargo install --git https://github.com/ChronoAIProject/NyxID.git nyxid-cli

# Then log in
nyxid login
```

### 3. Connect your AI agent

```bash
nyxid mcp config --tool claude-code   # or: --tool cursor
```

Follow the output to add NyxID to your MCP config. Done. Your agent
can now call any API you added through NyxID's proxy. Credentials are
injected automatically.

### 4. (Optional) Reach local services

Have services on localhost or behind a firewall? Deploy a credential node:

```bash
nyxid node register --token <reg-token> --url wss://nyx-api.chrono-ai.fun/api/v1/nodes/ws
nyxid node credentials add --service my-local-api --header Authorization
nyxid node start
```

The node makes an outbound WebSocket connection to NyxID. No port forwarding.
No VPN. Your AI agents can now reach localhost services through the tunnel.

## Quick Start (with AI assistant)

Paste this into Claude Code, Cursor, or any AI coding assistant:

> Help me set up NyxID. Install the CLI (cargo install --git
> https://github.com/ChronoAIProject/NyxID.git nyxid-cli), log in with
> nyxid login, add my OpenAI API key, and configure MCP so I can use
> NyxID-proxied tools from this session.

<!-- AI quickstart maintenance: validate this prompt against actual CLI on each release -->

## What NyxID does

**Reach anything.** Public APIs, internal APIs, localhost services. NyxID's
credential nodes (`nyxid node`) punch through NAT via outbound WebSocket.
SSH tunneling (`nyxid ssh`) reaches remote hosts. No VPN, no port forwarding.

**Never expose keys.** NyxID's reverse proxy injects credentials into every
request automatically. Your AI agent talks to NyxID. NyxID talks to the API
with the real key. The agent never sees it.

**MCP auto-wrap.** REST APIs with OpenAPI specs become MCP tools automatically.
`nyxid mcp config --tool cursor` generates the config. Works with Claude Code,
Cursor, VSCode, and any MCP client.

**Per-agent isolation.** Each agent session gets a scoped token. Agent A accesses
Slack and Gmail. Agent B only accesses your internal API. Revoke any session
without touching the underlying credentials.

**Full identity layer.** OIDC/OAuth 2.0 with PKCE, RBAC, service accounts,
transaction approval (Telegram + mobile push), LLM gateway for 7 providers.

## Why NyxID

| | NyxID | 1Password UA | Cloudflare Tunnel | Keycloak |
|---|---|---|---|---|
| Open source | Yes | No | No | Yes |
| NAT traversal to localhost | Yes (`nyxid node`) | No | Yes (no credentials) | No |
| Credential injection | Yes (any API) | Partner integrations | No | No |
| REST to MCP auto-wrap | Yes | No | No | No |
| Per-agent isolation | Yes | No | No | No |
| OIDC / OAuth 2.0 | Yes | No | No | Yes |

## Documentation

| Topic | Link |
|-------|------|
| API Reference | [docs/API.md](docs/API.md) |
| Architecture | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Credential Nodes | [docs/NODE_PROXY.md](docs/NODE_PROXY.md) |
| MCP Integration | [docs/MCP_DELEGATION_FLOW.md](docs/MCP_DELEGATION_FLOW.md) |
| AI Agent Playbook | [docs/AI_AGENT_PLAYBOOK.md](docs/AI_AGENT_PLAYBOOK.md) |
| SSH Tunneling | [docs/SSH_TUNNELING.md](docs/SSH_TUNNELING.md) |
| Security | [docs/SECURITY.md](docs/SECURITY.md) |
| Environment Variables | [docs/ENV.md](docs/ENV.md) |
| Deployment (self-host) | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| Developer Guide | [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md) |

## Self-hosting

NyxID is fully open source. If you prefer to run your own instance:

```bash
git clone https://github.com/ChronoAIProject/NyxID.git && cd NyxID
cp .env.example .env
# Edit .env: set ENCRYPTION_KEY=$(openssl rand -hex 32)
docker compose up -d                              # MongoDB
cargo run --manifest-path backend/Cargo.toml &    # Backend on :3001
(cd frontend && npm install && npm run dev) &     # Frontend on :3000
```

See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for production setup.

## Contributing

We welcome contributions. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT
