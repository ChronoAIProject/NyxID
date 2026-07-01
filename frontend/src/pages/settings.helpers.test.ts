import { describe, expect, it } from "vitest";
import type { ReactElement } from "react";
import { Monitor, Smartphone, Globe } from "lucide-react";
import {
  getDeviceIcon,
  humanizeUserAgent,
  humanizeIpAddress,
  buildCursorDeeplink,
  buildClaudeCodeCommand,
  buildCursorConfig,
  buildClaudeCodeConfig,
  buildCodexCommand,
  buildCodexConfig,
} from "./settings.helpers";

const MCP_URL = "https://auth.nyxid.dev/mcp";

describe("getDeviceIcon", () => {
  it("returns the Smartphone icon for mobile user-agents", () => {
    for (const ua of [
      "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)",
      "Mozilla/5.0 (Linux; Android 14; Pixel 8)",
      "Some Mobile Browser",
    ]) {
      const element = getDeviceIcon(ua) as ReactElement;
      expect(element.type).toBe(Smartphone);
    }
  });

  it("returns the Monitor icon for desktop browser user-agents", () => {
    for (const ua of [
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
      "Chrome/120.0.0.0 Safari/537.36",
    ]) {
      const element = getDeviceIcon(ua) as ReactElement;
      expect(element.type).toBe(Monitor);
    }
  });

  it("falls back to the Globe icon for unknown user-agents", () => {
    const element = getDeviceIcon("curl/8.4.0") as ReactElement;
    expect(element.type).toBe(Globe);
  });

  it("falls back to the Globe icon for null or undefined user-agents", () => {
    expect((getDeviceIcon(null) as ReactElement).type).toBe(Globe);
    expect((getDeviceIcon(undefined) as ReactElement).type).toBe(Globe);
    expect((getDeviceIcon("") as ReactElement).type).toBe(Globe);
  });

  it("prefers Smartphone over Monitor when both signals are present", () => {
    // A real mobile UA contains "Mozilla" (Monitor branch) but the mobile
    // branch must win because it is checked first.
    const ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile";
    const element = getDeviceIcon(ua) as ReactElement;
    expect(element.type).toBe(Smartphone);
  });
});

describe("humanizeUserAgent", () => {
  it("turns a Chrome-on-Mac UA into 'Chrome X on macOS'", () => {
    const ua =
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";
    expect(humanizeUserAgent(ua)).toBe("Chrome 149 on macOS");
  });

  it("recognises Safari on iPhone with the Version/X token", () => {
    const ua =
      "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
    expect(humanizeUserAgent(ua)).toBe("Safari 17 on iOS");
  });

  it("recognises Firefox on Windows", () => {
    expect(
      humanizeUserAgent(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:120.0) Gecko/20100101 Firefox/120.0",
      ),
    ).toBe("Firefox 120 on Windows");
  });

  it("collapses curl into a short label", () => {
    expect(humanizeUserAgent("curl/8.7.1")).toBe("curl (curl/8.7.1)");
  });

  it("returns 'Unknown device' for empty / nullish input", () => {
    expect(humanizeUserAgent(undefined)).toBe("Unknown device");
    expect(humanizeUserAgent(null)).toBe("Unknown device");
    expect(humanizeUserAgent("  ")).toBe("Unknown device");
  });

  it("falls back to the truncated raw UA when no browser+OS match", () => {
    const weird = "SomeRandomBrowser/1.0 (CustomOS)";
    expect(humanizeUserAgent(weird)).toBe(weird);
  });
});

describe("humanizeIpAddress", () => {
  it("turns loopback addresses into 'Local network'", () => {
    expect(humanizeIpAddress("127.0.0.1")).toBe("Local network");
    expect(humanizeIpAddress("::1")).toBe("Local network");
  });

  it("passes through real IPv4 / IPv6 addresses unchanged", () => {
    expect(humanizeIpAddress("203.0.113.42")).toBe("203.0.113.42");
    expect(humanizeIpAddress("2001:db8::1")).toBe("2001:db8::1");
  });

  it("returns the em-dash placeholder for empty / nullish input", () => {
    expect(humanizeIpAddress(undefined)).toBe("—");
    expect(humanizeIpAddress(null)).toBe("—");
    expect(humanizeIpAddress("  ")).toBe("—");
  });
});

describe("buildCursorDeeplink", () => {
  it("encodes the mcpUrl into a base64 url-encoded cursor deeplink", () => {
    const deeplink = buildCursorDeeplink(MCP_URL);
    expect(deeplink).toMatch(
      /^cursor:\/\/anysphere\.cursor-deeplink\/mcp\/install\?name=nyxid&config=/,
    );

    const encoded = deeplink.split("config=")[1] ?? "";
    const decoded = JSON.parse(atob(decodeURIComponent(encoded)));
    expect(decoded).toEqual({ url: MCP_URL });
  });
});

describe("buildClaudeCodeCommand", () => {
  it("produces the claude mcp add command with http transport and the url", () => {
    expect(buildClaudeCodeCommand(MCP_URL)).toBe(
      `claude mcp add --transport http --scope user nyxid ${MCP_URL}`,
    );
  });
});

describe("buildCursorConfig", () => {
  it("produces valid JSON with the mcpUrl under mcpServers.nyxid.url", () => {
    const config = buildCursorConfig(MCP_URL);
    const parsed = JSON.parse(config);
    expect(parsed).toEqual({ mcpServers: { nyxid: { url: MCP_URL } } });
    // Pretty-printed with 2-space indentation.
    expect(config).toContain("\n  ");
  });
});

describe("buildClaudeCodeConfig", () => {
  it("produces valid JSON with http type and the mcpUrl", () => {
    const config = buildClaudeCodeConfig(MCP_URL);
    const parsed = JSON.parse(config);
    expect(parsed).toEqual({
      mcpServers: { nyxid: { type: "http", url: MCP_URL } },
    });
    expect(parsed.mcpServers.nyxid.type).toBe("http");
  });
});

describe("buildCodexCommand", () => {
  it("produces the codex mcp add command with the url flag", () => {
    expect(buildCodexCommand(MCP_URL)).toBe(
      `codex mcp add nyxid --url ${MCP_URL}`,
    );
  });
});

describe("buildCodexConfig", () => {
  it("produces a TOML table for the nyxid mcp server containing the url", () => {
    const config = buildCodexConfig(MCP_URL);
    expect(config).toBe(`[mcp_servers.nyxid]\nurl = "${MCP_URL}"`);
    expect(config).toContain("[mcp_servers.nyxid]");
    expect(config).toContain(`url = "${MCP_URL}"`);
  });
});
