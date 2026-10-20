// @vitest-environment node
import { expect, it } from "vitest";
import { unified } from "unified";
import remarkParse from "remark-parse";
import { renderPage } from "../scripts/public-render";
// @ts-expect-error Build-only JavaScript module.
import { semanticMarkdown } from "../scripts/public-content.mjs";
// @ts-expect-error Build-only JavaScript module.
import { verifyMarkdownContract } from "../scripts/public-markdown-contract.mjs";
const parse = (value: string) =>
  unified()
    .use(remarkParse)
    .parse(value.replace(/^---\n[\s\S]*?\n---\n/, ""));
function nodes(tree: unknown, type: string): Record<string, unknown>[] {
  const found: Record<string, unknown>[] = [];
  const visit = (n: unknown) => {
    if (!n || typeof n !== "object") return;
    const node = n as Record<string, unknown>;
    if (node.type === type) found.push(node);
    if (Array.isArray(node.children)) node.children.forEach(visit);
  };
  visit(tree);
  return found;
}
it.each(["en", "zh"])(
  "F05 preserves every approved code and audience panel in %s",
  async (locale) => {
    const prefix = locale === "zh" ? "/zh" : "";
    const page = await renderPage(prefix + "/agents", async () => "");
    const md = semanticMarkdown(
      page.html,
      page.meta,
      "test",
      "https://nyx.chrono-ai.fun",
    );
    verifyMarkdownContract(page.meta.path, md);
    const ast = parse(md);
    const codes = nodes(ast, "code").map((n) => String(n.value));
    for (const marker of [
      "/api/v1/proxy/s/google-calendar",
      "/api/v1/llm/gateway/v1/chat/completions",
      '"mcpServers"',
      "nyxid api-key create",
      "nyxid node setup --machine --computer",
    ])
      expect(
        codes.some((code) => code.includes(marker)),
        marker,
      ).toBe(true);
    for (const label of ["curl", "LLM", "MCP", "CLI"])
      expect(
        nodes(ast, "heading").some((h) => JSON.stringify(h).includes(label)),
        label,
      ).toBe(true);
    const home = await renderPage(prefix || "/", async () => "");
    const homeMd = semanticMarkdown(
      home.html,
      home.meta,
      "test",
      "https://nyx.chrono-ai.fun",
    );
    verifyMarkdownContract(home.meta.path, homeMd);
    const homeCodes = nodes(parse(homeMd), "code").map((n) => String(n.value));
    for (const marker of [
      '"mcpServers"',
      "https://nyx-api.chrono-ai.fun/mcp",
      "curl -X POST",
      "delete_event",
    ])
      expect(
        homeCodes.some((code) => code.includes(marker)),
        marker,
      ).toBe(true);
    for (const label of locale === "zh"
      ? ["给企业", "给 AI 爱好者", "给开发者"]
      : ["For businesses", "For AI enthusiasts", "For developers"])
      expect(homeMd).toContain(label);
  },
);
it.each(["en", "zh"])(
  "F06 emits parseable card links and separated blocks in %s",
  async (locale) => {
    const prefix = locale === "zh" ? "/zh" : "";
    for (const [route, targets] of [
      [
        "/agents",
        [
          "https://github.com/ChronoAIProject/NyxID",
          "/docs/ai/guides/mcp-proxy",
          "/docs/ai/guides/agent-isolation",
        ],
      ],
      ["/", ["/how-it-works", "/security", "/solutions"]],
    ] as const) {
      const page = await renderPage(
        prefix + (route === "/" ? "" : route) || "/",
        async () => "",
      );
      const ast = parse(
        semanticMarkdown(
          page.html,
          page.meta,
          "test",
          "https://nyx.chrono-ai.fun",
        ),
      );
      const links = nodes(ast, "link").map((n) => n.url);
      for (const target of targets) {
        const expected = target.startsWith("https:")
          ? target
          : "https://nyx.chrono-ai.fun" +
            (target.startsWith("/docs") ? "" : prefix) +
            target;
        expect(links, expected).toContain(expected);
      }
    }
    const md = semanticMarkdown(
      '<main><h1>A guide</h1><a href="/security?a=1&amp;b=2"><h3>Title [A] &amp; B *</h3><p>A useful paragraph.</p></a><a href="/agents"><b>开发者 [工具]</b><p>另一个说明。</p></a></main>',
      { path: "/", locale: "en", title: "A guide" },
      "test",
      "https://nyx.chrono-ai.fun",
    );
    expect(nodes(parse(md), "link").map((n) => n.url)).toEqual([
      "https://nyx.chrono-ai.fun/security?a=1&b=2",
      "https://nyx.chrono-ai.fun/agents",
    ]);
    expect(nodes(parse(md), "heading").length).toBe(2);
  },
);

it("retains inline code within card titles and excludes unapproved hidden panels", () => {
  const md = semanticMarkdown(
    '<main><h1>Guide</h1><a href="/security"><h3>Use <code>scope_key</code></h3><p>Check the scope.</p></a><p hidden>HIDDEN_SENTINEL</p><div hidden data-public-panel="draft">DRAFT_SENTINEL</div><pre hidden data-public-panel="unknown">UNKNOWN_SENTINEL</pre><pre hidden data-public-panel="code" data-public-label="Approved">approved_code</pre></main>',
    { path: "/", locale: "en", title: "Guide" },
    "test",
    "https://nyx.chrono-ai.fun",
  );
  expect(nodes(parse(md), "inlineCode").map((n) => n.value)).toContain(
    "scope_key",
  );
  expect(md).not.toMatch(/HIDDEN_SENTINEL|DRAFT_SENTINEL|UNKNOWN_SENTINEL/);
  expect(nodes(parse(md), "code").map((n) => n.value)).toContain(
    "approved_code",
  );
});
