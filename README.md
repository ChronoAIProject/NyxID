[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![GitHub Stars](https://img.shields.io/github/stars/ChronoAIProject/NyxID)](https://github.com/ChronoAIProject/NyxID)
[![Discord](https://img.shields.io/discord/1422500823925264438?logo=discord&logoColor=white&label=Discord&color=5865F2)](https://discord.gg/QMvcs8UQBW)
[![nyxid MCP server](https://glama.ai/mcp/servers/ChronoAIProject/nyxid/badges/score.svg)](https://glama.ai/mcp/servers/ChronoAIProject/nyxid)

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

Administrators can configure shared credentials, access, endpoint rules, and
billing from either services or linked providers. See [Service configuration and
vendor retirement](docs/SERVICE_CONFIGURATION.md). They can also give a role read-only
access to platform-wide usage reports, for example with the seeded **Usage
Viewer** role, without granting other admin access. See [Admin usage](docs/BILLING_UI_GLOSSARY.md#admin-usage).
**Usage → Dashboard / List** share top-level user, billing-owner organization,
service, and time filters. Dashboard charts offer **Group by → Model** to inspect
recorded model usage within a service. See [billing analytics](docs/BILLING_ANALYTICS.md#insight-controls-and-valid-data)
for grouping, missing models, and filter semantics.
Admins, operators and Usage Viewers can open **Usage → NyxBot** for cost and billing, token
traffic, and top billing accounts split by model, alongside active-session
counts, turn outcomes, and reply feedback. See [NyxBot model usage](docs/BILLING_ANALYTICS.md#nyxbot-model-usage)
for exact values, attribution, and historical coverage, and
[NyxBot activity](docs/chat/08-nyxagent-engine.md#admin-nyxbot-activity) for activity definitions.

Feature flag access can be delegated to any user or group with
`feature-flag-operator` (create flags and manage their own) or
`feature-flag-admin` (manage all flags, including built-ins). Both roles use
Admin → Feature Flags. See [feature flag management](docs/API.md#feature-flag-roles-and-management).

## What NyxID Does

- **Allocate repeatable credits** — admin Credits supports exact one-time or recurring allocations, optional shared-total service ceilings, and reclaim of each allocation's unspent remainder. Existing free-unit allowances stay separate. New policy authoring is off until all billing writers are upgraded; see [credit allocation accounting and rollout](docs/BILLING_EXACT_ACCOUNTING.md#allocation-local-service-ceilings-and-reclaim).

- **Connect phone and speech services to NyxBot** — [service adapters](docs/NYXBOT_SERVICE_ADAPTERS.md) add Telnyx/Twilio inbound calls and SMS plus ElevenLabs Custom LLM, with declared capabilities, verified connections and specialist tools. The feature is disabled by default; shared-platform Telnyx voice is unavailable.

- **Manage Oracle pool ownership and retirement** — transfer a pool between your account and organizations you manage while preserving its identity, or delete an idle pool through the API and CLI. See [Oracle pool lifecycle](docs/ORACLE_RELAY.md#pool-ownership-and-deletion-unreleased-1527-1542) for credential renewal and coordinated rollout requirements. Available in the first tagged release containing these changes.

- **Speak to NyxBot and specialists** — voice calls start with saved account preferences and show live captions and audio activity in the composer. Change the next call’s defaults through Voice settings in the composer chevron. Dictation offers click-to-toggle or hold-to-talk capture, microphone selection, and transcript review before insertion into a draft. Platform audio requires enabled voice features, a configured provider and tariff, and sufficient funding; microphone access is requested when capture is activated. See [voice and dictation](docs/chat/11-voice.md#19-implemented-composer-voice-and-dictation).

- **Display source images in NyxBot** — supported Notion reads return image references that NyxBot can retain as encrypted chat attachments and deliver through supported channels. The shared retrieval tool rechecks source access; see [source resources and image display](docs/chat/10-source-resources.md).

- **Control NyxBot service use** — NyxBot enforces saved connection order, prefers suitable working personal connections, and asks before organization use in Auto and Ask every time. Choose an exact connection for the next message without changing permission modes. See [NyxBot service execution](docs/NYXBOT_SERVICE_EXECUTION.md).

- **Spend NyxID credits on Telnyx tools** — the [Telnyx platform offering](docs/TELNYX_PLATFORM_BILLING.md) supports a restricted pilot of one model's nonstreaming text chat through HTTP or MCP, including NyxBot tool calls. Administrators must configure and enable the shared credential, verified model bounds, prices, billing, and pilot access before use; users do not connect Telnyx OAuth.
- **Connect Stripe to agents** — [Stripe Apps OAuth](docs/STRIPE_OAUTH.md) supports a managed account connection and read-only customer, invoice, payment, and subscription tools.

- **Connect email to agents** — [Aurinko Email](docs/AURINKO_INTEGRATION.md) provides an AI Service for mailbox API/MCP operations and an email channel bot for signed incoming notifications and replies. Connect with an account token; the channel additionally uses the application's signing secret.
- **Create a Telegram channel bot** — [Telegram New](docs/TELEGRAM_NEW.md) uses a platform-managed creation flow so customers can create and connect a bot without copying a token. The existing Telegram option still connects bots using their current tokens. Administrators configure a dedicated manager before enabling Telegram New.
- **Use BotFather as your Telegram assistant** — linked managers deliver full replies in ordered messages and retain forwarded text, quotes, links and attachments. Pair with a copyable command or Telegram return link, and edit shared FAQ/use-case libraries with preview, publication history and rollback in Admin → Platform Bots. Multiple managers can share one library. See [BotFather setup and gateway rollout](docs/TELEGRAM_NEW.md).
- **Mention channel specialists** — Exact native Telegram and Lark/Feishu mentions reach the receiving bot's linked agent in an enabled group without participant OAuth. Bot-authored messages remain guests with durable handoff limits. See [channel admission and gateway rollout](docs/CHANNEL_BOT_RELAY.md#callback-contract).
- **Reach anything** — public APIs, internal APIs, localhost services via credential nodes (`nyxid node`). SSH (Secure Shell) tunneling (`nyxid ssh`) reaches remote hosts. No VPN (Virtual Private Network), no port forwarding.
- **Never expose keys** — the reverse proxy injects credentials automatically. Your agent talks to NyxID; NyxID talks to the API with the real key.
- **MCP auto-wrap** — REST APIs with OpenAPI specs become [MCP](https://modelcontextprotocol.io/) (Model Context Protocol) tools. `nyxid mcp config --tool cursor` generates the config. Works with Claude Code, Cursor, VS Code, and any MCP client.
- **Per-agent isolation** — each agent gets a scoped token. Agent A accesses Slack and Gmail. Agent B only accesses your internal API. Revoke any session without touching the underlying credentials.
- **Full identity layer** — OIDC (OpenID Connect) / OAuth 2.0 with PKCE (Proof Key for Code Exchange), RBAC (Role-Based Access Control), service accounts, transaction approval (Telegram + mobile push), LLM (Large Language Model) gateway for 7 providers.
- **Add service access when needed** — applications can request [incremental OAuth consent](docs/site/shared/concepts/oauth-oidc.md#adding-service-access-incrementally), showing new services separately while preserving existing access in refresh tokens and broker bindings.

## See It in Action

The web app checks for frontend updates every ten minutes. Assistant updates
apply on safe chat or view switches; every frontend build still pending after
15 minutes shows a refresh reminder, including on Nyxbot pages.
See [deployment behavior](docs/DEPLOYMENT.md#frontend).

The end-to-end loop is short: connect a service to NyxID once, then any AI agent pointed at your NyxID MCP endpoint can use it — without ever seeing the raw API key.

1. **Add a service** in the web console — paste your OpenAI (or Anthropic, GitHub, etc.) key once; NyxID stores it encrypted.
2. **Wire up your AI tool** — `claude mcp add --transport http --scope user nyxid http://localhost:3001/mcp` (or one-click install for Cursor in `Settings` → `MCP`).
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

## Getting Started

NyxID is used in two phases — install the `nyxid` CLI client, then pick a workflow. The CLI logs into either the hosted service or a self-hosted backend and can create the Agent Key your workflow needs.

### 1. Install NyxID

The default "install NyxID" path installs the lightweight `nyxid` CLI. It is user-scoped, needs no Docker, and does not start a backend server.

Just need the client? Install the CLI. Running your own backend? Self-host with Docker.

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

After the CLI is installed, choose where it should log in:

| | Hosted | Self-host |
|---|---|---|
| **What it is** | We run the NyxID backend for you in the cloud | You run the NyxID backend on your own machine |
| **Best for** | Getting started quickly, no server setup | Full control, private networks, offline use |
| **Status** | Open sign-up | Open — anyone can run it |

> **Driving NyxID from an AI coding agent?** Hand it this single line at any point — it tells the agent to install the `nyxid` CLI plus the Nyx skill files:
>
> ```
> Install nyx skills from https://github.com/ChronoAIProject/NyxID/blob/main/skills/INSTALL.md
> ```
>
> The agent reads [`skills/INSTALL.md`](skills/INSTALL.md) end-to-end. It should use the CLI installer by default and only run the Docker backend setup if you explicitly ask to self-host.

> **Prefer a plugin install?** The Claude Code and Codex plugins connect your agent to NyxID's hosted MCP server and bundle an MCP-first NyxID skill; they need no CLI. The last command opens NyxID sign-in. When you approve the app, choose the services it may use (or all services). Connect a service afterwards to expose its tools.
>
> **Claude Code**
>
> ```bash
> claude plugin marketplace add ChronoAIProject/NyxID
> claude plugin install nyxid@nyxid
> claude mcp login plugin:nyxid:nyxid
> ```
>
> **Codex**
>
> ```bash
> codex plugin marketplace add ChronoAIProject/NyxID
> codex plugin add nyxid@nyxid
> codex mcp login nyxid
> ```
>
> `claude mcp login` needs Claude Code 2.1.284 or later; on older versions run `/mcp` inside Claude Code to sign in. The same Claude marketplace also offers `nyxid-cli@nyxid`, the CLI edition that bundles every skill in `skills/` (it calls the `nyxid` CLI installed above). **Cursor**: see [`integrations/cursor-plugin/`](integrations/cursor-plugin/). Maintainers: see [`docs/PLUGINS.md`](docs/PLUGINS.md).

#### Hosted (Recommended)

Start using NyxID in under a minute — no Docker, no setup.

1. Open **[nyx.chrono-ai.fun/register](https://nyx.chrono-ai.fun/register)** in a new tab (Cmd/Ctrl-click, or right-click → Open Link in New Tab) so you can keep this checklist open.
2. Sign up with Google, GitHub, or Apple
3. Open `AI Services`, add and connect your first external service, and run the API Usage verification curl
4. After the service is verified, wire your AI tool to NyxID's MCP endpoint

The full click-through flow is in **[Add your first AI Service](docs/connecting-services/web-ui.md)**.

#### Self-Host

Run your own NyxID backend on your machine. This is the optional server-side path and sets up three Docker containers (database, backend, frontend) — takes about 2 minutes.

**Prerequisites:** [Docker](https://docs.docker.com/get-docker/) and a bash shell. macOS and Linux already have one — Windows users, see [docs/WINDOWS_SETUP.md](docs/WINDOWS_SETUP.md) before going further. Docker is required only for this backend path. Full prereqs and disk budgets are in [SETUP.md](docs/SETUP.md).

##### AI-Assisted (Recommended)

If you have Claude Code, Cursor, or any AI coding assistant open, paste the prompt below into it only when you explicitly want a self-hosted backend. It will drive the entire self-host flow for you — preflight, clone, env generation, Docker stack, health check, CLI login, first credential, and MCP config.

<details>
<summary><strong>Click to expand the full AI-assisted self-host prompt</strong></summary>

> I want to self-host the NyxID backend on this machine (the repo is https://github.com/ChronoAIProject/NyxID). This is the Docker server path, not the default CLI install. Walk me through the full setup interactively. If anything fails or I'd prefer to follow the manual steps myself, the full step-by-step with troubleshooting is at https://github.com/ChronoAIProject/NyxID/blob/main/docs/SETUP.md. If I'm on Windows, confirm I'm running from a WSL Ubuntu shell (not native PowerShell or CMD) before proceeding — see https://github.com/ChronoAIProject/NyxID/blob/main/docs/WINDOWS_SETUP.md.
> 1. Confirm Docker is installed and running before touching anything (check `git`, `docker`, `openssl`, `curl`, `docker compose` v2, and `docker info`).
> 2. **Before cloning or generating anything, check whether NyxID install STATE is present** — look for a `./NyxID/.env.dev` file OR any Docker volume matching `nyx*_mongodb_data` (run `docker volume ls --format '{{.Name}}' | grep -E 'nyx.*_mongodb_data$'` — this catches the default `nyxid_mongodb_data` plus any variant from a renamed checkout). A bare `./NyxID` directory alone does NOT count as "installed" — `uninstall.sh` leaves the source tree in place, so the directory can exist with no state. **If install state is present, stop and tell me the quickstart is a first-time-only install.** Ask whether I want to (a) uninstall first — if `./NyxID` exists, run `cd NyxID && ./scripts/uninstall.sh --yes && cd ..`; if only the stale Docker volume is orphaned (checkout was manually deleted earlier), run `docker volume ls --format '{{.Name}}' | grep -E 'nyx.*_mongodb_data$' | xargs -r docker volume rm` directly. Either path wipes the volume, containers, and (for the script path) `.env.dev`/keys — destroys all NyxID accounts and encrypted credentials. Or (b) keep my existing install and stop here — I can verify it's still running with `curl -sf http://localhost:3001/health`. Do not proceed to step 3 until I answer.
> 3. If `./NyxID` already exists (post-uninstall reinstall), `cd` into it; otherwise clone the repo into the current directory and `cd` in. Generate `.env.dev` with a fresh `ENCRYPTION_KEY` and `MONGO_ROOT_PASSWORD` (set `ENVIRONMENT=development`, `AUTO_VERIFY_EMAIL=true`, and `EMAIL_AUTH_ENABLED=true` so I can complete email verification quickly; after initial admin setup, disable the `auth:invitation-code` feature flag under Admin > Feature Flags to open signup), symlink it to `.env.production`, create the PKCS#1 JWT signing keys under `keys/` (with a LibreSSL fallback using `-pubout` if `-RSAPublicKey_out` isn't supported), then pull images and start the stack with `docker compose -f docker-compose.yml -f docker-compose.prod.yml --env-file .env.production up -d`. Wait up to 90 seconds for `http://localhost:3001/health` to return 200 — if it times out, tell me to run `docker logs nyxid-backend`. If the logs show `SCRAM failure: Authentication failed`, that means the MongoDB volume has a stale password from a previous install — tell me to run `./scripts/uninstall.sh --yes` (or, if the checkout is gone, `docker volume ls --format '{{.Name}}' | grep -E 'nyx.*_mongodb_data$' | xargs -r docker volume rm` to remove any nyx-flavored orphan volume) and retry. Show me the generated `ENCRYPTION_KEY` so I can back it up.
> 4. Tell me to open http://localhost:3000 and register my account (no email verification needed — accounts are auto-verified in dev mode), and wait until I confirm I've done that.
> 5. Ensure the `nyxid` CLI is available. If it is not installed, ask me whether I want to install the `nyxid` CLI plus the Nyx skill so you can drive NyxID from the terminal afterwards. Explain that the installer downloads a roughly 10 MB prebuilt binary plus a small set of skill files (no Rust toolchain required), installs the CLI into a versioned layout with rollback support, and that only unsupported OS/arch combinations fall back to a Rust source build. If I say yes, follow the install manifest at https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/INSTALL.md end-to-end (it installs the CLI under `~/.local/share/nyxid/`, drops the skill into your skill directory, and tells you to `export PATH="$HOME/.local/bin:$PATH"` if needed). Then verify with `nyxid doctor`, log me in with `nyxid login --base-url http://localhost:3001`, add my OpenAI key with `nyxid service add llm-openai --credential-env OPENAI_API_KEY`, and verify with `nyxid proxy request <slug> models` using the slug the previous `service add` command printed under `Slug:` (typically `llm-openai`, but suffixed if I already had a service with that slug). If I do not want the CLI, walk me through adding the same OpenAI credential in the web console instead.
> 6. Finish by connecting my AI tool to NyxID's MCP endpoint at `http://localhost:3001/mcp`. For Claude Code: `claude mcp add --transport http --scope user nyxid http://localhost:3001/mcp`. For Codex: `codex mcp add nyxid --url http://localhost:3001/mcp`. For Cursor: open `Settings` > `MCP` in the web console and click `Install to Cursor`.

</details>

<!-- AI setup-prompt maintenance: validate this prompt against actual CLI + web console on each release -->

##### Manual Setup

Prefer to run the optional backend setup yourself, or need the full troubleshooting guide? Follow **[docs/SETUP.md](docs/SETUP.md)** (macOS, Linux, or Windows via WSL).

It covers:

- System preflight check — [Step 1](docs/SETUP.md#step-1-of-3--check-your-system)
- One paste-block install — [Step 2](docs/SETUP.md#step-2-of-3--install-and-start)
- Register your account — [Step 3](docs/SETUP.md#step-3-of-3--register-and-connect)
- [CLI install or verification](docs/SETUP.md#install-or-verify-the-nyxid-cli)
- [Uninstall & reinstall](docs/SETUP.md#uninstall--reinstall), [orphan volume recovery](docs/SETUP.md#recovering-an-orphan-volume), and [SCRAM failure](docs/SETUP.md#stuck-on-scram-failure) troubleshooting

Once NyxID is running and you've registered at `http://localhost:3000`, continue to [2. Pick a workflow](#2-pick-a-workflow).

For production deployment (TLS, custom domain, email verification), see [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

### 2. Pick a workflow

With a NyxID account, pick the workflow that matches what you want to build and use the identity its guide requires. Each is a step-by-step procedure that ends with a working integration; the guides are independent and can be completed in any order.

| Quickstart | Outcome | NyxID capability |
|---|---|---|
| [**Run NyxBot from your terminal**](docs/site/cli/reference/nyxbot.md) | Submit prompts from text, files, or stdin; inspect, wait for, and cancel durable work using a first-party owner login. The CLI runs locally and uses your hosted or self-hosted NyxID instance. | Owner CLI and invocation API; requires server admission enabled |
| [**n8n: Daily AI News Digest with One NyxID Credential**](docs/quickstarts/n8n.md) | An n8n workflow pulls an RSS feed, summarizes each article with Gemini, and posts to Telegram — using one `Header Auth` credential in n8n while NyxID stores the upstream Gemini and Telegram secrets. | Per-service credential injection |
| [**Per-Agent Keys for Claude Code and Codex**](docs/quickstarts/claude-code.md) | Two coding agents on one machine, each scoped to a distinct service and credential, attributed independently in the audit log. | Agent isolation, scoped Agent Keys |
| [**Reach a Localhost API from a Cloud-Hosted Agent**](docs/quickstarts/node-proxy.md) | A private-host API is reachable from a cloud agent without VPN, port forwarding, or a tunneling service. | Credential Node, outbound-only NAT traversal |
| [**Wrap a REST API as MCP Tools**](docs/quickstarts/mcp-wrapping.md) | An OpenAPI spec is exposed as typed MCP tools to Claude Code / Cursor / VS Code / Codex with no MCP server code. | OpenAPI → MCP auto-wrap |
| [**Use Supabase as an AI Agent Database**](docs/quickstarts/supabase.md) | A Supabase project becomes a credential-brokered PostgREST database for scoped AI agents. | Per-user endpoint + encrypted API key |
| [**Connect Cloudflare, Supabase Management, or Railway**](docs/CLOUD_PLATFORM_OAUTH.md) | Connect a cloud account through a shared or BYO OAuth app for scoped management API access. | PKCE OAuth, encrypted tokens, and refresh |

> For a per-interface reference walkthrough that ends with `HTTP/1.1 200` from your first proxied call (Web UI · CLI · AI-driven · Direct API), see the [Connecting AI Services hub](docs/connecting-services/).

## Connecting AI Services (interface reference)

[Pick a workflow](#2-pick-a-workflow) in Getting Started is organized by use case. For an interface-oriented reference that ends with a verified proxy call (`HTTP/1.1 200`) using your preferred entry point — Web UI, CLI, AI-driven (MCP), or Direct API — see **[docs/connecting-services/](docs/connecting-services/)**. The hub distinguishes external service credentials from NyxID Agent Keys and links to one walkthrough per interface.

## Resources

| Topic | Link | Description |
|-------|------|-------------|
| Quickstarts | [docs/quickstarts/](docs/quickstarts/) | End-to-end recipes — n8n, per-agent keys, node proxy, MCP wrapping |
| Connecting AI Services | [docs/connecting-services/](docs/connecting-services/) | Add your first (or Nth) AI Service — Web UI / CLI / AI-driven / Direct API |
| Setup | [docs/SETUP.md](docs/SETUP.md) | Optional self-hosted backend + troubleshooting (macOS, Linux, Windows via WSL) |
| Nyxbot Onboarding | [docs/NYXBOT_ONBOARDING.md](docs/NYXBOT_ONBOARDING.md) | Onboarding entry, existing integrations, and pending backend contracts |
| NyxBot CLI and API | [docs/chat/09-owner-cli-api.md](docs/chat/09-owner-cli-api.md) | Owner commands, invocation lifecycle, security boundaries, and rollout |
| Deployment | [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | Start here for production setup |
| AI Agent Playbook | [docs/AI_AGENT_PLAYBOOK.md](docs/AI_AGENT_PLAYBOOK.md) | Start here for agent integration |
| Architecture | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design and data flows |
| API Reference | [docs/API.md](docs/API.md) | Full endpoint documentation |
| Credential Nodes | [docs/NODE_PROXY.md](docs/NODE_PROXY.md) | NAT traversal setup |
| MCP Integration | [docs/MCP_DELEGATION_FLOW.md](docs/MCP_DELEGATION_FLOW.md) | MCP protocol details |
| SSH Tunneling | [docs/SSH_TUNNELING.md](docs/SSH_TUNNELING.md) | Remote host access over WebSocket |
| Security | [docs/SECURITY.md](docs/SECURITY.md) | Threat model and hardening |
| Environment Variables | [docs/ENV.md](docs/ENV.md) | Full config reference |
| Telemetry | [docs/TELEMETRY.md](docs/TELEMETRY.md) | Opt-in usage analytics — hot-swap contract, event taxonomy, consent + GDPR erasure |
| Developer Guide | [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md) | Local development setup |

## Contributing

We welcome contributions. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[Apache-2.0](LICENSE)
