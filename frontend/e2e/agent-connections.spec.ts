import { expect, test } from "@playwright/test";
import { openAssistant } from "./helpers";

for (const width of [1280, 320]) {
  test(`Agent Details groups saved accounts by service at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await page.route("**/api/v1/service-preferences", (route) => route.fulfill({
      json: { groups: [], version: 0, updated_at: null },
    }));
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await page.getByRole("region", { name: "What NyxBot remembers" }).getByRole("button", { name: "Manage" }).click();
    const sheet = page.getByRole("dialog", { name: "Agent details" });
    const connections = sheet.getByRole("region", { name: "Connections" });
    await connections.scrollIntoViewIfNeeded();
    await expect(connections).toContainText("Your saved connection order applies across your agents");
    await connections.locator("summary").filter({ hasText: /^OpenAI/ }).click();
    const accounts = connections.getByRole("list", { name: "OpenAI connections" });
    await expect(accounts).toContainText("Shared GPT-4o");
    await expect(accounts).toContainText("ChronoAI");
    await expect(accounts).toContainText("Personal");
    await expect(connections.getByRole("link", { name: "Manage connected accounts" })).toBeVisible();
    const bounds = (await connections.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    expect(await connections.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath("agent-connections.png") });
  });
}

test("Add context keeps file attachment available beside account selection", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.getByRole("button", { name: "Add context" }).click();
  const menu = page.getByRole("menu", { name: "Add context" });
  await expect(menu.getByRole("menuitem", { name: "Use a connection" })).toBeVisible();
  const chooser = page.waitForEvent("filechooser");
  await menu.getByRole("menuitem", { name: "Attach files" }).click();
  expect((await chooser).isMultiple()).toBe(true);
});
