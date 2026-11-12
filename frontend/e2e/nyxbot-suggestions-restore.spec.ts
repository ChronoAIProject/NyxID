import { expect, test, type Locator, type Page } from "@playwright/test";

const root = "/api/v1/assistant/nyxagent/onboarding";
const at = "2026-11-12T00:00:00Z";
const limits = {
  skip_destructive_confirmation: false, max_live_subagents: 2,
  max_concurrent_subagent_turns: 1, max_live_subagents_limit: 8,
  max_concurrent_subagent_turns_limit: 4,
};
const copy = {
  en: { hide: "Hide", undo: "Undo", show: "Show suggestions", hidden: "Suggestions are hidden." },
  "zh-CN": { hide: "隐藏", undo: "撤销", show: "显示推荐", hidden: "推荐目前已隐藏。" },
  "zh-TW": { hide: "隱藏", undo: "復原", show: "顯示推薦項目", hidden: "推薦項目目前已隱藏。" },
};

async function tabTo(page: Page, target: Locator) {
  for (let index = 0; index < 60; index++) {
    if (await target.evaluate(element => document.activeElement === element)) return;
    await page.keyboard.press("Tab");
  }
  await expect(target).toBeFocused();
}

async function stubHome(page: Page) {
  // Test mode installs an in-memory assistant transport even without ?mock=1.
  // Keep these checks on the real HTTP client and page.route boundary instead.
  await page.addInitScript(() => {
    Object.defineProperty(globalThis, "__nyxidAssistantHttpMock", {
      get: () => undefined,
      set: () => {},
    });
  });
  const unexpected: string[] = [];
  const writes: string[] = [];
  let hidden = false;
  await page.route("**/api/**", route => {
    const path = new URL(route.request().url()).pathname.replace("/api/v1", "");
    const fixtures: Record<string, unknown> = {
      "/users/me": {
        id: "suggestions-owner", email: "reader@example.test", display_name: "Reader",
        email_verified: true, is_admin: false, is_active: true, onboarding_completed: true, created_at: at,
        capabilities: { billing_available: false, enabled_features: ["experimental:ai-assistant", "assistant:nyxagent-engine"] },
      },
      "/public/config": {}, "/orgs": { orgs: [], organizations: [] },
      "/assistant/nyxagent/agents": { agents: [], limits },
      "/assistant/nyxagent/settings": limits,
      "/assistant/nyxagent/groups": { groups: [] },
      "/assistant/nyxagent/conversations": { conversations: [], total: 0 },
      "/assistant/nyxagent/channels": { channel_agents: [] },
      "/assistant/nyxagent/models": [{ id: "fixture/chat", label: "Example model", available: true }],
      "/assistant/nyxagent/platform-models": { models: [], has_more: false, next_after: null },
      "/assistant/nyxagent/machines": [],
      "/assistant/nyxagent/cma-sessions": { sessions: [] },
      "/approvals/requests": { requests: [], total: 0 },
      "/platform-personal-bindings": { bindings: [], next_before_id: null },
      "/onboarding/bindings": { bindings: [] },
      "/keys": { keys: [] }, "/catalog": { entries: [] },
    };
    if (path === "/assistant/nyxagent/live") return route.fulfill({ status: 404, json: {} });
    if (path in fixtures && route.request().method() === "GET") return route.fulfill({ json: fixtures[path] });
    unexpected.push(`${route.request().method()} ${path}`);
    return route.fulfill({ status: 500, json: { message: "Unconfigured fixture request blocked locally" } });
  });
  await page.route(`**${root}**`, route => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (request.method() === "POST") {
      expect([`${root}/suggestions/hide`, `${root}/suggestions/show`]).toContain(path);
      hidden = path.endsWith("/hide");
      writes.push(path);
    } else expect(request.method()).toBe("GET");
    return route.fulfill({ json: {
      show_intro: false, suggestions_hidden: hidden, intro_version: 1,
      profile: { outcome: "completed", topics: ["research"], work_style: "solo", intro_version: 1, updated_at: at },
      topics: [], work_styles: [], recommendations: [
        { id: "research", title: "Research a topic", description: "Learn about something new", prompt: "Research a topic", icon: "search" },
      ],
    } });
  });
  return { unexpected, writes };
}

for (const width of [1440, 390])
  for (const theme of ["light", "dark"])
    for (const language of ["en", "zh-CN", "zh-TW"] as const) {
      test(`suggestions restore with keyboard (${width}, ${theme}, ${language})`, async ({ page }, testInfo) => {
        await page.setViewportSize({ width, height: 900 });
        await page.addInitScript(({ theme, language }) => {
          localStorage.setItem("nyxid.language", language);
          localStorage.setItem("nyxid.theme", JSON.stringify({ version: 2, state: { mode: theme, motion: "reduce" } }));
        }, { theme, language });
        const fixture = await stubHome(page);
        const errors: string[] = [];
        page.on("pageerror", error => errors.push(error.message));
        await page.goto("/assistant", { waitUntil: "domcontentloaded" });
        const labels = copy[language];
        const hide = page.getByRole("button", { name: labels.hide, exact: true });
        await expect(hide).toBeVisible();
        await expect(page.locator("html")).toHaveClass(new RegExp(`theme-${theme}`));
        const editor = page.locator("[data-composer-textbox]");
        await editor.fill("Keep this draft 保留草稿");
        await tabTo(page, hide);
        await page.keyboard.press("Enter");
        const undo = page.getByRole("button", { name: labels.undo, exact: true });
        await expect(undo).toBeFocused();
        await expect(undo).toBeEnabled();
        await page.screenshot({ path: testInfo.outputPath(`hidden-undo-${width}-${theme}-${language}.png`) });
        await page.keyboard.press("Enter");
        await expect(page.getByText("Research a topic", { exact: true })).toBeVisible();
        await expect(hide).toBeFocused();
        await expect(editor).toHaveText("Keep this draft 保留草稿");
        await page.keyboard.press("Enter");
        await expect(undo).toBeFocused();
        await page.reload({ waitUntil: "domcontentloaded" });
        const show = page.getByRole("button", { name: labels.show, exact: true });
        await expect(show).toBeVisible();
        await expect(page.getByText(labels.hidden, { exact: true })).toBeVisible();
        await page.screenshot({ path: testInfo.outputPath(`hidden-reload-${width}-${theme}-${language}.png`) });
        await tabTo(page, show);
        await page.keyboard.press("Enter");
        await expect(page.getByText("Research a topic", { exact: true })).toBeVisible();
        await expect(hide).toBeFocused();
        await page.screenshot({ path: testInfo.outputPath(`restored-${width}-${theme}-${language}.png`) });
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
        expect(fixture.writes).toEqual([`${root}/suggestions/hide`, `${root}/suggestions/show`, `${root}/suggestions/hide`, `${root}/suggestions/show`]);
        expect(fixture.unexpected).toEqual([]);
        expect(errors).toEqual([]);
      });
    }
