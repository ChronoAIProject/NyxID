import { expect, test } from "@playwright/test";
import { composerInput, openAssistant } from "./helpers";

for (const width of [1280, 390]) {
  test(`language changes survive navigation and reload at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    const editor = composerInput(page);
    await editor.fill("Keep my draft 保留草稿");
    await page
      .getByRole("button", { name: width < 768 ? "User menu" : "Account menu" })
      .click();
    await expect(page.getByRole("menuitem", { name: "Language", exact: true })).toHaveCount(0);
    await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
    await page.getByRole("combobox", { name: "Language", exact: true }).selectOption("zh-CN");
    await page.getByRole("button", { name: "关闭", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
    await expect(
      page.getByRole("button", { name: "发送消息", exact: true }),
    ).toBeVisible();
    await expect(editor).toHaveText("Keep my draft 保留草稿");

    await page
      .getByRole("button", { name: width < 768 ? "用户菜单" : "账号菜单" })
      .click();
    await page.getByRole("menuitem", { name: "设置", exact: true }).click();
    await page.getByRole("combobox", { name: "语言", exact: true }).selectOption("zh-TW");
    await page.getByRole("button", { name: "關閉", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-TW");
    await expect(
      page.getByRole("button", { name: "傳送訊息", exact: true }),
    ).toBeVisible();
    await expect(editor).toHaveText("Keep my draft 保留草稿");
    await page.screenshot({
      path: testInfo.outputPath(`nyxbot-zh-TW-${width}.png`),
    });

    await page
      .getByRole("button", { name: width < 768 ? "使用者選單" : "帳號選單" })
      .click();
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
