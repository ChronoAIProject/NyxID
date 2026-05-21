import { Monitor, Smartphone, Globe } from "lucide-react";

export function getDeviceIcon(userAgent: string | null | undefined) {
  const ua = (userAgent ?? "").toLowerCase();
  if (
    ua.includes("mobile") ||
    ua.includes("android") ||
    ua.includes("iphone")
  ) {
    return <Smartphone className="h-4 w-4" aria-hidden="true" />;
  }
  if (
    ua.includes("mozilla") ||
    ua.includes("chrome") ||
    ua.includes("safari")
  ) {
    return <Monitor className="h-4 w-4" aria-hidden="true" />;
  }
  return <Globe className="h-4 w-4" aria-hidden="true" />;
}

// ---------------------------------------------------------------------------
// MCP Install helpers
// ---------------------------------------------------------------------------

export function buildCursorDeeplink(mcpUrl: string): string {
  const config = JSON.stringify({ url: mcpUrl });
  const encoded = encodeURIComponent(btoa(config));
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=nyxid&config=${encoded}`;
}

export function buildClaudeCodeCommand(mcpUrl: string): string {
  return `claude mcp add --transport http --scope user nyxid ${mcpUrl}`;
}

export function buildCursorConfig(mcpUrl: string): string {
  return JSON.stringify({ mcpServers: { nyxid: { url: mcpUrl } } }, null, 2);
}

export function buildClaudeCodeConfig(mcpUrl: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        nyxid: {
          type: "http",
          url: mcpUrl,
        },
      },
    },
    null,
    2,
  );
}

export function buildCodexCommand(mcpUrl: string): string {
  return `codex mcp add nyxid --url ${mcpUrl}`;
}

export function buildCodexConfig(mcpUrl: string): string {
  return `[mcp_servers.nyxid]\nurl = "${mcpUrl}"`;
}
