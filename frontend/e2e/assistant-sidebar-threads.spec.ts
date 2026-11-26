import { expect, test, type Page } from "@playwright/test";
import { openAssistant, waitForAssistantNavigation } from "./helpers";

async function seedThreads(page: Page) {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  return page.evaluate(() => {
    const key = "nyxagent-http-fixture-v4";
    const fixture = JSON.parse(sessionStorage.getItem(key)!);
    const nyxbot = fixture.agents.find((agent: { kind: string }) => agent.kind === "nyxbot");
    const template = fixture.rows.find(([, row]: [string, { agentId: string }]) => row.agentId === nyxbot.id)[1];
    const ids = Array.from({ length: 12 }, (_, index) => `nyxa-${(index + 100).toString(16).padStart(32, "0")}`);
    const rows = ids.map((id, index) => {
      const row = structuredClone(template);
      row.history.messages = [];
      row.history.acknowledgements = [];
      row.history.conversation = {
        ...row.history.conversation, id, title: `Regular chat ${index + 1}`, channel: null,
        message_count: 0, pending_acknowledgements: 0,
        last_message_at: new Date(Date.UTC(2026, 10, 12, 12) - index * 1000).toISOString(),
        active_turn: index === 7 ? { turn_id: "sidebar-running", started_at: new Date().toISOString() } : null,
      };
      return [id, row];
    });
    fixture.rows.push(...rows);
    sessionStorage.setItem(key, JSON.stringify(fixture));
    return ids;
  });
}

for (const width of [1440, 390]) {
  for (const mode of ["light", "dark"] as const) {
    for (const language of ["en", "zh-CN", "zh-TW"] as const) {
      test(`regular chat disclosure at ${width}px in ${mode}, ${language}`, async ({ page }, testInfo) => {
        await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
        await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) => route.fulfill({ json: [] }));
        await page.route("**/api/v1/assistant/nyxagent/cma-sessions?**", (route) => route.fulfill({ json: { sessions: [] } }));
        const ids = await seedThreads(page);
        await page.addInitScript(({ mode, language }) => {
          localStorage.setItem("nyxid.language", language);
          localStorage.setItem("nyxid.theme", JSON.stringify({ version: 2, state: { mode, textScale: 1, density: 1, motion: "reduce" } }));
        }, { mode, language });
        await page.goto(`/assistant?mock=1&c=${ids[9]}`, { waitUntil: "domcontentloaded" });
        await expect(page.locator("[data-composer-textbox]")).toBeVisible();
        await page.evaluate(async (id) => {
          const { useAssistantDraftStore } = await import("/src/stores/assistant-draft-store.ts");
          const app = (window as unknown as { __nyxApp: { authStore: { getState: () => { user: { id: string } } } } }).__nyxApp;
          useAssistantDraftStore.getState().saveDraft(app.authStore.getState().user.id, `conv:${id}`, "Draft to finish later");
        }, ids[10]!);
        if (width === 390) {
          await waitForAssistantNavigation(page);
          await page.getByRole("button", { name: language === "en" ? "Open chats" : language === "zh-CN" ? "打开聊天" : "開啟聊天" }).click();
        }
        if (width >= 1024) {
          const column = page.getByRole("complementary", { name: language === "en" ? "Threads" : language === "zh-CN" ? "对话" : "對話", exact: true });
          const list = column.getByRole("navigation");
          await expect(column).toBeVisible();
          const sidebar = page.locator('aside[data-tour="sidebar"]:visible');
          await expect(sidebar.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(0);
          await expect(list.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(12);
          const search = column.getByRole("searchbox");
          await search.fill("Regular chat 12");
          await expect(list.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(1);
          await expect(list.getByRole("button", { name: "Regular chat 12", exact: true })).toBeVisible();
          await search.fill("");
          await expect(list.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(12);
          await page.screenshot({ path: testInfo.outputPath(`thread-column-${mode}-${language}.png`) });
          await page.keyboard.press("Escape");
          await expect(column).not.toBeVisible();
          await expect(sidebar.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(0);
          await page.reload();
          await expect(column).not.toBeVisible();
          return;
        }
        const drawer = page.locator('aside[data-tour="sidebar"]:visible');
        const nav = drawer.getByRole("navigation");
        await expect(nav.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(12);
        await expect(nav.getByRole("button", { name: "Regular chat 10", exact: true })).toHaveAttribute("aria-current", "page");
        await expect(nav.getByRole("button", { name: "Regular chat 11", exact: true })).toHaveAccessibleDescription(
          language === "en" ? "Draft: Draft to finish later" : "草稿： Draft to finish later",
        );
        const search = drawer.getByRole("searchbox");
        await search.fill("Regular chat 12");
        await expect(nav.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(1);
        await search.fill("");
        await expect(nav.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(12);
        await page.screenshot({ path: testInfo.outputPath(`mobile-threads-${mode}-${language}.png`) });
      });
    }
  }
}
