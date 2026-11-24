import { expect, test, type Page } from "@playwright/test";
import { openAssistant } from "./helpers";

async function seedChannelThreads(page: Page) {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  return page.evaluate(() => {
    const key = "nyxagent-http-fixture-v4";
    const fixture = JSON.parse(sessionStorage.getItem(key)!);
    const agent = fixture.agents.find((item: { kind: string }) => item.kind === "nyxbot");
    const template = fixture.rows.find(([, row]: [string, { agentId: string }]) => row.agentId === agent.id)[1];
    const ids = ["nyxa-11111111111111111111111111111111", "nyxa-22222222222222222222222222222222"];
    fixture.rows = fixture.rows.filter(([, row]: [string, { agentId: string }]) => row.agentId !== agent.id);
    const direct = structuredClone(template);
    direct.history.conversation = { ...direct.history.conversation, id: "nyxa-33333333333333333333333333333333",
      title: "What I Can Do", channel: null };
    fixture.rows.push([direct.history.conversation.id, direct]);
    for (const [index, id] of ids.entries()) {
      const row = structuredClone(template);
      row.history.conversation = { ...row.history.conversation, id,
        title: "Thread · 2026-11-02 10:16 UTC", title_source: "provisional",
        created_at: `2026-11-02T10:16:${index ? "45" : "12"}Z`,
        last_message_at: `2026-11-02T10:16:${index ? "45" : "12"}Z`,
        channel: { platform: "lark", channel_agent_id: "lark-bot", bot_label: "NyxBot",
          chat_id: `child-${index}`, thread_id: `child-${index}`, parent_chat_id: "parent",
          parent_title: "Product team", chat_kind: "group", chat_title: null } };
      fixture.rows.push([id, row]);
    }
    agent.home_conversation_id = null;
    sessionStorage.setItem(key, JSON.stringify(fixture));
    return ids;
  });
}

for (const width of [1440, 390]) for (const mode of ["light", "dark"] as const) {
  test(`source threads at ${width}px in ${mode}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.addInitScript((mode) => {
      localStorage.setItem("nyxid.theme", JSON.stringify({ version: 2, state: { mode, motion: "reduce" } }));
    }, mode);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const ids = await seedChannelThreads(page);
    await page.goto(`/assistant?mock=1&c=${ids[0]}`, { waitUntil: "domcontentloaded" });
    await expect(page.locator("[data-channel-thread-context]")).toContainText("Product team on Lark");
    if (width === 390) await page.getByRole("button", { name: "Open chats", exact: true }).click();
    const list = width === 390 ? page.locator('aside[data-tour="sidebar"]:visible')
      : page.getByRole("complementary", { name: "Threads", exact: true });
    await expect(list.getByRole("button", { name: "Product team on Lark, 2 threads", exact: true })).toHaveAttribute("aria-expanded", "true");
    await expect(list.getByText("Lark · Bot: NyxBot", { exact: true })).toBeVisible();
    await expect(list.getByRole("button", { name: "Thread · 10:16:12 UTC", exact: true })).toHaveAttribute("aria-current", "page");
    await expect(list.getByRole("button", { name: "Thread · 10:16:45 UTC", exact: true })).toBeVisible();
    expect(await list.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath(`threads-${width}-${mode}.png`) });
    const search = list.getByRole("searchbox", { name: "Search threads" });
    await search.fill("Product team");
    await expect(list.getByRole("button", { name: /^Thread · / })).toHaveCount(2);
    await search.fill("nothing matches");
    await expect(list.getByRole("status")).toHaveText("No matching threads");
    await list.getByRole("button", { name: "Clear search", exact: true }).click();
    await expect(search).toHaveValue("");
    await list.getByRole("button", { name: "Thread · 10:16:45 UTC", exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`c=${ids[1]}`));
    await expect(page.locator("[data-channel-thread-context]")).toContainText("Product team on Lark");
    if (width === 390) {
      await expect(list).not.toBeVisible();
      await page.getByRole("button", { name: "Open chats", exact: true }).click();
      await expect(list.getByRole("button", { name: "Thread · 10:16:45 UTC", exact: true })).toHaveAttribute("aria-current", "page");
      await list.getByRole("button", { name: "Back to workspace", exact: true }).click();
      await list.getByRole("button", { name: /^researcher, specialist/ }).click();
      await expect(list.getByRole("heading", { name: "researcher", exact: true })).toBeVisible();
      await expect(page).toHaveURL(new RegExp(`c=${ids[1]}`));
      await list.getByRole("button", { name: "Back to workspace", exact: true }).click();
      await list.getByRole("button", { name: /^NyxBot —/ }).click();
      await expect(list.getByRole("button", { name: "Thread · 10:16:45 UTC", exact: true })).toHaveAttribute("aria-current", "page");
    } else {
      await list.getByRole("button", { name: "Options for Thread · 10:16:45 UTC", exact: true }).click();
      await page.getByRole("menuitem", { name: "Rename", exact: true }).click();
      await list.getByRole("textbox", { name: "Title", exact: true }).fill("Deployment investigation");
      await list.getByRole("button", { name: "Save", exact: true }).click();
      await expect(list.getByRole("button", { name: "Deployment investigation", exact: true })).toBeVisible();
    }
    expect(errors).toEqual([]);
  });
}
