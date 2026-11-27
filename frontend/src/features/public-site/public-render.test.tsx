// @vitest-environment node
import { describe, expect, it } from "vitest";
import fs from "node:fs/promises";
import { renderPage } from "../../../scripts/public-render";
import { metadataHead, PUBLIC_ORIGIN } from "./metadata";
import { publicationPath } from "./publication";
describe("window-free public renderer", () => {
  it("keeps closing script text inside inert JSON-LD data", async () => {
    const { meta } = await renderPage("/security", async () => "");
    const payload = '</script><SCRIPT>alert("example")</SCRIPT>';
    const head = metadataHead({ ...meta, title: payload, description: payload });
    const data = head.match(/<script data-public-head type="application\/ld\+json">([\s\S]*?)<\/script>/i)?.[1];
    expect(data).toBeDefined();
    expect(data).not.toContain("<");
    expect(JSON.parse(data!).name).toBe(payload);
    expect(head.match(/<script(?:\s|>)/gi)).toHaveLength(1);
  });
  it.each(["/", "/security", "/solutions/help-desk"])(
    "renders %s in two locales with real body and reciprocal head",
    async (route) => {
      expect(typeof window).toBe("undefined");
      for (const locale of ["en", "zh"] as const) {
        const path = publicationPath(route, locale);
        const { html, meta } = await renderPage(path, async () => {
          throw new Error("Unexpected documentation access");
        });
        expect(html).toContain("<h1");
        if (route === "/") {
          // Both homepage locales use the same design and their own chrome.
          expect(html).toContain('class="nyx-home nyx-public"');
          expect(html).toContain("data-nav");
          expect(html).toContain('id="end"');
          expect(html).toContain(locale === "zh" ? "能办事的 AI 助手" : "The AI assistant");
          expect(html).not.toContain("unicorn.studio");
        } else {
          expect(html).toContain('id="nav"');
          expect(html).toContain('id="foot"');
        }
        expect(html).not.toContain("<!--$!-->");
        expect(meta.locale).toBe(locale);
        expect(meta.description.length).toBeGreaterThan(20);
        expect(html).not.toMatch(/<script(?:\s|>)/i);
        const head = metadataHead(meta);
        expect(head).toContain(`href="${PUBLIC_ORIGIN}${path}"`);
        expect(head).toContain('hreflang="zh-Hans"');
        expect(metadataHead(meta, "https://preview.example")).toContain(
          'content="noindex,follow"',
        );
        if (route.startsWith("/solutions/")) {
          expect(html).toContain("disabled");
          expect(html).not.toContain('href="/start');
        }
      }
    },
  );
  it("renders all four first-task cards in the public Docs landing page", async () => {
    const { html } = await renderPage("/docs", async () => "");
    for (const slug of [
      "nyxbot/getting-started/first-chat", "ai/getting-started/connect-your-agent",
      "web/getting-started/first-connection", "cli/getting-started/install",
    ]) {
      expect(html).toContain(`href="/docs/${slug}"`);
    }
  });

  it.each([
    ["nyxbot/getting-started/first-chat", "NyxBot"],
    ["ai/guides/agent-isolation", "AI clients (MCP)"],
    ["web/getting-started/first-connection", "Studio"],
    ["cli/getting-started/install", "CLI"],
    ["shared/concepts/the-proxy", "Concepts"],
  ])("renders %s with its surface in the page title", async (slug, label) => {
    const { html, meta } = await renderPage(`/docs/${slug}`, (source) =>
      fs.readFile(
        new URL(`../../../../docs/site/${source}.md`, import.meta.url),
        "utf8",
      ),
    );
    expect(html).toContain("<h1");
    expect(meta.title).toContain(` · ${label} Docs`);
    expect(meta.translated).toBe(false);
    expect(metadataHead(meta)).not.toContain("hreflang");
  });
});
