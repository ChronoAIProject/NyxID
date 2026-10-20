# NyxID for ChatGPT and Codex

NyxID is a credential broker. This plugin connects ChatGPT and Codex to NyxID's hosted MCP server so they can list the services you have connected, help you connect new ones, and call their APIs. Your API keys and OAuth tokens stay in NyxID; the assistant only sees the responses.

## What the plugin contains

| Component | Purpose |
| --- | --- |
| `plugin.json` | Manifest: listing metadata, icons, and default prompts for the OpenAI plugin directory and Codex. |
| `mcp.json` | Registers NyxID's remote MCP server, `https://nyx-api.chrono-ai.fun/mcp` (streamable HTTP). |
| `skills/nyxid` | Teaches the assistant the connect-then-call workflow and to never request secrets in chat. |

The plugin runs no local code, scripts, or hooks.

## Data and network disclosure

- The plugin talks only to `https://nyx-api.chrono-ai.fun`, NyxID's API, over HTTPS.
- Signing in uses OAuth 2.1 with PKCE in your browser at `https://nyx.chrono-ai.fun`. NyxID supports Google, GitHub, and Apple sign-in.
- When you connect a service, you enter that service's credentials on NyxID's or the provider's own website, never in the conversation. NyxID stores them encrypted.
- When the assistant calls a service tool, NyxID forwards the request to that third-party service with the stored credential and returns the response to the assistant. Only services available to your account are reachable: the ones you connected, services that need no credential, and services shared with you through an organization you belong to, subject to your grants and organization permissions.
- Accounts with those capabilities enabled also expose NyxID's SSH command and browser-relay ("Oracle") tools.

See the [privacy policy](https://nyx.chrono-ai.fun/privacy) and [terms](https://nyx.chrono-ai.fun/terms).

## Getting started

1. Install the plugin and approve the NyxID connection when prompted.
2. Sign in to NyxID with Google, GitHub, or Apple. A NyxID account is created on first sign-in.
3. Ask the assistant to connect a service, open the secure link it returns, and approve access there.

## Example prompts

- "What services do I have connected in NyxID?"
- "Connect my GitHub account to NyxID."
- "List my GitHub repositories."
- "What other services can I connect through NyxID?"

## Tools

`nyx__list_connected_services`, `nyx__discover_services`, `nyx__connect_service`, `nyx__wait_for_connection`, `nyx__search_tools`, and `nyx__call_tool` provide the core workflow. Connected services may add typed tools named after the service and operation. Every tool declares read-only, destructive, and open-world hints.

Disconnecting services and account changes happen in the NyxID web app, not through the assistant.

## Support

Report issues at <https://github.com/ChronoAIProject/NyxID/issues>.
