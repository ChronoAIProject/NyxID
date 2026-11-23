import { expect, test, type Page } from "@playwright/test";
import { openAssistant } from "./helpers";

async function openThreadPreview(page: Page) {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await expect(page.getByRole("button", { name: /^NyxBot —/ })).toBeVisible();
  await page.evaluate(() => {
    const key = "nyxagent-http-fixture-v4";
    const fixture = JSON.parse(sessionStorage.getItem(key)!);
    const nyxbot = fixture.agents.find((agent: { kind: string }) => agent.kind === "nyxbot");
    const template = fixture.rows.find(([, row]: [string, { agentId: string }]) => row.agentId === nyxbot.id)[1];
    for (let index = 0; index < 12; index++) {
      const row = structuredClone(template);
      const id = `nyxa-${(index + 100).toString(16).padStart(32, "0")}`;
      row.history.messages = [];
      row.history.acknowledgements = [];
      row.history.conversation = { ...row.history.conversation, id, title: `Thread ${index + 1}`,
        channel: null, message_count: 0, pending_acknowledgements: 0, active_turn: null,
        last_message_at: new Date(Date.now() - index * 1000).toISOString() };
      fixture.rows.push([id, row]);
    }
    sessionStorage.setItem(key, JSON.stringify(fixture));
  });
  await page.reload();
  await expect(page.getByRole("button", { name: /^NyxBot —/ })).toBeVisible();
}

test("clicking an agent opens its threads in the content area with no nested sidebar controls", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openThreadPreview(page);
  const roster = page.getByRole("navigation", { name: "Agents and conversations" });
  const column = page.getByRole("complementary", { name: "Threads", exact: true });
  await expect(column).not.toBeVisible();
  await expect(roster.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(0);
  const nyxbot = roster.getByRole("button", { name: /^NyxBot —/ });
  await nyxbot.hover();
  await expect(column).not.toBeVisible();
  await nyxbot.click();
  await expect(column).toBeVisible();
  await expect(column.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(12);
  await expect(roster.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(0);
  await expect(roster.getByRole("button", { name: /^Show \d+ more$|^Show fewer$/ })).toHaveCount(0);
  await expect(roster.getByRole("group", { name: "Threads with NyxBot" })).toHaveCount(0);
  await column.getByRole("button", { name: "Thread 12", exact: true }).click();
  await expect(column).toBeVisible();
  await expect(column.getByRole("button", { name: "Thread 12", exact: true })).toHaveAttribute("aria-current", "page");
  await nyxbot.click();
  await expect(column.getByRole("button", { name: "Thread 12", exact: true })).toHaveAttribute("aria-current", "page");
  await roster.getByRole("button", { name: /^researcher, specialist/ }).click();
  await expect(column.getByRole("heading", { name: "researcher", exact: true })).toBeVisible();
  await expect(column.getByRole("button", { name: "researcher", exact: true })).toBeVisible();
  await expect(column.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(0);
  await nyxbot.click();
  await expect(column.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(12);
  await page.reload();
  await expect(column.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(12);
  await page.getByRole("link", { name: "Automations", exact: true }).click();
  await expect(column).not.toBeVisible();
});

test("an agent's threads can be searched and closed to return home", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openThreadPreview(page);
  const roster = page.getByRole("navigation", { name: "Agents and conversations" });
  await roster.getByRole("button", { name: /^NyxBot —/ }).click();
  const column = page.getByRole("complementary", { name: "Threads", exact: true });
  const search = column.getByRole("searchbox", { name: "Search threads" });
  await search.fill("Morning");
  await expect(column.getByRole("button", { name: "Morning briefing", exact: true })).toBeVisible();
  await search.fill("nothing matches this");
  await expect(column.getByRole("status")).toHaveText("No matching threads");
  await column.getByRole("button", { name: "Clear search" }).click();
  await expect(search).toHaveValue("");
  await column.getByRole("button", { name: "Close", exact: true }).click();
  await expect(column).not.toBeVisible();
  await expect(page.getByRole("button", { name: "Home", exact: true })).toHaveAttribute("aria-current", "page");
  await roster.getByRole("button", { name: /^NyxBot —/ }).click();
  await expect(column).toBeVisible();
  await search.focus();
  await page.keyboard.press("Escape");
  await expect(column).not.toBeVisible();
  await page.reload();
  await expect(column).not.toBeVisible();
});

test("mobile expands in the drawer only when Show more is clicked", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openThreadPreview(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Open chats" }).click();
  const drawer = page.locator('aside[data-tour="sidebar"]:visible');
  await expect(drawer.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(5);
  await drawer.getByRole("button", { name: "Show 7 more", exact: true }).click();
  await expect(drawer.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(12);
  await expect(page.getByRole("complementary", { name: "Threads", exact: true })).not.toBeVisible();
  await drawer.getByRole("button", { name: "Show fewer", exact: true }).click();
  await expect(drawer.getByRole("button", { name: /^Thread \d+$/ })).toHaveCount(5);
});

for (const mode of ["light", "dark"] as const) {
  test(`expanded threads use the existing content surface in ${mode} mode`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((mode) => {
      localStorage.setItem("nyxid.theme", JSON.stringify({ version: 2, state: { mode } }));
    }, mode);
    await openThreadPreview(page);
    const column = page.getByRole("complementary", { name: "Threads", exact: true });
    await expect(column).not.toBeVisible();
    await page.screenshot({ path: testInfo.outputPath(`thread-preview-${mode}.png`) });
    await page.getByRole("button", { name: /^NyxBot —/ }).click();
    await expect(column).toBeVisible();
    const background = await column.evaluate((element) => getComputedStyle(element.firstElementChild!).backgroundColor);
    expect(background).toBe(await page.getByRole("main").evaluate((element) => getComputedStyle(element).backgroundColor));
    expect(background).not.toBe(await page.locator('aside[data-tour="sidebar"]:visible').evaluate((element) => getComputedStyle(element).backgroundColor));
    await page.screenshot({ path: testInfo.outputPath(`thread-expanded-${mode}.png`) });
  });
}
