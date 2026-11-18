import { expect, test } from "@playwright/test";
import { openAssistant } from "./helpers";

for (const width of [1280, 320]) {
  test(`Agent Details groups saved accounts by service at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await page.route("**/api/v1/assistant/nyxagent/cma-sessions?**", route => route.fulfill({ json: { sessions: [] } }));
    await page.route("**/api/v1/assistant/nyxagent/machines?**", route => route.fulfill({ json: [] }));
    await page.route("**/api/v1/saved-logins?**", route => route.fulfill({ json: { logins: [] } }));
    await page.route("**/api/v1/triggers", route => route.fulfill({ json: { triggers: [] } }));
    await page.route("**/api/v1/service-preferences", (route) => route.fulfill({
      json: { groups: [], version: 0, updated_at: null },
    }));
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await page.getByRole("region", { name: "What NyxBot remembers" }).getByRole("button", { name: "Manage" }).click();
    const sheet = page.getByRole("dialog", { name: "Agent details" });
    await sheet.getByRole("tab", { name: "Access", exact: true }).click();
    await sheet.locator('[data-drawer-page-trigger="grants/connections"]').click();
    const connections = sheet.getByRole("region", { name: "Connections", exact: true })
      .filter({ has: page.getByRole("list", { name: "OpenAI connections", exact: true }) }).last();
    await connections.scrollIntoViewIfNeeded();
    await expect(connections).toContainText("Your saved connection order applies across your agents");
    const accounts = connections.getByRole("list", { name: "OpenAI connections" });
    await expect(accounts).toContainText("Shared GPT-4o");
    await expect(accounts).toContainText("ChronoAI");
    await expect(accounts).toContainText("Personal");
    await expect(connections.getByRole("link", { name: "Manage connected accounts" })).toBeVisible();
    const bounds = (await connections.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    expect(await connections.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath("agent-connections.png"), animations: "disabled" });
    await sheet.getByRole("tab", { name: "Channels", exact: true }).click();
    await sheet.getByRole("button", { name: "Connect another bot", exact: true }).click();
    const heading = sheet.getByRole("heading", { name: "Connect another bot", exact: true });
    await expect(heading).toBeFocused();
    await expect(heading.locator("..")).not.toHaveAttribute("data-compact-chrome");
    await page.screenshot({ path: testInfo.outputPath("agent-detail-page.png"), animations: "disabled" });
    await sheet.getByRole("button", { name: "Back to Channels", exact: true }).click();
    await expect(sheet.getByRole("button", { name: "Connect another bot", exact: true })).toBeFocused();
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
