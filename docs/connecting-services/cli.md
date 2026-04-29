# CLI — Connect an AI Service from the Terminal

Three commands, one verification. End state: an `HTTP/1.1 200` response from your first proxied call.

For Web UI / AI-driven / Direct API, see the [hub](README.md).

## Prerequisites

The `nyxid` CLI installed and logged in. If you don't have it yet:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
source ~/.cargo/env
nyxid login --base-url <BASE_URL>
```

`<BASE_URL>` is `https://nyx.chrono-ai.fun` for hosted, `http://localhost:3001` for self-host.

`nyxid login` opens your browser and stores a session locally. The next two commands reuse it.

## Connect and verify

Set the **external service credential** in your shell first (this is your OpenAI / Anthropic / GitHub key, not a NyxID key):

```bash
export OPENAI_API_KEY=sk-...   # or your real provider key
```

Then:

```bash
# 1. Connect a service from the catalog.
nyxid service add llm-openai --credential-env OPENAI_API_KEY

# 2. Verify the proxy works end-to-end. Should return a real OpenAI models response.
nyxid proxy request llm-openai models
```

If `proxy request` returns a real JSON response (not an auth error), you're done. The service is connected and the credential is good.

## Browse the catalog

To see what slugs are available before adding:

```bash
nyxid catalog list                # connectable services only
nyxid catalog list --all          # include system services
nyxid catalog show llm-openai     # details + capabilities
nyxid catalog endpoints llm-openai   # parsed OpenAPI endpoints
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

```bash
nyxid service list                                                # find the service ID
nyxid service rotate-credential <id> --credential-env <NEW_VAR>
```

## Next

- **Wire your AI agent to NyxID's MCP endpoint:** see [ai-driven.md](ai-driven.md).
- **Same flow without the CLI dependency** (n8n, Zapier, CI/CD): see [direct-api.md](direct-api.md).
