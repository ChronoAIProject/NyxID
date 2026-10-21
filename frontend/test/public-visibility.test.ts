// @vitest-environment node
import { afterEach, expect, it, vi } from "vitest";
// @ts-expect-error Build-only module.
import { semanticMarkdown } from "../scripts/public-content.mjs";
afterEach(() => {
  vi.unstubAllEnvs();
  vi.resetModules();
});
const env = {
  VITE_WEBSITE_NEWSLETTER_ENDPOINT: "/api/public/newsletter",
  VITE_WEBSITE_NEWSLETTER_PRIVACY_URL: "https://nyx.chrono-ai.fun/privacy",
  VITE_WEBSITE_NEWSLETTER_UNSUBSCRIBE_URL:
    "https://status.example.test/unsubscribe",
  VITE_WEBSITE_NEWSLETTER_ACCEPTANCE: "review/newsletter",
  VITE_WEBSITE_CONTACT_CHAT_ENDPOINT: "/api/public/chat",
  VITE_WEBSITE_CONTACT_CHAT_ACCEPTANCE: "review/chat",
  VITE_WEBSITE_STATUS_URL: "https://status.example.test/",
  VITE_WEBSITE_STATUS_ACCEPTANCE: "review/status",
};
it.each([null, "newsletter", "contact-chat", "footer-status"])(
  "keeps actual public rendering and Markdown aligned (%s)",
  async (feature) => {
    for (const [key, value] of Object.entries(env)) vi.stubEnv(key, value);
    for (const [name, key] of [
      ["newsletter", "NEWSLETTER"],
      ["contact-chat", "CONTACT_CHAT"],
      ["footer-status", "STATUS"],
    ])
      vi.stubEnv(`VITE_WEBSITE_${key}_ENABLED`, String(name === feature));
    vi.resetModules();
    const { renderPage, websiteVisibility } =
      await import("../scripts/public-render");
    for (const locale of ["en", "zh"]) {
      const prefix = locale === "zh" ? "/zh" : "";
      for (const route of ["/blog", "/contact", "/security"]) {
        const page = await renderPage(prefix + route, async () => "");
        for (const name of ["newsletter", "contact-chat", "footer-status"])
          expect(page.html.includes(`data-website-feature="${name}"`)).toBe(
            name === feature &&
              (name === "footer-status" ||
                route === (name === "newsletter" ? "/blog" : "/contact")),
          );
        const md = semanticMarkdown(
          page.html,
          page.meta,
          "test",
          "https://nyx.chrono-ai.fun",
        );
        expect(
          md.includes(locale === "zh" ? "订阅更新" : "Subscribe for updates"),
        ).toBe(feature === "newsletter" && route === "/blog");
        expect(
          md.includes(locale === "zh" ? "和 NyxBot 聊聊" : "Chat with NyxBot"),
        ).toBe(feature === "contact-chat" && route === "/contact");
        expect(page.html).not.toContain('data-public-panel="newsletter"');
        expect(page.meta.path).toBe(prefix + route);
      }
    }
    expect(websiteVisibility.solutions).toEqual([]);
  },
);
it("fails the real renderer import for requested but incomplete release configuration", async () => {
  vi.stubEnv("VITE_WEBSITE_NEWSLETTER_ENABLED", "true");
  vi.stubEnv("VITE_WEBSITE_NEWSLETTER_ENDPOINT", "");
  vi.resetModules();
  await expect(import("../scripts/public-render")).rejects.toThrow(
    /configuration/,
  );
});
it("updates availability metadata only for the configured solution", async () => {
  vi.stubEnv(
    "VITE_WEBSITE_SOLUTION_ACTIVATIONS",
    JSON.stringify({
      "help-desk": {
        target: "/assistant?setup=help-desk",
        acceptance: "review/accepted",
      },
    }),
  );
  vi.resetModules();
  const { renderPage } = await import("../scripts/public-render");
  const enabled = await renderPage("/solutions/help-desk", async () => "");
  const closed = await renderPage(
    "/solutions/restaurant-manager",
    async () => "",
  );
  expect(enabled.html).toContain("/assistant?setup=help-desk");
  expect(enabled.meta.description).not.toContain("Not available yet.");
  expect(closed.html).toContain("Not available yet");
  expect(closed.meta.description).toContain("Not available yet.");
});

it.each(["false", "true"])(
  "controls Blog navigation only, keeping direct access and footer (%s)",
  async (enabled) => {
    vi.stubEnv("VITE_WEBSITE_BLOG_MENU_ENABLED", enabled);
    vi.resetModules();
    const { renderPage, publishedPages } =
      await import("../scripts/public-render");
    for (const path of ["/blog", "/zh/blog"]) {
      const page = await renderPage(path, async () => "");
      expect(page.html.includes('data-nav="blog"')).toBe(enabled === "true");
      if (enabled === "true")
        expect(page.html.match(/<a[^>]*data-nav="blog"[^>]*>/)?.[0]).toContain(
          'aria-current="page"',
        );
      expect(page.html).toContain("<h1");
      expect(page.html).toContain(path);
      expect(page.html).toMatch(/href="(?:\/zh)?\/blog"/);
    }
    expect(publishedPages.some((page) => page.route === "/blog")).toBe(true);
  },
);
