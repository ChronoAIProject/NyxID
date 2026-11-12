import { expect, test, type Locator, type Page } from "@playwright/test";
import { conversationRow, openAssistant, sendMessage, stopButton } from "./helpers";

const titles = ["First pin candidate", "Second pin candidate", "Newest conversation"];

async function showChats(page: Page) {
  const nav = page.getByRole("navigation", { name: "Agents and conversations" });
  await expect(nav.or(page.getByRole("button", { name: "Open chats" })).first()).toBeVisible();
  if (!(await nav.isVisible())) await page.getByRole("button", { name: "Open chats" }).click();
  await expect(nav).toBeVisible();
  return nav;
}

async function togglePin(page: Page, title: string, pinned = false, focusAfter?: Locator) {
  await showChats(page);
  await conversationRow(page, title).hover();
  const options = page.getByRole("button", { name: `Options for ${title}`, exact: true });
  await options.focus();
  await page.keyboard.press("Enter");
  const action = page.getByRole("menuitem", { name: pinned ? "Unpin chat" : "Pin chat", exact: true });
  await expect(action).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(focusAfter ?? options).toBeFocused();
}

async function expectOrder(page: Page, expected: string[]) {
  const nav = await showChats(page);
  await expect.poll(async () => (await nav.locator('button[aria-label^="Options for "]').evaluateAll(
    (buttons) => buttons.map((button) => button.getAttribute("aria-label")!.slice("Options for ".length)),
  )).filter((title) => titles.includes(title))).toEqual(expected);
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
  test(`pins survive reload and Chat re-entry at ${viewport.width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize(viewport);
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    for (const [index, title] of titles.entries()) {
      if (index) {
        const nav = await showChats(page);
        await nav.getByRole("button", { name: "New chat with NyxBot", exact: true }).click();
      }
      await sendMessage(page, title);
      await expect(page).toHaveURL(/c=nyxa-/);
      await expect(stopButton(page)).not.toBeVisible();
      await showChats(page);
      await expect(conversationRow(page, title)).toBeVisible();
    }
    const selectedUrl = page.url();
    await expectOrder(page, [titles[2]!, titles[1]!, titles[0]!]);
    await togglePin(page, titles[0]!);
    await togglePin(page, titles[1]!);
    await expectOrder(page, [titles[1]!, titles[0]!, titles[2]!]);
    await expect(page).toHaveURL(selectedUrl);
    await expect(conversationRow(page, titles[2]!)).toHaveAttribute("aria-current", "page");
    await expect(page.getByRole("img", { name: "Pinned chat", exact: true })).toHaveCount(2);
    await testInfo.attach("multiple-pins", { body: await page.screenshot({ path: testInfo.outputPath("multiple-pins.png") }), contentType: "image/png" });

    await page.reload();
    await expectOrder(page, [titles[1]!, titles[0]!, titles[2]!]);
    await expect(conversationRow(page, titles[2]!)).toHaveAttribute("aria-current", "page");
    await page.getByRole("link", { name: "Automations", exact: true }).click();
    await expect(page).toHaveURL(/\/assistant\/automations/);
    await showChats(page);
    await page.getByRole("button", { name: "Home", exact: true }).click();
    await showChats(page);
    await conversationRow(page, titles[2]!).click();
    await expectOrder(page, [titles[1]!, titles[0]!, titles[2]!]);

    await togglePin(page, titles[0]!, true);
    await expectOrder(page, [titles[1]!, titles[2]!, titles[0]!]);
    await togglePin(page, titles[1]!, true);
    await expectOrder(page, [titles[2]!, titles[1]!, titles[0]!]);
    await expect(page.getByRole("img", { name: "Pinned chat", exact: true })).toHaveCount(0);

    await togglePin(page, titles[0]!);
    await conversationRow(page, titles[0]!).hover();
    await page.getByRole("button", { name: `Options for ${titles[0]}`, exact: true }).click();
    await page.getByRole("menuitem", { name: "Delete", exact: true }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Delete", exact: true }).click();
    await expect(conversationRow(page, titles[0]!)).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => localStorage.getItem("nyxid.assistant_pins"))).toBeNull();
  });
}

test("channel pins stay within their bot across keyboard actions, reload and Show more", async ({ page }, testInfo) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  // Extend the existing HTTP fixture's durable history, keeping its real schema.
  await page.evaluate(() => {
    const key = "nyxagent-http-fixture-v4";
    const state = JSON.parse(sessionStorage.getItem(key)!);
    const source = state.rows.find(([, row]: [string, { history: { conversation: { channel: unknown } } }]) => row.history.conversation.channel);
    const extra = [];
    for (const [botIndex, bot] of ["First bot", "Second bot"].entries()) {
      for (let index = 0; index < 7; index++) {
        const row = structuredClone(source[1]);
        const id = `nyxa-${String(botIndex + 1)}${String(index).padStart(31, "0")}`;
        row.history.conversation = { ...row.history.conversation,
          id, title: `${bot} chat ${index}`, last_message_at: `2026-11-${String(12 - botIndex).padStart(2, "0")}T00:0${6-index}:00.000Z`,
          channel: { ...row.history.conversation.channel, channel_agent_id: `bot-${botIndex}`, bot_label: bot },
        };
        extra.push([id, row]);
      }
    }
    state.rows = [...state.rows.filter(([id]: [string]) => id !== source[0]), ...extra];
    sessionStorage.setItem(key, JSON.stringify(state));
  });
  await page.reload();
  const nav = await showChats(page);
  const first = nav.getByRole("button", { name: "First bot on Telegram, 7 chats", exact: true });
  const second = nav.getByRole("button", { name: "Second bot on Telegram, 7 chats", exact: true });
  const group = second.locator("..");
  const rows = group.locator("[data-conversation-options]");
  await second.click();
  await group.getByRole("button", { name: "Show 2 more" }).click();
  await togglePin(page, "Second bot chat 6");
  await togglePin(page, "Second bot chat 5");
  await expect(first).toHaveAttribute("aria-expanded", "false");
  await expect(second).toHaveAttribute("aria-expanded", "true");
  await expect(rows).toHaveCount(7);
  await expect(rows.nth(0)).toHaveAttribute("data-conversation-options", `nyxa-2${"5".padStart(31, "0")}`);
  await expect(rows.nth(1)).toHaveAttribute("data-conversation-options", `nyxa-2${"6".padStart(31, "0")}`);
  await expect(nav.locator("[data-channel-group]").nth(0)).toHaveAttribute("data-channel-group", "bot-0");
  await expect(nav.locator("[data-channel-group]").nth(1)).toHaveAttribute("data-channel-group", "bot-1");
  await testInfo.attach("channel-pins-in-bot", { body: await page.screenshot({ path: testInfo.outputPath("channel-pins-in-bot.png") }), contentType: "image/png" });
  await page.reload();
  await showChats(page);
  await expect(second).toHaveAttribute("aria-expanded", "false");
  await second.click();
  await expect(rows).toHaveCount(5);
  await expect(rows.nth(0)).toHaveAttribute("aria-label", "Options for Second bot chat 5");
  await expect(rows.nth(1)).toHaveAttribute("aria-label", "Options for Second bot chat 6");
  await togglePin(page, "Second bot chat 5", true, second);
  await expect(rows.nth(0)).toHaveAttribute("aria-label", "Options for Second bot chat 6");
  // Unpinning beyond the preview limit returns focus to the bot header.
  const options = page.getByRole("button", { name: "Options for Second bot chat 6", exact: true });
  await options.focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(second).toBeFocused();
  await expect(rows.nth(0)).toHaveAttribute("aria-label", "Options for Second bot chat 0");
  await expect(first).toHaveAttribute("aria-expanded", "false");
});

test("storage failures remain retryable and account changes clear saved pins", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Local pin preference");
  await expect(page).toHaveURL(/c=nyxa-/);
  await showChats(page);
  await page.evaluate(() => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (key === "nyxid.assistant_pins") {
        Storage.prototype.setItem = original;
        throw new DOMException("Storage full", "QuotaExceededError");
      }
      original.call(this, key, value);
    };
  });
  await togglePin(page, "Local pin preference");
  await expect(page.getByRole("alert")).toHaveText(/Could not save pinned chats/);
  await expect(page.getByRole("img", { name: "Pinned chat", exact: true })).toHaveCount(0);
  await togglePin(page, "Local pin preference");
  await expect(page.getByRole("img", { name: "Pinned chat", exact: true })).toHaveCount(1);
  await expect(page.getByText("Could not save pinned chats in this browser. Please try again.")).toHaveCount(0);
  await page.evaluate(() => {
    const { authStore } = (window as unknown as { __nyxApp: { authStore: typeof import("../src/stores/auth-store").useAuthStore } }).__nyxApp;
    authStore.getState().setUser({ ...authStore.getState().user!, id: "another-human" });
  });
  await expect.poll(() => page.evaluate(() => localStorage.getItem("nyxid.assistant_pins"))).toBeNull();
  await expect(page.getByRole("img", { name: "Pinned chat", exact: true })).toHaveCount(0);
});
