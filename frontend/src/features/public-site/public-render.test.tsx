// @vitest-environment node
import { describe, expect, it } from "vitest";
import fs from "node:fs/promises";
import { renderPage } from "../../../scripts/public-render";
import { metadataHead } from "./metadata";
import { publicationPath } from "./publication";
describe("window-free public renderer", () => {
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
        expect(html).toContain('id="nav"');
        expect(html).toContain('id="foot"');
        expect(html).not.toContain("<!--$!-->");
        expect(meta.locale).toBe(locale);
        expect(meta.description.length).toBeGreaterThan(20);
        expect(html).not.toMatch(/<script(?:\s|>)/);
        const head = metadataHead(meta);
        expect(head).toContain(`href="https://nyx.chrono-ai.fun${path}"`);
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
  it("renders manifested Docs from Markdown without the private application", async () => {
    const path = "/docs/ai/guides/agent-isolation";
    const { html, meta } = await renderPage(path, (slug) =>
      fs.readFile(
        new URL(`../../../../docs/site/${slug}.md`, import.meta.url),
        "utf8",
      ),
    );
    expect(html).toContain("<h1");
    expect(meta.translated).toBe(false);
    expect(metadataHead(meta)).not.toContain("hreflang");
  });
});
