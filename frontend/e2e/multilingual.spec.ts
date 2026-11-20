import { expect, test, type Page } from "@playwright/test";
import { composerInput, openAssistant, sendMessage, stopButton } from "./helpers";

async function openAccountMenu(page: Page) {
  const menu = page.getByRole("button", { name: /^(Account menu|账号菜单|帳號選單)$/ });
  if (!await menu.isVisible()) {
    await page.getByRole("button", { name: /^(Open chats|打开聊天|開啟聊天)$/ }).click();
  }
  await menu.click();
}

async function closeMobileChats(page: Page) {
  const close = page.getByRole("button", { name: /^(Close chats|关闭聊天|關閉聊天)$/ });
  if (await close.isVisible()) await close.click();
}

test("an open action card and its decision receipt follow the selected language", async ({ page }) => {
  const unexpected: string[] = [];
  await page.route("**/api/v1/**", (route) => {
    unexpected.push(`${route.request().method()} ${new URL(route.request().url()).pathname}`);
    return route.fulfill({ status: 500, json: { message: "Unexpected request outside fixtures" } });
  });
  await page.route("**/api/v1/assistant/nyxagent/cma-sessions?**", (route) => route.fulfill({ json: { sessions: [] } }));
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) => route.fulfill({ json: [] }));
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Delete agent key ci-bot");
  const card = page.getByRole("region", { name: /Confirm: Delete agent key 'ci-bot'/ });
  await expect(card).toBeVisible();
  await expect(stopButton(page)).not.toBeVisible({ timeout: 10_000 });
  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  await page.getByRole("combobox", { name: "Language", exact: true }).selectOption("zh-CN");
  await page.getByRole("button", { name: "关闭", exact: true }).click();
  await expect(page.getByRole("region", { name: /确认：Delete agent key 'ci-bot'/ })).toBeVisible();
  await page.getByRole("button", { name: "拒绝", exact: true }).click();
  const receipt = page.getByRole("status").filter({ hasText: "你已拒绝 · Delete agent key" });
  await expect(receipt).toBeVisible();
  await expect(receipt).toContainText("刚刚");

  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "设置", exact: true }).click();
  await page.getByRole("combobox", { name: "语言", exact: true }).selectOption("en");
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.getByRole("status").filter({ hasText: "You denied · Delete agent key" })).toBeVisible();
  expect(unexpected).toEqual([]);
});

for (const width of [1280, 390]) {
  test(`language changes survive navigation and reload at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    const editor = composerInput(page);
    await editor.fill("Keep my draft 保留草稿");
    await openAccountMenu(page);
    await expect(page.getByRole("menuitem", { name: "Language", exact: true })).toHaveCount(0);
    await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
    await page.getByRole("combobox", { name: "Language", exact: true }).selectOption("zh-CN");
    await page.getByRole("button", { name: "关闭", exact: true }).click();
    await closeMobileChats(page);
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
    await expect(
      page.getByRole("button", { name: "发送消息", exact: true }),
    ).toBeVisible();
    await expect(editor).toHaveText("Keep my draft 保留草稿");

    await openAccountMenu(page);
    await page.getByRole("menuitem", { name: "设置", exact: true }).click();
    await page.getByRole("combobox", { name: "语言", exact: true }).selectOption("zh-TW");
    await page.getByRole("button", { name: "關閉", exact: true }).click();
    await closeMobileChats(page);
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-TW");
    await expect(
      page.getByRole("button", { name: "傳送訊息", exact: true }),
    ).toBeVisible();
    await expect(editor).toHaveText("Keep my draft 保留草稿");
    await page.screenshot({
      path: testInfo.outputPath(`nyxbot-zh-TW-${width}.png`),
    });

    await openAccountMenu(page);
    await page
      .getByRole("menuitem", { name: "開啟 Studio", exact: true })
      .click();
    await expect(page).toHaveURL(/\/dashboard/);
    await expect(page.getByRole("heading", { name: /歡迎回來/ })).toBeVisible();
    await expect(page.getByText("建立限定權限的金鑰", { exact: true })).toBeVisible();
    await page.goto("/settings?mock=1");
    await expect(
      page.getByRole("combobox", { name: "語言", exact: true }),
    ).toHaveValue("zh-TW");
    await expect(
      page.getByRole("heading", { name: "帳號設定", exact: true }),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`studio-zh-TW-${width}.png`),
    });

    await page.reload();
    await expect(
      page.getByRole("combobox", { name: "語言", exact: true }),
    ).toHaveValue("zh-TW");
    await page
      .getByRole("combobox", { name: "語言", exact: true })
      .selectOption("en");
    await expect(
      page.getByRole("heading", { name: "Account Settings", exact: true }),
    ).toBeVisible();
  });
}

test("Studio translates open forms across tabs without translating drafts", async ({
  page,
  context,
}, testInfo) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/services?mock=1");
  await page.getByRole("button", { name: "Create Service", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Service Name", { exact: true }).fill("Profile");
  await dialog.getByLabel("Description", { exact: true }).fill("Keep this draft 保留草稿");

  const settings = await context.newPage();
  settings.on("pageerror", (error) => errors.push(error.message));
  await settings.goto("/settings?mock=1");
  await settings.getByRole("combobox", { name: "Language", exact: true }).selectOption("zh-CN");
  await expect(dialog.getByRole("heading", { name: "创建服务", exact: true })).toBeVisible();
  await expect(dialog.getByLabel("服务名称", { exact: true })).toHaveValue("Profile");
  await expect(dialog.getByLabel("说明", { exact: true })).toHaveValue("Keep this draft 保留草稿");

  await settings.getByRole("combobox", { name: "语言", exact: true }).selectOption("zh-TW");
  await expect(dialog.getByRole("heading", { name: "建立服務", exact: true })).toBeVisible();
  await expect(dialog.getByLabel("服務名稱", { exact: true })).toHaveValue("Profile");
  await expect(dialog.getByLabel("說明", { exact: true })).toHaveValue("Keep this draft 保留草稿");
  await page.screenshot({ path: testInfo.outputPath("studio-service-draft-zh-TW.png") });

  await page.goto("/billing?mock=1");
  await expect(page.getByRole("heading", { name: "計費與用量", exact: true })).toBeVisible();
  await page.goto("/admin/users?mock=1");
  await expect(page.getByRole("heading", { name: "使用者管理", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "建立使用者", exact: true })).toBeVisible();
  await settings.getByRole("combobox", { name: "語言", exact: true }).selectOption("en");
  await expect(page.getByRole("button", { name: "Create User", exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});
