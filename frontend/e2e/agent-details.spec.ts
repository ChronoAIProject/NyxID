import { expect, test, type Page } from "@playwright/test";
import { openAssistant } from "./helpers";

test.beforeEach(async ({ page }) => {
  await page.route("**/api/v1/triggers", (route) => route.fulfill({ json: { triggers: [] } }));
  await page.route("**/api/v1/assistant/nyxagent/cma-sessions?**", (route) => route.fulfill({ json: { sessions: [] } }));
});

async function openResearcherDetails(page: Page) {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.getByRole("region", { name: "Your agents" }).getByRole("button", { name: "Chat with researcher" }).click();
  await page.getByRole("button", { name: "Agent details", exact: true }).click();
  return page.getByRole("dialog", { name: "Agent details" });
}

for (const width of [390, 1280]) {
  test(`Agent details retains moved drafts, reviews scopes and returns from nested pages at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const sheet = await openResearcherDetails(page);
    const instructions = sheet.getByRole("textbox", { name: "Instructions", exact: true });
    await instructions.fill("Review urgent issues carefully.");
    await sheet.getByRole("button", { name: /Approval behavior:/ }).click();
    await expect(sheet.getByRole("tab", { name: "Access", exact: true })).toHaveAttribute("aria-selected", "true");
    await sheet.getByRole("combobox", { name: "Agent approval behavior" }).click();
    await page.getByRole("option", { name: "Ask every time", exact: true }).click();
    const searchToggle = sheet.getByRole("button", { name: /Search researcher agent details/ });
    await searchToggle.click();
    const search = sheet.getByRole("searchbox");
    await search.fill("channel progress");
    await search.press("ArrowDown");
    await expect(sheet.getByRole("button", { name: "Channel progress reply Channels" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(sheet.getByRole("tab", { name: "Channels", exact: true })).toHaveAttribute("aria-selected", "true");
    await sheet.getByRole("switch", { name: "Channel progress reply" }).check();
    const bar = sheet.getByRole("region", { name: "Unsaved changes" });
    await expect(bar).toContainText("profile");
    await expect(bar).toContainText("Approval behavior");
    await expect(bar).toContainText("Channel progress reply");
    await sheet.getByRole("button", { name: "Connect another bot" }).click();
    await sheet.getByRole("button", { name: "Back to Channels" }).click();
    await expect(sheet.getByRole("button", { name: "Connect another bot" })).toBeFocused();
    await expect(sheet.getByRole("switch", { name: "Channel progress reply" })).toBeChecked();
    await sheet.getByRole("button", { name: "Save changes", exact: true }).click();
    const review = page.getByRole("dialog", { name: "Review changes" });
    await expect(review).toContainText("Instructions");
    await expect(review).toContainText("Ask every time");
    await expect(review).toContainText("Channel progress reply");
    await review.getByRole("button", { name: "Keep editing" }).click();
    await expect(review).toHaveCount(0);
    await expect(bar).toBeVisible();
    const bounds = (await bar.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    await page.screenshot({ path: testInfo.outputPath("retained-drafts.png") });
    await sheet.getByRole("button", { name: "Save changes", exact: true }).click();
    await review.getByRole("button", { name: "Save changes", exact: true }).click();
    await expect(bar).toHaveCount(0);
    await sheet.getByRole("button", { name: "Close agent details" }).click();
    await page.reload();
    await page.getByRole("button", { name: "Agent details", exact: true }).click();
    await expect(instructions).toHaveValue("Review urgent issues carefully.");
    await sheet.getByRole("tab", { name: "Access", exact: true }).click();
    await expect(sheet.getByRole("combobox", { name: "Agent approval behavior" })).toHaveText("Ask every time");
    await sheet.getByRole("tab", { name: "Channels", exact: true }).click();
    await expect(sheet.getByRole("switch", { name: "Channel progress reply" })).toBeChecked();
  });
}

test("registration returns to the selected agent, preserves a draft on cancel, and connects the registered bot", async ({ page }) => {
  const sheet = await openResearcherDetails(page);
  const instructions = sheet.getByRole("textbox", { name: "Instructions", exact: true });
  await instructions.fill("Keep this working draft.");
  await sheet.getByRole("tab", { name: "Channels", exact: true }).click();
  await sheet.getByRole("button", { name: "Register a bot", exact: true }).click();
  const registration = page.getByRole("dialog", { name: "Add Channel Bot" });
  await registration.getByRole("button", { name: "Cancel", exact: true }).click();
  await sheet.getByRole("tab", { name: "Overview", exact: true }).click();
  await expect(instructions).toHaveValue("Keep this working draft.");
  await sheet.getByRole("tab", { name: "Channels", exact: true }).click();
  await sheet.getByRole("button", { name: "Register a bot", exact: true }).click();
  await registration.getByLabel("Label", { exact: true }).fill("New review helper");
  await registration.getByLabel("Bot token", { exact: true }).fill("demo-bot-token");
  await registration.getByRole("button", { name: "Add Bot", exact: true }).click();
  const completed = page.getByRole("dialog", { name: /Bot Created/ });
  await expect(completed).toContainText("Registration is complete.");
  await completed.getByRole("button", { name: "Continue to connection setup" }).click();
  await expect(registration).toHaveCount(0);
  await sheet.getByRole("button", { name: "Connect New review helper to researcher" }).click();
  await sheet.getByRole("button", { name: "Back to Channels" }).click();
  await expect(sheet.getByRole("list", { name: "Connected channel bots" })).toContainText("New review helper");
  await expect(sheet.getByRole("region", { name: "Owner link" })).toBeVisible();
  await sheet.getByRole("tab", { name: "Overview", exact: true }).click();
  await expect(instructions).toHaveValue("Keep this working draft.");
  await sheet.getByRole("button", { name: "Discard", exact: true }).click();
  await sheet.getByRole("tab", { name: "Channels", exact: true }).click();
  await expect(sheet.getByRole("list", { name: "Connected channel bots" })).toContainText("New review helper");
});
