export type AiTool = "cursor" | "claude-code" | "codex" | "openclaw" | "chatgpt";

export interface AiToolMeta {
  readonly id: AiTool;
  readonly name: string;
  readonly description: string;
  readonly configFileName: string;
  readonly configFilePath: string;
}

export const AI_TOOLS: readonly AiToolMeta[] = [
  {
    id: "cursor",
    name: "Cursor",
    description: "Native MCP support via project config",
    configFileName: "mcp.json",
    configFilePath: ".cursor/mcp.json",
  },
  {
    id: "claude-code",
    name: "Claude Code",
    description: "MCP via npx proxy command",
    configFileName: "settings.json",
    configFilePath: "~/.claude/settings.json",
  },
  {
    id: "codex",
    name: "Codex",
    description: "MCP via TOML config",
    configFileName: "config.toml",
    configFilePath: "~/.codex/config.toml",
  },
  {
    id: "openclaw",
    name: "OpenClaw",
    description: "Provider-level integration via system prompt",
    configFileName: "system-prompt",
    configFilePath: "",
  },
  {
    id: "chatgpt",
    name: "ChatGPT / Other",
    description: "Paste the llms.txt URL into your AI chat",
    configFileName: "llms.txt URL",
    configFilePath: "",
  },
] as const;

export interface AiToolConfigParams {
  readonly baseUrl: string;
  readonly mcpUrl: string;
  readonly clientId: string;
  readonly clientName: string;
  readonly redirectUris: readonly string[];
}

function cursorConfig(params: AiToolConfigParams): string {
  const config = {
    mcpServers: {
      nyxid: {
        url: params.mcpUrl,
      },
    },
  };
  return JSON.stringify(config, null, 2);
}

function claudeCodeConfig(params: AiToolConfigParams): string {
  const config = {
    mcpServers: {
      nyxid: {
        command: "npx",
        args: ["-y", "@anthropic-ai/mcp-proxy", params.mcpUrl],
        description: "NyxID MCP Proxy",
      },
    },
  };
  return JSON.stringify(config, null, 2);
}

function codexConfig(params: AiToolConfigParams): string {
  return `[mcp_servers.nyxid]\nurl = "${params.mcpUrl}"`;
}

function chatgptConfig(params: AiToolConfigParams): string {
  return [
    `# NyxID Integration Context`,
    ``,
    `Paste this message (or the URL below) into your AI chat to give it full context:`,
    ``,
    `  ${params.baseUrl}/llms-full.txt`,
    ``,
    `Or paste the short version:`,
    ``,
    `  ${params.baseUrl}/llms.txt`,
    ``,
    `## Your OAuth Client`,
    ``,
    `- Server: ${params.baseUrl}`,
    `- Client ID: ${params.clientId}`,
    `- Client Name: ${params.clientName}`,
    `- OIDC Discovery: ${params.baseUrl}/.well-known/openid-configuration`,
    ...(params.redirectUris.length > 0
      ? [`- Redirect URIs: ${params.redirectUris.join(", ")}`]
      : []),
    ``,
    `## Quick prompt`,
    ``,
    `> Read ${params.baseUrl}/llms-full.txt and help me integrate NyxID login`,
    `> into my app. My client_id is ${params.clientId}.`,
  ].join("\n");
}

function openclawConfig(params: AiToolConfigParams): string {
  return [
    `# NyxID + OpenClaw Integration`,
    ``,
    `NyxID provides credential proxying for OpenClaw.`,
    ``,
    `## Setup`,
    ``,
    `1. Install the CLI: \`bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"\``,
    `2. Log in: \`nyxid login --base-url ${params.baseUrl}\``,
    `3. Run: \`nyxid openclaw setup --url <OPENCLAW_GATEWAY_URL>\``,
    ``,
    `## Via Node Agent`,
    ``,
    `\`nyxid node openclaw connect --url <OPENCLAW_GATEWAY_URL>\``,
    ``,
    `## Full Guide`,
    ``,
    `See: ${params.baseUrl}/llms.txt (section: Using NyxID in OpenClaw)`,
  ].join("\n");
}

const GENERATORS: Record<AiTool, (params: AiToolConfigParams) => string> = {
  cursor: cursorConfig,
  "claude-code": claudeCodeConfig,
  codex: codexConfig,
  openclaw: openclawConfig,
  chatgpt: chatgptConfig,
};

export function generateToolConfig(
  tool: AiTool,
  params: AiToolConfigParams,
): string {
  return GENERATORS[tool](params);
}

// ---------------------------------------------------------------------------
// Setup prompts -- tool-specific instructions for installing persistent skills
// ---------------------------------------------------------------------------

export interface SetupPromptParams {
  readonly baseUrl: string;
  readonly dashboardUrl: string;
}

