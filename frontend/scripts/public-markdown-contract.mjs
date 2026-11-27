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
      : [];
  for (const marker of markers)
    assert.ok(
      code.some((value) => value.includes(marker)),
      `${path}: missing code ${marker}`,
    );
  if (route === "/") {
    assert.ok(code.some(value => value.trim().split(/\s+/).at(-1) === "https://nyx-api.chrono-ai.fun/mcp" && value.includes("claude mcp add --transport http --scope user")), "Home: retain the visible MCP setup example");
    const bodies = prefix ? [
      "台面上积下的事，醒来前就办妥。",
      "智能体在你连接的应用和聊天渠道里办事。",
      "为每个智能体分配独立 Agent Key",
      "先交给它一件事。",
    ] : [
      "The pile on your counter, sorted before you wake.",
      "Your agent works in the apps and chats you connect.",
      "Use a separate Agent Key for each agent",
      "Start with a job.",
    ];
    for (const body of bodies)
      assert.ok(markdown.includes(body), `${path}: missing story or progressive detail ${body}`);
    for (const target of ["/agents", "/integrations", "/pricing"])
      assert.ok(links.includes(`${origin}${prefix}${target}`), `${path}: missing detail destination ${target}`);
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
