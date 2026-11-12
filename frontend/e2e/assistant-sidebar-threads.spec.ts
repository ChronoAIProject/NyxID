import { expect, test, type Page } from "@playwright/test";
import { openAssistant } from "./helpers";

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
      test(`regular chat disclosure at ${width}px in ${mode}, ${language}`, async ({ page }) => {
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
          await page.getByRole("button", { name: language === "en" ? "Open chats" : language === "zh-CN" ? "打开聊天" : "開啟聊天" }).click();
        }
        const nav = page.getByRole("navigation").filter({ has: page.getByRole("button", { name: "Regular chat 1", exact: true }) });
        await expect(nav.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveText([
          "Regular chat 1", "Regular chat 2", "Regular chat 3", "Regular chat 4", "Regular chat 5",
          "Regular chat 8", "Regular chat 10", /^Regular chat 11/,
        ]);
        const more = language === "en" ? "Show 4 more" : language === "zh-CN" ? "再显示 4 个" : "再顯示 4 個";
        const fewer = language === "en" ? "Show fewer" : language === "zh-CN" ? "收起" : "收合";
        const controlsId = await nav.getByRole("button", { name: more, exact: true }).getAttribute("aria-controls");
        const toggle = nav.locator(`button[aria-controls="${controlsId}"]`);
        await expect(toggle).toHaveAttribute("aria-expanded", "false");
        await nav.getByRole("button", { name: "Regular chat 11", exact: true }).focus();
        for (let tab = 0; tab < 4 && !(await toggle.evaluate((element) => element === document.activeElement)); tab++) {
          await page.keyboard.press("Tab");
        }
        await expect(toggle).toBeFocused();
        await expect(toggle).toBeInViewport({ ratio: 1 });
        await page.screenshot({ path: `../.tmp/sol-b/sidebar-${width}-${mode}-${language}-collapsed.png` });
        await page.keyboard.press("Enter");
        await expect(toggle).toHaveAccessibleName(fewer);
        await expect(toggle).toHaveAttribute("aria-expanded", "true");
        await expect(toggle).toBeFocused();
        await expect(toggle).toBeInViewport({ ratio: 1 });
        await expect(nav.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(12);
        await page.screenshot({ path: `../.tmp/sol-b/sidebar-${width}-${mode}-${language}-expanded.png` });
        await page.keyboard.press("Space");
        await expect(toggle).toHaveAccessibleName(more);
        await expect(toggle).toHaveAttribute("aria-expanded", "false");
        await expect(toggle).toBeFocused();
        await expect(nav.getByRole("button", { name: /^Regular chat \d+$/ })).toHaveCount(8);
        await expect(nav).toBeVisible();
        // Disclosure state is intentionally local to the mounted sidebar.
        await page.reload({ waitUntil: "domcontentloaded" });
        if (width === 390) await page.getByRole("button", { name: language === "en" ? "Open chats" : language === "zh-CN" ? "打开聊天" : "開啟聊天" }).click();
        await expect(page.getByRole("button", { name: more, exact: true })).toHaveAttribute("aria-expanded", "false");
      });
    }
  }
}
