# Connecting AI Services

Connect a downstream API (OpenAI, Anthropic, GitHub, your private API) to NyxID so your AI agents can call it through the proxy without ever seeing the raw credential.

**The deliverable:** an `HTTP/1.1 200` response from your first proxied request.

This guide works for **hosted** (`https://nyx.chrono-ai.fun`) and **self-host** (`http://localhost:3001`). It works for your first service and your tenth.

## Two kinds of credential — don't mix them up

| Term | What it is | Where it lives |
|---|---|---|
| **External service credential** | The real third-party API key (e.g. an OpenAI `sk-...` key, a GitHub PAT). NyxID stores this encrypted and never returns it to your agent. | Pasted once when you add a service (**External Services** tab). |
| **NyxID Agent Key** (`nyx_...`) | A scoped key your terminal or agent uses to call NyxID. NyxID injects the external credential server-side. | Created in **AI Services → Agent Keys → Create API Key**, with the `proxy` scope. Used in `X-API-Key` headers. |

## Pick your path

| Path | When to use | Time to first 200 |
|---|---|---|
| **[Web UI](web-ui.md)** *(default)* | First time using NyxID, or any time you want a click-through. | ~2 min |
| [CLI](cli.md) | You already use `nyxid` for automation. | ~1 min |
| [AI-driven](ai-driven.md) | You want Claude Code / Codex / Cursor to do it via NyxID's MCP meta-tools. | ~3 min, plus MCP setup |
| [Direct API](direct-api.md) | n8n, Zapier, CI/CD, custom code — anywhere a CLI dependency is awkward. | ~1 min |

If you're unsure, **[start with Web UI](web-ui.md)**. The other paths are equivalent under the hood.

## Did it work?

After any path completes, you should be able to make a real downstream call through NyxID's proxy and get a real response back, not an auth error.

For the Web UI and CLI paths, the path itself runs the verification call. If you're using AI-driven or Direct API, the verification is the explicit final step in those guides.

If you got a 401, 403, or 5xx from the proxy:

- **401 from the downstream service:** the external credential is wrong. Re-add it from the service's detail page.
- **401 from NyxID (`Missing API key` / `Invalid API key`):** you didn't replace the `nyx_...` placeholder in the copied example with your real Agent Key.
- **403 from NyxID:** your Agent Key is missing the `proxy` scope (required for `/api/v1/proxy/...`), or the key has an `allowed_service_ids` restriction that excludes this service. Edit the key under **AI Services → Agent Keys → \[your key\]**.
- **5xx from NyxID:** check `docker logs nyxid-backend` (self-host) or the status page (hosted).
- **MCP client only shows `nyx__...` tools and nothing real:** wiring MCP doesn't connect a downstream service by itself. Run the [Web UI](web-ui.md) walkthrough first, then reconnect MCP.

## Adding more services later

Same flow, different service. From the Web UI, **AI Services → Add Service** any time. From the CLI, `nyxid service add <slug> --credential-env <ENV_VAR>`. The other paths work identically for service N as for service 1.

## Connecting custom (non-catalog) services

For private APIs the catalog doesn't know about, see the **Custom services** section at the bottom of [cli.md](cli.md#custom-non-catalog-services). For services behind a firewall, see [docs/NODE_PROXY.md](../NODE_PROXY.md).

## Related

- [docs/QUICKSTART.md](../QUICKSTART.md) — self-host setup
- [docs/MCP_DELEGATION_FLOW.md](../MCP_DELEGATION_FLOW.md) — MCP auth + delegation under the hood
- [docs/AI_AGENT_PLAYBOOK.md](../AI_AGENT_PLAYBOOK.md) — patterns for using NyxID from agent code
- [docs/API.md](../API.md) — full REST endpoint reference