function toolSetupPrompt(
  tool: AiTool,
  p: SetupPromptParams,
): string {
  const installSh = `bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"`;
  const cliInstall = [
    `1. Install the NyxID CLI (handles Rust, build, and PATH automatically):`,
    `   ${installSh}`,
    `   Then open a new terminal and log in:`,
    `   nyxid login --base-url ${p.baseUrl}`,
  ];

  if (tool === "chatgpt") {
    return [
      `Read ${p.baseUrl}/llms.txt to understand what NyxID can do, then help me with whatever I need.`,
      `The NyxID server is at ${p.baseUrl} and the dashboard is at ${p.dashboardUrl}.`,
      `Use the nyxid CLI for all operations -- if it's not installed or needs updating, run:`,
      `bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"`,
      `(installs Rust if missing, builds the CLI, and configures PATH).`,
      `For secrets, always use --credential-env to read from environment variables.`,
      `Use --output json for machine-readable output.`,
    ].join(" ");
  }

  const toolFlag = tool === "claude-code" ? "claude-code" : tool;

  const lines = [
    `Install the NyxID skill so you have NyxID context in every future session:`,
    ``,
    ...cliInstall,
    ``,
    `2. Install the skill:`,
    `   nyxid ai-setup install --tool ${toolFlag}`,
    ``,
  ];

  const toolNotes: Record<string, string[]> = {
    "claude-code": [
      `This fetches the canonical NyxID skill from GitHub and the latest playbook`,
      `from ${p.baseUrl}/llms.txt, then installs:`,
      `  ~/.claude/skills/nyxid/SKILL.md                (concise skill, loaded on demand)`,
      `  ~/.claude/skills/nyxid/references/             (per-topic references, loaded when needed)`,
      `  ~/.claude/skills/nyxid/references/playbook.md  (full reference, loaded when needed)`,
      ``,
      `Claude loads the description at startup. Full content loads when you ask about NyxID.`,
      `Use /nyxid as a slash command, or just ask and it activates automatically.`,
    ],
    cursor: [
      `This fetches the NyxID skill from GitHub and installs it as a Cursor rule:`,
      `  .cursor/rules/nyxid.mdc (concise skill, always applied in this project)`,
      ``,
      `Note: Cursor rules are project-level. Run this command in each project.`,
    ],
    codex: [
      `This fetches the NyxID skill from GitHub and the latest playbook, then installs:`,
      `  ~/.codex/skills/nyxid/SKILL.md                (concise skill, loaded on demand)`,
      `  ~/.codex/skills/nyxid/references/             (per-topic references, loaded when needed)`,
      `  ~/.codex/skills/nyxid/references/playbook.md  (full reference, loaded when needed)`,
    ],
    openclaw: [
      `This fetches the NyxID skill from GitHub and installs into OpenClaw:`,
      `  ~/.openclaw/skills/nyxid/SKILL.md                (concise skill)`,
      `  ~/.openclaw/skills/nyxid/references/             (per-topic references)`,
      `  ~/.openclaw/skills/nyxid/references/playbook.md  (full reference, populated from <BASE_URL>/llms.txt)`,
      `  ~/.openclaw/skills/nyxid/scripts/install.sh      (CLI bootstrap installer)`,
      ``,
      `After install: start a new OpenClaw chat.`,
      `Optional: install the gateway as a background service with openclaw gateway install, then openclaw gateway start.`,
      `Verify: openclaw skills check (should show NyxID as ready).`,
    ],
  };

  lines.push(...(toolNotes[tool] ?? []));
  lines.push(
    ``,
    `To update the skill: nyxid ai-setup update --tool ${toolFlag}`,
    `To update the CLI itself, re-run the installer:`,
    `  ${installSh}`,
  );

  return lines.join("\n");
}

const SETUP_PROMPT_GENERATORS: Record<AiTool, (p: SetupPromptParams) => string> = {
  "claude-code": (p) => toolSetupPrompt("claude-code", p),
  cursor: (p) => toolSetupPrompt("cursor", p),
  codex: (p) => toolSetupPrompt("codex", p),
  openclaw: (p) => toolSetupPrompt("openclaw", p),
  chatgpt: (p) => toolSetupPrompt("chatgpt", p),
};

export function generateSetupPrompt(
  tool: AiTool,
  params: SetupPromptParams,
): string {
  return SETUP_PROMPT_GENERATORS[tool](params);
}

export type AiToolSkillType = "auto-refresh" | "manual-refresh" | "provider-level" | "paste-prompt";

export const AI_TOOL_SKILL_INFO: Record<AiTool, { type: AiToolSkillType; note: string }> = {
  "claude-code": {
    type: "manual-refresh",
    note: "Same SKILL.md from GitHub + playbook from server. Update skill: nyxid ai-setup update. Update CLI: re-run install.sh",
  },
  cursor: {
    type: "manual-refresh",
    note: "Concise skill as Cursor rule (project-level). Update skill: nyxid ai-setup update --tool cursor. Update CLI: re-run install.sh",
  },
  codex: {
    type: "manual-refresh",
    note: "Same SKILL.md from GitHub + playbook from server. Update skill: nyxid ai-setup update. Update CLI: re-run install.sh",
  },
  openclaw: {
    type: "manual-refresh",
    note: "Full skill bundle with tool scripts. Update skill: nyxid ai-setup update --tool openclaw. Update CLI: re-run install.sh",
  },
  chatgpt: {
    type: "paste-prompt",
    note: "Paste the prompt into each new chat session",
  },
};
