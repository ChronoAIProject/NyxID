# CLI — Connect an AI Service from the Terminal

Four numbered steps. End state: an `HTTP/1.1 200` response from your first proxied call.

For Web UI / AI-driven / Direct API, see the [hub](README.md).

## Prerequisites

The `nyxid` CLI installed and logged in. If you don't have it yet:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
source ~/.cargo/env 2>/dev/null || export PATH="$HOME/.cargo/bin:$PATH"
nyxid login --base-url <BASE_URL>
```

`<BASE_URL>` is `https://nyx-api.chrono-ai.fun` for hosted, `http://localhost:3001` for self-host.

`nyxid login` opens your browser and stores a session locally. The steps below reuse it.

<details>
<summary><strong>Windows / native PowerShell</strong></summary>

The bash one-liner above runs as-is in [Git Bash](https://gitforwindows.org/) / [WSL](https://learn.microsoft.com/en-us/windows/wsl/install). For native PowerShell, install the CLI via [cargo](../QUICKSTART_POWERSHELL.md#optional-install-the-nyxid-cli), then:

```powershell
nyxid login --base-url https://nyx-api.chrono-ai.fun  # or http://localhost:3001 for self-host
```

</details>

## Connect and verify

Substitute your real OpenAI / Anthropic / GitHub key for `sk-...` below — this is your **external service credential**, not a NyxID key.

### Step 1 — Set the provider credential

```bash
export OPENAI_API_KEY=sk-...
```

### Step 2 — Add the service from the catalog

```bash
nyxid service add llm-openai --credential-env OPENAI_API_KEY
```

### Step 3 — Copy the returned slug

The CLI prints a `Slug:` line in its output. If `llm-openai` already existed on your account, the new entry may be suffixed (e.g. `llm-openai-2`). **Use that exact slug in Step 4** — it is the only handle that addresses your specific service instance.

### Step 4 — Verify with a proxied request

```bash
nyxid proxy request <RETURNED_SERVICE_SLUG> models
```

Success looks like an `HTTP/1.1 200` response carrying a real provider JSON body. For OpenAI's `models` endpoint that is `{"object":"list","data":[{"id":"gpt-...","object":"model",...}, ...]}`. If you see `401`, `403`, `5xx`, or an HTML error page instead, see [Did it work?](README.md#did-it-work) in the hub.

<details>
<summary><strong>Windows / native PowerShell</strong></summary>

```powershell
$env:OPENAI_API_KEY = "sk-..."
nyxid service add llm-openai --credential-env OPENAI_API_KEY
nyxid proxy request <RETURNED_SERVICE_SLUG> models
```

Replace `<RETURNED_SERVICE_SLUG>` with the slug `nyxid service add` prints.

</details>

You're done with the required path. The sections below are **optional**, **advanced**, or **maintenance** — skip them unless you need them.

## Optional — Browse the catalog

To preview what slugs are available before adding a service:

- `nyxid catalog list` — connectable services only.
- `nyxid catalog list --all` — also includes system services.
- `nyxid catalog show <slug>` — details and capabilities for one service.
- `nyxid catalog endpoints <slug>` — parsed OpenAPI endpoints for one service.

```bash
nyxid catalog list
nyxid catalog list --all
nyxid catalog show llm-openai
nyxid catalog endpoints llm-openai
```

## Advanced — Custom (non-catalog) services

For a private API the catalog doesn't know about, pass `--custom` plus an `--endpoint-url`:

```bash
nyxid service add my-internal-api \
  --custom \
  --endpoint-url https://internal.example.com \
  --credential-env MY_API_KEY \
  --auth-method bearer
```

For private APIs behind a firewall, see [docs/NODE_PROXY.md](../NODE_PROXY.md) for the credential node setup that punches through NAT.

## Maintenance — Rotate credentials

Find the service ID:

```bash
nyxid service list
```

Then rotate the credential:

```bash
nyxid service rotate-credential <id> --credential-env <NEW_VAR>
```

## Next

- **Wire your AI agent to NyxID's MCP endpoint:** see [ai-driven.md](ai-driven.md).
- **Same flow without the CLI dependency** (n8n, Zapier, CI/CD): see [direct-api.md](direct-api.md).
