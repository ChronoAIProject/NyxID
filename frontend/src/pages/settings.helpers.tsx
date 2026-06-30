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

/**
 * Turn a raw `User-Agent` string into a short human label like
 * "Chrome 149 on macOS" or "Safari on iPhone" so the Sessions tab is
 * readable. Falls back to the raw UA (or "Unknown device" when blank).
 *
 * Intentionally simple — handles the common browsers/OSes most NyxID
 * users will see. Anything weirder shows the raw UA, which is better
 * than guessing wrong.
 */
export function humanizeUserAgent(
  userAgent: string | null | undefined,
): string {
  const ua = userAgent?.trim();
  if (!ua) return "Unknown device";

  // CLI / scripted clients first — usually the cleanest match.
  if (/^curl\//i.test(ua)) return `curl (${ua.split(" ")[0]})`;
  if (/^postmanruntime\//i.test(ua)) return "Postman";
  if (/python-requests\//i.test(ua)) return "python-requests";
  if (/^node\b|axios|fetch/i.test(ua) && !ua.includes("Mozilla")) {
    return "Node / fetch client";
  }

  // OS detection.
  let os: string | null = null;
  if (/iPad|iPhone|iPod/.test(ua)) os = "iOS";
  else if (/Android/.test(ua)) os = "Android";
  else if (/Mac OS X|Macintosh/.test(ua)) os = "macOS";
  else if (/Windows/.test(ua)) os = "Windows";
  else if (/Linux/.test(ua)) os = "Linux";

  // Browser detection — order matters (Edge before Chrome, Chrome before Safari).
  let browser: string | null = null;
  const edge = ua.match(/Edg\/(\d+)/);
  const chrome = ua.match(/Chrome\/(\d+)/);
  const firefox = ua.match(/Firefox\/(\d+)/);
  const safari = ua.match(/Version\/(\d+).*Safari\//);
  if (edge) browser = `Edge ${edge[1]}`;
  else if (chrome) browser = `Chrome ${chrome[1]}`;
  else if (firefox) browser = `Firefox ${firefox[1]}`;
  else if (safari) browser = `Safari ${safari[1]}`;

  if (browser && os) return `${browser} on ${os}`;
  if (browser) return browser;
  if (os) return `Browser on ${os}`;
  return ua.length > 60 ? `${ua.slice(0, 57)}…` : ua;
}

/**
 * Friendlier label for an IP address. Today it just collapses
 * `127.0.0.1` / `::1` to "Local network" so dev sessions don't look
 * like garbage. Future: country / city lookup via the backend.
 */
export function humanizeIpAddress(ip: string | null | undefined): string {
  const trimmed = ip?.trim();
  if (!trimmed) return "—";
  if (trimmed === "127.0.0.1" || trimmed === "::1") return "Local network";
  return trimmed;
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
