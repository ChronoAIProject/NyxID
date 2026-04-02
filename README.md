# NyxID

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Turn your localhost into an MCP Server.**

Your AI agent lives in the cloud. Your APIs live on your network.
NyxID bridges the gap. Open source. Self-hosted. Written in Rust.

## Why we built this

We run AI agents that need to call APIs. Some are public (OpenAI, Slack).
Some are private (our database, our internal tools, HomeAssistant on our LAN).

The public ones are easy. The private ones? No way to reach them from the cloud.
And every agent needs credentials scattered across .env files on 3 machines.

So we built a gateway. It tunnels into your network, injects credentials
automatically, and wraps any REST API as an MCP tool. That's NyxID.

## How it works

```
Claude Code / Cursor / n8n (cloud)
         |
         v
      NyxID Gateway --- credentials injected automatically
         |
    +----+----+
    v    v    v
 Public  Internal  localhost
  APIs    APIs     services
         (NAT traversal)
```

## Quick Start (with AI assistant)

Paste this into Claude Code, Cursor, or any AI coding assistant:

> Set up NyxID on my machine. Clone github.com/ChronoAIProject/NyxID,
> run docker compose up -d for MongoDB, build and start the backend
> (cargo run --manifest-path backend/Cargo.toml), start the frontend
> (cd frontend && npm install && npm run dev), then install the CLI
> (cargo install --path cli), log in with nyxid login, and show me
> how to add my first API credential and configure MCP for Claude Code.

Your AI assistant will handle the setup. When it's done, run:

```bash
curl http://localhost:3001/health   # Should return {"status":"ok"}
nyxid mcp config --tool claude-code # Generates your MCP config
```

<!-- AI quickstart maintenance: validate this prompt against actual repo setup on each release -->

## Quick Start (manual)

Prerequisites: Docker, Rust 1.85+, Node.js 20+

```bash
git clone https://github.com/ChronoAIProject/NyxID.git && cd NyxID
cp .env.example .env
# Edit .env: set ENCRYPTION_KEY=$(openssl rand -hex 32)

# Start infrastructure + backend + frontend
docker compose up -d                              # MongoDB + Mailpit
cargo run --manifest-path backend/Cargo.toml &    # Backend on :3001
(cd frontend && npm install && npm run dev) &     # Frontend on :3000

# Install CLI and verify
cargo install --path cli
nyxid login --base-url http://localhost:3001
nyxid status
```

Add API credentials at http://localhost:3000, then:

```bash
nyxid mcp config --tool claude-code   # or: --tool cursor
```

For production deployment, see [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Connect a local service

Run a credential node on your network. It tunnels back to NyxID
so your AI agents can reach services on localhost or behind a firewall.

```bash
nyxid node register --token <reg-token> --url ws://localhost:3001/api/v1/nodes/ws
nyxid node credentials add --service my-local-api --header Authorization
nyxid node start
```

No port forwarding. No VPN. The node makes an outbound WebSocket connection.
Multi-node failover, HMAC-SHA256 request signing.

See [docs/NODE_PROXY.md](docs/NODE_PROXY.md) for full setup.

## Features

**Connectivity**
- NAT traversal via credential nodes (`nyxid node`) ... reach localhost from the cloud
- SSH-over-WebSocket tunneling (`nyxid ssh`) ... reach remote hosts
- MCP auto-wrap ... REST APIs become MCP tools automatically (from OpenAPI specs)

**Security**
- Reverse proxy with automatic credential injection ... agents never see raw keys
- Per-agent session isolation ... scoped access, revocable
- AES-256-GCM encrypted credential storage
- Transaction approval via Telegram or mobile push

**Identity**
- Full OIDC/OAuth 2.0 provider with PKCE
- RBAC with roles, groups, permissions
- Service accounts for machine-to-machine auth

**Infrastructure**
- LLM gateway ... unified endpoint for 7 AI providers (OpenAI, Anthropic, Google AI, Mistral, Cohere, DeepSeek, Codex)
- API documentation discovery and catalog
- Mobile app for approvals (iOS + Android)

## Why NyxID

| | NyxID | 1Password UA | Cloudflare Tunnel | Keycloak |
|---|---|---|---|---|
| Open source, self-hosted | Yes | No | No | Yes |
| NAT traversal to localhost | Yes (`nyxid node`) | No | Yes (no credentials) | No |
| Credential injection (reverse proxy) | Yes (any API) | Partner integrations only | No | No |
| REST to MCP auto-wrap | Yes (from OpenAPI specs) | No | No | No |
| Per-agent isolation | Yes | No | No | No |
| OIDC / OAuth 2.0 | Yes | No | No | Yes |

## Documentation

| Topic | Link |
|-------|------|
| API Reference | [docs/API.md](docs/API.md) |
| Architecture | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Deployment | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) |
| Environment Variables | [docs/ENV.md](docs/ENV.md) |
| SSH Tunneling | [docs/SSH_TUNNELING.md](docs/SSH_TUNNELING.md) |
| Credential Nodes | [docs/NODE_PROXY.md](docs/NODE_PROXY.md) |
| MCP Integration | [docs/MCP_DELEGATION_FLOW.md](docs/MCP_DELEGATION_FLOW.md) |
| AI Agent Playbook | [docs/AI_AGENT_PLAYBOOK.md](docs/AI_AGENT_PLAYBOOK.md) |
| Security | [docs/SECURITY.md](docs/SECURITY.md) |
| Developer Guide | [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md) |
| OpenClaw Integration | [docs/OPENCLAW_INTEGRATION.md](docs/OPENCLAW_INTEGRATION.md) |

## Contributing

We welcome contributions. See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow, coding conventions, and pull request process.

## License

MIT
