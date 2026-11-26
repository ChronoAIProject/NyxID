import { getPublicOrigin } from "./public-origin.mjs";
import assert from "node:assert/strict";
import { unified } from "unified";
import remarkParse from "remark-parse";

// Independent CommonMark parser and manually selected published-content
// obligations: this does not compare the converter with its own output.
export function verifyMarkdownContract(path, markdown, origin = getPublicOrigin()) {
  const ast = unified()
    .use(remarkParse)
    .parse(markdown.replace(/^---\n[\s\S]*?\n---\n/, ""));
  const nodes = [];
  function visit(node) {
    nodes.push(node);
    node.children?.forEach(visit);
  }
  visit(ast);
  const code = nodes
    .filter((node) => node.type === "code")
    .map((node) => node.value);
  if (path === "/agents" || path === "/zh/agents") {
    const groups = [
      "/api/v1/proxy/s/llm-openai/models",
      "/api/v1/llm/gateway/v1/chat/completions",
      "nyxid mcp config",
      "nyxid api-key create",
    ];
    assert.equal(
      new Set(
        groups.map((marker) =>
          code.findIndex((value) => value.includes(marker)),
        ),
      ).size,
      4,
      `${path}: code examples must remain separate`,
    );
  }
  if (path === "/zh") {
    assert.notEqual(
      code.findIndex((value) => value.includes('"mcpServers"')),
      code.findIndex((value) => value.includes("curl -X POST")),
      `${path}: MCP and curl examples must remain separate`,
    );
    const bodies =
      path === "/zh"
        ? [
            "在 Telegram 和 X 上全天候回复客户",
            "一句话连接 Gmail、日历、X 等服务",
            "通过 MCP 和 REST 接入 Claude、Cursor、Codex 或你自己的代码",
          ]
        : [
            "Answers customers on Telegram and X, day and night",
            "Connect Gmail, Calendar, X and more in one message",
            "MCP and REST for Claude, Cursor, Codex or your own code",
          ];
    for (const body of bodies)
      assert.ok(
        markdown.includes(body),
        `${path}: missing audience body ${body}`,
      );
  }
  const links = nodes
    .filter((node) => node.type === "link")
    .map((node) => node.url);
  const prefix = path.startsWith("/zh") ? "/zh" : "";
  const route = path.slice(prefix.length) || "/";
  const markers =
    route === "/agents"
      ? [
          "/api/v1/proxy/s/llm-openai/models",
          "/api/v1/llm/gateway/v1/chat/completions",
          "nyxid mcp config",
          "nyxid api-key create",
          "--allow-all-services false",
        ]
      : path === "/zh"
        ? [
            '"mcpServers"',
            "https://nyx-api.chrono-ai.fun/mcp",
            "curl -X POST",
            "delete_event",
          ]
        : [];
  for (const marker of markers)
    assert.ok(
      code.some((value) => value.includes(marker)),
      `${path}: missing code ${marker}`,
    );
  if (path === "/zh")
    for (const title of prefix
      ? [
          "一个开箱即用的助手",
          "看看你的 AI 能做到什么",
          "给智能体钥匙，而不是密码",
        ]
      : [
          "An assistant that just works",
          "See what your AI can really do",
          "Keys for your agents, not passwords",
        ])
      assert.ok(markdown.includes(title), `${path}: missing audience ${title}`);
  if (path === "/") {
    assert.ok(code.some(value => value.trim().split(/\s+/).at(-1) === "https://nyx-api.chrono-ai.fun/mcp" && value.includes("claude mcp add --transport http --scope user")), "Home: retain the visible MCP setup example");
    for (const body of [
      "The pile on your counter, sorted before you wake.",
      "Your agent works in the apps and chats you connect.",
      "Use a separate Agent Key for each agent",
      "Start with a job.",
    ]) assert.ok(markdown.includes(body), `Home: missing story or progressive detail ${body}`);
    for (const target of ["/agents", "/integrations", "/pricing"])
      assert.ok(links.includes(`${origin}${target}`), `Home: missing detail destination ${target}`);
  }
  const targets =
    route === "/agents"
      ? [
          "https://github.com/ChronoAIProject/NyxID",
          `${origin}/docs/ai/guides/mcp-proxy`,
          `${origin}/docs/ai/guides/agent-isolation`,
        ]
      : route === "/"
        ? ["/how-it-works", "/security", "/solutions"].map(
            (value) => `${origin}${prefix}${value}`,
          )
        : [];
  for (const target of targets)
    assert.ok(
      links.includes(target),
      `${path}: missing CommonMark link ${target}`,
    );
}
