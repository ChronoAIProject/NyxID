# CLI — Connect an AI Service from the Terminal

Three commands, one verification. End state: an `HTTP/1.1 200` response from your first proxied call.

For Web UI / AI-driven / Direct API, see the [hub](README.md).

## Prerequisites

The `nyxid` CLI installed and logged in. If you don't have it yet:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
source ~/.cargo/env 2>/dev/null || export PATH="$HOME/.cargo/bin:$PATH"
nyxid login --base-url <BASE_URL>
```

`<BASE_URL>` is `https://nyx-api.chrono-ai.fun` for hosted, `http://localhost:3001` for self-host.

`nyxid login` opens your browser and stores a session locally. The next two commands reuse it.

> **Windows users:** The examples below use bash syntax. In PowerShell, set environment variables with `$env:NAME="value"` and use backticks instead of `\` for line continuations. In CMD, use `set NAME=value` and `^` for line continuations. If you adapt any `curl` example, run `curl.exe` so PowerShell does not invoke its `curl` alias.

## Connect and verify

Set the **external service credential** in your shell first. This is your OpenAI / Anthropic / GitHub key, not a NyxID key. Substitute your real provider key:

```bash
export OPENAI_API_KEY=sk-...
```

In PowerShell, the same setup is:

```powershell
$env:OPENAI_API_KEY="sk-..."
```

Connect a service from the catalog:

```bash
nyxid service add llm-openai --credential-env OPENAI_API_KEY
```

Copy the service slug returned by the CLI. If `llm-openai` already exists, the created slug may be `llm-openai-2` or another suffixed value. Use that returned slug in the verification call:

```bash
nyxid proxy request <RETURNED_SERVICE_SLUG> models
```

PowerShell uses the same `nyxid` commands after the environment variable is set:

```powershell
nyxid service add llm-openai --credential-env OPENAI_API_KEY
nyxid proxy request <RETURNED_SERVICE_SLUG> models
```

If `proxy request` returns a real JSON response (not an auth error), you're done. The service is connected and the credential is good.

## Browse the catalog

To see what slugs are available before adding, run the commands you need. `catalog list` shows connectable services only; `catalog list --all` also includes system services; `catalog show` prints details and capabilities; `catalog endpoints` prints parsed OpenAPI endpoints.

```bash
nyxid catalog list
nyxid catalog list --all
nyxid catalog show llm-openai
nyxid catalog endpoints llm-openai
```

## Custom (non-catalog) services

For a private API the catalog doesn't know about — `--custom` plus an `--endpoint-url`:

```bash
nyxid service add my-internal-api \
  --custom \
  --endpoint-url https://internal.example.com \
  --credential-env MY_API_KEY \
  --auth-method bearer
```

For private APIs behind a firewall, see [docs/NODE_PROXY.md](../NODE_PROXY.md) for the credential node setup that punches through NAT.

## Rotating credentials

First find the service ID:

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
