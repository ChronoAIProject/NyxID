import { describe, expect, it } from "vitest";
import type { ReactElement } from "react";
import { Monitor, Smartphone, Globe } from "lucide-react";
import {
  getDeviceIcon,
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
