import { describe, it, expect } from "vitest";
import {
  AI_TOOLS,
  AI_TOOL_SKILL_INFO,
  generateToolConfig,
  generateSetupPrompt,
  type AiTool,
} from "./ai-tool-configs";

const params = {
  baseUrl: "https://auth.nyxid.dev",
  mcpUrl: "https://auth.nyxid.dev/mcp",
  clientId: "client-123",
  clientName: "My App",
  redirectUris: ["https://app.example.com/callback"],
};

const ALL_TOOLS: AiTool[] = ["cursor", "claude-code", "codex", "openclaw", "chatgpt"];

describe("AI_TOOLS / AI_TOOL_SKILL_INFO catalog", () => {
  it("lists every tool exactly once with a skill-info entry", () => {
    expect(AI_TOOLS.map((t) => t.id).sort()).toEqual([...ALL_TOOLS].sort());
    for (const tool of ALL_TOOLS) {
      expect(AI_TOOL_SKILL_INFO[tool]).toBeDefined();
      expect(AI_TOOL_SKILL_INFO[tool].note.length).toBeGreaterThan(0);
    }
  });
});

describe("generateToolConfig", () => {
  it("cursor emits valid JSON pointing mcpServers.nyxid.url at the MCP URL", () => {
    const parsed = JSON.parse(generateToolConfig("cursor", params));
    expect(parsed.mcpServers.nyxid.url).toBe(params.mcpUrl);
  });

  it("claude-code emits the npx mcp-proxy command with the MCP URL as an arg", () => {
    const parsed = JSON.parse(generateToolConfig("claude-code", params));
    expect(parsed.mcpServers.nyxid.command).toBe("npx");
    expect(parsed.mcpServers.nyxid.args).toContain(params.mcpUrl);
  });

  it("codex emits TOML with the server table and url", () => {
    const toml = generateToolConfig("codex", params);
    expect(toml).toContain("[mcp_servers.nyxid]");
    expect(toml).toContain(`url = "${params.mcpUrl}"`);
  });

  it("chatgpt includes the llms-full URL, client id, and a Redirect URIs line when URIs exist", () => {
    const text = generateToolConfig("chatgpt", params);
    expect(text).toContain(`${params.baseUrl}/llms-full.txt`);
    expect(text).toContain(params.clientId);
    expect(text).toContain(`Redirect URIs: ${params.redirectUris[0]}`);
  });

  it("chatgpt omits the Redirect URIs line when there are none", () => {
    const text = generateToolConfig("chatgpt", { ...params, redirectUris: [] });
    expect(text).not.toContain("Redirect URIs:");
  });

  it("openclaw references the gateway connect command and login base URL", () => {
    const text = generateToolConfig("openclaw", params);
    expect(text).toContain(`nyxid login --base-url ${params.baseUrl}`);
    expect(text).toContain("nyxid openclaw setup");
  });
});

describe("generateSetupPrompt", () => {
  const sp = { baseUrl: "https://auth.nyxid.dev", dashboardUrl: "https://auth.nyxid.dev/app" };

  it("chatgpt returns a single paste-prompt referencing llms.txt and the installer", () => {
    const text = generateSetupPrompt("chatgpt", sp);
    expect(text).toContain(`${sp.baseUrl}/llms.txt`);
    expect(text).toContain("install.sh");
    // chatgpt path is a one-paragraph join(" "), not the multi-line installer flow
    expect(text).not.toContain("nyxid ai-setup install --tool");
  });

  it("claude-code emits the install command with the claude-code flag and its skill paths", () => {
    const text = generateSetupPrompt("claude-code", sp);
    expect(text).toContain("nyxid ai-setup install --tool claude-code");
    expect(text).toContain("~/.claude/skills/nyxid/SKILL.md");
    expect(text).toContain(`nyxid login --base-url ${sp.baseUrl}`);
  });

  it("cursor notes the project-level rule install", () => {
    const text = generateSetupPrompt("cursor", sp);
    expect(text).toContain("nyxid ai-setup install --tool cursor");
    expect(text).toContain(".cursor/rules/nyxid.mdc");
  });

  it("codex and openclaw each include their own skill install paths", () => {
    expect(generateSetupPrompt("codex", sp)).toContain("~/.codex/skills/nyxid/SKILL.md");
    expect(generateSetupPrompt("openclaw", sp)).toContain(
      "~/.openclaw/skills/nyxid/SKILL.md",
    );
  });
});
