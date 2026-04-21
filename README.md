[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![GitHub Stars](https://img.shields.io/github/stars/ChronoAIProject/NyxID)](https://github.com/ChronoAIProject/NyxID)
[![Discord](https://img.shields.io/discord/1422500823925264438?logo=discord&logoColor=white&label=Discord&color=5865F2)](https://discord.gg/fQw5Nep9)

<p align="center">
  <img src="assets/banner.png" alt="NyxID — Connect AI agents to any API, anywhere. Securely." width="100%">
</p>

**Connect AI agents to any API, anywhere. Securely.** Open-source Agent Connectivity Gateway.

NyxID lets your AI agents (Claude Code, Cursor, n8n) reach any API you have,
public or private, and handles all the credentials so your agent never sees
a raw key.

```mermaid
flowchart LR
    subgraph Agents["AI Agents"]
        CC[Claude Code]
        CU[Cursor]
        N8[n8n]
    end

    subgraph NyxID["NyxID Gateway"]
        AUTH[OIDC and API Key Auth]
        PROXY[Credential Injection Proxy]
        MCP[MCP Tool Wrapping]
    end

    subgraph Connectivity["Private Reach"]
        NODE[Credential Node]
    end

    subgraph Targets["Connected Services"]
        PUB[Public APIs]
        INT[Internal APIs]
        LOC[Localhost Services]
    end

    CC --> AUTH
    CU --> AUTH
    N8 --> AUTH

    AUTH --> PROXY
    PROXY --> MCP
    PROXY -->|Direct proxy| PUB
    PROXY -->|Private network| INT
    PROXY -->|NAT traversal| NODE
    NODE --> LOC

    classDef agents fill:#eef2ff,stroke:#4f46e5,color:#111827,stroke-width:1.5px;
    classDef gateway fill:#111827,stroke:#22d3ee,color:#ffffff,stroke-width:2px;
    classDef tooling fill:#e0f2fe,stroke:#0284c7,color:#0f172a,stroke-width:1.5px;
    classDef node fill:#fef3c7,stroke:#f59e0b,color:#111827,stroke-width:1.5px;
    classDef targets fill:#ecfeff,stroke:#14b8a6,color:#0f172a,stroke-width:1.5px;

    class CC,CU,N8 agents;
    class AUTH,PROXY gateway;
    class MCP tooling;
    class NODE node;
    class PUB,INT,LOC targets;
```

NyxID proxies requests, injects credentials automatically, punches through
NAT (Network Address Translation) to reach your local services, and wraps
any REST API as MCP (Model Context Protocol) tools.

## What NyxID Does

- **Reach anything** — public APIs, internal APIs, localhost services via credential nodes (`nyxid node`). SSH (Secure Shell) tunneling (`nyxid ssh`) reaches remote hosts. No VPN (Virtual Private Network), no port forwarding.
- **Never expose keys** — the reverse proxy injects credentials automatically. Your agent talks to NyxID; NyxID talks to the API with the real key.
- **MCP auto-wrap** — REST APIs with OpenAPI specs become [MCP](https://modelcontextprotocol.io/) (Model Context Protocol) tools. `nyxid mcp config --tool cursor` generates the config. Works with Claude Code, Cursor, VS Code, and any MCP client.
- **Per-agent isolation** — each agent gets a scoped token. Agent A accesses Slack and Gmail. Agent B only accesses your internal API. Revoke any session without touching the underlying credentials.
- **Full identity layer** — OIDC (OpenID Connect) / OAuth 2.0 with PKCE (Proof Key for Code Exchange), RBAC (Role-Based Access Control), service accounts, transaction approval (Telegram + mobile push), LLM (Large Language Model) gateway for 7 providers.

## See It in Action

The end-to-end loop is short: connect a service to NyxID once, then any AI agent pointed at your NyxID MCP endpoint can use it — without ever seeing the raw API key.

1. **Add a service** in the web console — paste your OpenAI (or Anthropic, GitHub, etc.) key once; NyxID stores it encrypted.
2. **Wire up your AI tool** — `claude mcp add --transport http nyxid http://localhost:3001/mcp` (or one-click install for Cursor in **Settings → MCP**).
3. **Use it** — Claude Code, Cursor, or any MCP client can now call the service through NyxID. The agent sees the response; never the key.

<p align="center">
  <img src="assets/demo.gif" alt="NyxID end-to-end demo — connect a service, wire up MCP, call through the proxy" width="80%">
</p>

<p align="center"><a href="https://cdn.godgpt-labs.workers.dev/nyx-ai-service-flow-demo.mp4" target="_blank" rel="noopener noreferrer">▶ Watch fullscreen</a></p>

## Why NyxID

Other tools solve parts of this — NyxID combines credential injection, NAT traversal, and MCP tooling in one open-source gateway:

| | NyxID | 1Password Universal Autofill | Cloudflare Tunnel | Keycloak |
|---|---|---|---|---|
| Open source | Yes | No | No | Yes |
| NAT traversal to localhost | Yes (`nyxid node`) | No | Yes (no credentials) | No |
| Credential injection | Yes (any API) | Partner integrations | No | No |
| REST to MCP auto-wrap | Yes | No | No | No |
| Per-agent isolation | Yes | No | No | No |
| OIDC / OAuth 2.0 | Yes | No | No | Yes |

## Use Cases

- Give Claude Code access to your private APIs without sharing keys
- Expose internal microservices to AI agents through a single MCP endpoint
- Secure AI agent access to self-hosted tools (Grafana, Jenkins, n8n) behind your firewall

## Quick Start

There are two ways to use NyxID — pick the one that fits your situation:

| | Hosted | Self-host |
|---|---|---|
| **What it is** | We run NyxID for you in the cloud | You run NyxID on your own machine |
| **Best for** | Getting started quickly, no setup | Full control, private networks, offline use |
| **Status** | Early access (invite code below) | Open — anyone can run it |

### Option A: Hosted (Recommended)

Start using NyxID in under a minute — no Docker, no setup:

1. Go to **[nyx.chrono-ai.fun/register](https://nyx.chrono-ai.fun/register)**
2. Enter invite code: **`NYX-FGNY85AF`**
3. Sign in with Google, GitHub, or Apple
4. [Connect your first AI Service](#connecting-ai-services) and verify a proxy request

Early access — limited to 20 users.

### Option B: Self-Host

Run NyxID on your own machine. This sets up three Docker containers (database, backend, frontend) — takes about 2 minutes.

**Prerequisites:** [Docker](https://docs.docker.com/get-docker/) and a bash-compatible terminal. The `nyxid` CLI is optional. Full prereqs and disk budgets in [docs/QUICKSTART.md](docs/QUICKSTART.md#prerequisites).

#### AI-Assisted (Recommended)

If you have Claude Code, Cursor, or any AI coding assistant open, paste the prompt below into it and it will drive the entire self-host flow for you — preflight, clone, env generation, Docker stack, health check, optional CLI install, login, first credential, and MCP config.

<details>
<summary><strong>Click to expand the full AI-assisted self-host prompt</strong></summary>

> I want to self-host NyxID on this machine (the repo is https://github.com/ChronoAIProject/NyxID). Walk me through the full quickstart interactively. If anything fails or I'd prefer to follow the manual steps myself, the full step-by-step with troubleshooting is at https://github.com/ChronoAIProject/NyxID/blob/main/docs/QUICKSTART.md.
> 1. Confirm Docker is installed and running before touching anything (check `git`, `docker`, `openssl`, `curl`, `docker compose` v2, and `docker info`).
> 2. **Before cloning or generating anything, check whether NyxID install STATE is present** — look for a `./NyxID/.env.dev` file OR any Docker volume matching `nyx*_mongodb_data` (run `docker volume ls --format '{{.Name}}' | grep -E 'nyx.*_mongodb_data$'` — this catches the default `nyxid_mongodb_data` plus any variant from a renamed checkout). A bare `./NyxID` directory alone does NOT count as "installed" — `uninstall.sh` leaves the source tree in place, so the directory can exist with no state. **If install state is present, stop and tell me the quickstart is a first-time-only install.** Ask whether I want to (a) uninstall first — if `./NyxID` exists, run `cd NyxID && ./scripts/uninstall.sh --yes && cd ..`; if only the stale Docker volume is orphaned (checkout was manually deleted earlier), run `docker volume ls --format '{{.Name}}' | grep -E 'nyx.*_mongodb_data$' | xargs -r docker volume rm` directly. Either path wipes the volume, containers, and (for the script path) `.env.dev`/keys — destroys all NyxID accounts and encrypted credentials. Or (b) keep my existing install and stop here — I can verify it's still running with `curl -sf http://localhost:3001/health`. Do not proceed to step 3 until I answer.
> 3. If `./NyxID` already exists (post-uninstall reinstall), `cd` into it; otherwise clone the repo into the current directory and `cd` in. Generate `.env.dev` with a fresh `ENCRYPTION_KEY` and `MONGO_ROOT_PASSWORD` (set `ENVIRONMENT=development`, `INVITE_CODE_REQUIRED=false`, `AUTO_VERIFY_EMAIL=true`, and `EMAIL_AUTH_ENABLED=true` so I don't get stuck on email verification or a locked-down signup page), symlink it to `.env.production`, create the PKCS#1 JWT signing keys under `keys/` (with a LibreSSL fallback using `-pubout` if `-RSAPublicKey_out` isn't supported), then pull images and start the stack with `docker compose -f docker-compose.yml -f docker-compose.prod.yml --env-file .env.production up -d`. Wait up to 90 seconds for `http://localhost:3001/health` to return 200 — if it times out, tell me to run `docker logs nyxid-backend`. If the logs show `SCRAM failure: Authentication failed`, that means the MongoDB volume has a stale password from a previous install — tell me to run `./scripts/uninstall.sh --yes` (or, if the checkout is gone, `docker volume ls --format '{{.Name}}' | grep -E 'nyx.*_mongodb_data$' | xargs -r docker volume rm` to remove any nyx-flavored orphan volume) and retry. Show me the generated `ENCRYPTION_KEY` so I can back it up.
> 4. Tell me to open http://localhost:3000 and register my account (no email verification needed — accounts are auto-verified in dev mode), and wait until I confirm I've done that.
> 5. **Ask me whether I want to install the `nyxid` CLI.** Explain that it's optional, that the installer will pull the Rust toolchain (~300 MB) if I don't have it, and that the first build takes 3–10 minutes and ~1.5 GB of disk. If I say yes, install it using https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh, then `source ~/.cargo/env`, log me in with `nyxid login --base-url http://localhost:3001`, add my OpenAI key with `nyxid service add llm-openai --credential-env OPENAI_API_KEY`, and verify with `nyxid proxy request llm-openai models`. If I say no, walk me through adding the same OpenAI credential in the web console instead.
> 6. Finish by connecting my AI tool to NyxID's MCP endpoint at `http://localhost:3001/mcp`. For Claude Code: `claude mcp add --transport http nyxid http://localhost:3001/mcp`. For Codex: `codex mcp add nyxid --url http://localhost:3001/mcp`. For Cursor: open **Settings > MCP** in the web console and click **Install to Cursor**.

</details>

<!-- AI quickstart maintenance: validate this prompt against actual CLI + web console on each release -->

#### Manual Setup

Prefer to run each step yourself, or need the full troubleshooting guide? The complete manual flow lives in **[docs/QUICKSTART.md](docs/QUICKSTART.md)**:

- System preflight check — [Step 1](docs/QUICKSTART.md#step-1-of-3--check-your-system)
- One paste-block install — [Step 2](docs/QUICKSTART.md#step-2-of-3--install-and-start)
- Register your account — [Step 3](docs/QUICKSTART.md#step-3-of-3--register-and-connect)
- Optional [CLI install](docs/QUICKSTART.md#optional-install-the-nyxid-cli)
- [Uninstall & reinstall](docs/QUICKSTART.md#uninstall--reinstall), [orphan volume recovery](docs/QUICKSTART.md#recovering-an-orphan-volume), and [SCRAM failure](docs/QUICKSTART.md#stuck-on-scram-failure) troubleshooting

Once NyxID is running, jump to [Connecting AI Services](#connecting-ai-services) below to connect your first downstream API.

For production deployment (TLS, custom domain, email verification), see [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Connecting AI Services

After NyxID is running (hosted or self-host), the next step is to connect a downstream API — OpenAI, Anthropic, GitHub, your private API, anything — so your AI agents can call it through the proxy without ever seeing the raw key.

> **Wiring MCP alone won't show real tools.** Until you also connect a real downstream service *and* verify the proxy works, your AI agent will only see NyxID's `nyx__...` meta-tools — that's the trap behind issue [#298](https://github.com/ChronoAIProject/NyxID/issues/298). The flow below does both steps in one paste; the manual paths in [docs/CONNECTING_SERVICES.md](docs/CONNECTING_SERVICES.md) make the connect-and-verify step explicit.

The full walkthrough is at **[docs/CONNECTING_SERVICES.md](docs/CONNECTING_SERVICES.md)** — base-URL-agnostic, so the same guide works for hosted (`https://nyx.chrono-ai.fun`) and self-host (`http://localhost:3001`). It covers four paths in order of friction:

- **AI-driven (recommended)** — paste a prompt into Claude Code / Codex / Cursor and let your agent use NyxID's MCP meta-tools (`nyx__discover_services`, `nyx__connect_service`, `nyx__call_tool`) to add and verify your first service end-to-end.
- **CLI** — `nyxid service add llm-openai --credential-env OPENAI_API_KEY` then `nyxid proxy request llm-openai models` to verify.
- **Web UI** — dashboard click-through with a "Test request" verify step.
- **Direct API** — curl + `x-api-key` header for automation and CI.

Whichever path you pick, the verification step (calling a real downstream tool and getting a real response back) is the gate everything hinges on. The doc also has an "Adding more services later" section, so the same guide covers your tenth service the same way it covers your first.

For the AI-driven path: wire MCP first with `claude mcp add --transport http nyxid <BASE_URL>/mcp` (or `codex mcp add nyxid --url <BASE_URL>/mcp`, or Cursor's one-click install) — the first run opens your browser to authenticate. Then paste this into your AI agent:

> Help me connect an AI Service in NyxID. Use `nyx__discover_services` to list what's available in the catalog and ask me which one I want (e.g. OpenAI, Anthropic, GitHub). Once I pick, ask me for the credential I want to use, then call `nyx__connect_service` with the `service_id` from discover results and my credential. After it returns success, call `nyx__search_tools` to confirm the new service's tools are now exposed, then call `nyx__call_tool` on one of them (e.g. list models, list repos) to verify the proxy works end-to-end. Report back with the actual response so I know it's working — not just "looks good." If anything errors, tell me whether it's a credential problem or a service config problem.

<!-- Sync this prompt with docs/CONNECTING_SERVICES.md Path A on every release. -->

## Reach Local Services (Optional)

Services behind a firewall? Deploy a credential node to punch through NAT and expose them as MCP tools:

```bash
# Register and start a node (outbound WebSocket — no port forwarding, no VPN)
nyxid node register --token <reg-token> --url wss://<your-server>/api/v1/nodes/ws
nyxid node credentials add --service my-local-api --header Authorization --secret-format bearer
nyxid node start

# Register the service and link it to the node
nyxid node credentials setup --service my-local-api --api-url http://localhost:8080

# Import endpoints as MCP tools (if the service has an OpenAPI spec)
nyxid catalog endpoints my-local-api
```

## Resources

| Topic | Link | Description |
|-------|------|-------------|
| Connecting AI Services | [docs/CONNECTING_SERVICES.md](docs/CONNECTING_SERVICES.md) | Add your first (or Nth) AI Service — works for hosted + self-host |
| Quickstart (manual) | [docs/QUICKSTART.md](docs/QUICKSTART.md) | Step-by-step self-host + troubleshooting |
| Deployment | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | Start here for production setup |
| AI Agent Playbook | [docs/AI_AGENT_PLAYBOOK.md](docs/AI_AGENT_PLAYBOOK.md) | Start here for agent integration |
| Architecture | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design and data flows |
| API Reference | [docs/API.md](docs/API.md) | Full endpoint documentation |
| Credential Nodes | [docs/NODE_PROXY.md](docs/NODE_PROXY.md) | NAT traversal setup |
| MCP Integration | [docs/MCP_DELEGATION_FLOW.md](docs/MCP_DELEGATION_FLOW.md) | MCP protocol details |
| SSH Tunneling | [docs/SSH_TUNNELING.md](docs/SSH_TUNNELING.md) | Remote host access over WebSocket |
| Security | [docs/SECURITY.md](docs/SECURITY.md) | Threat model and hardening |
| Environment Variables | [docs/ENV.md](docs/ENV.md) | Full config reference |
| Developer Guide | [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md) | Local development setup |

## Contributing

We welcome contributions. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[Apache-2.0](LICENSE)
