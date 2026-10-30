import { expect, test, type Locator, type Page } from "@playwright/test";
import { composerInput, openAccountMenu, openAssistant, sendMessage } from "./helpers";

test.beforeEach(async ({ page }) => {
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) => route.fulfill({ json: [] }));
});

test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status !== testInfo.expectedStatus) await page.screenshot({ path: testInfo.outputPath("display-failure.png") });
});

async function openDisplay(page: Page) {
  await openAccountMenu(page);
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Account Settings" });
  await dialog.getByRole("tab", { name: "Display", exact: true }).click();
  return dialog;
}

async function chooseScale(dialog: Locator, label: string, value: number) {
  const slider = dialog.getByRole("slider", { name: label, exact: true });
  await slider.press("Home");
  if (value === 2) await slider.press("End");
  else for (let step = 0; step < Math.round((value - 0.5) * 100); step++) await slider.press("ArrowRight");
  await expect(slider).toHaveValue(String(value));
}

async function expectContained(element: Locator) {
  const bounds = await element.evaluate((el) => {
    const rect = el.getBoundingClientRect();
    return {
      width: el.clientWidth,
      scrollWidth: el.scrollWidth,
      overflowing: Array.from(el.querySelectorAll("*")).filter((child) => child.getBoundingClientRect().right > rect.right + 1).slice(0, 8).map((child) => child.outerHTML.slice(0, 200)),
    };
  });
  expect(bounds.scrollWidth, JSON.stringify(bounds.overflowing)).toBeLessThanOrEqual(bounds.width + 1);
}

for (const width of [320, 1280]) {
  for (const [textScale, spacing] of [[0.5, 2], [1, 1], [1.83, 0.83], [2, 0.5], [2, 2]]) {
    test(`display choices remain usable at ${width}px, text ${textScale}× and spacing ${spacing}×`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width, height: 900 });
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      const message = `A readable message with a long identifier: ${"abcdefghij".repeat(35)}`;
      await sendMessage(page, message);
      await expect(page.getByText("Your connected services are ready.", { exact: false })).toBeVisible();
      await composerInput(page).fill(Array.from({ length: 8 }, (_, index) => `Draft line ${index + 1}`).join("\n"));

      const dialog = await openDisplay(page);
      await chooseScale(dialog, "Spacing", spacing);
      await chooseScale(dialog, "Text size", textScale);
      await expect(dialog.getByRole("slider", { name: "Text size", exact: true })).toHaveValue(String(textScale));
      await expectContained(dialog);
      const theme = dialog.getByRole("radiogroup", { name: "Theme", exact: true });
      for (const option of ["Dark", "Light", "System"]) {
        const button = theme.getByRole("radio", { name: option, exact: true });
        await button.click();
        await expect(button).toHaveAttribute("aria-checked", "true");
        const size = await button.evaluate((el) => ({ height: el.clientHeight, lineHeight: parseFloat(getComputedStyle(el).lineHeight) }));
        expect(size.height).toBeGreaterThanOrEqual(size.lineHeight);
      }
      const displayTab = dialog.getByRole("tab", { name: "Display", exact: true });
      const tabBounds = await displayTab.boundingBox();
      const stripBounds = await dialog.getByRole("tablist").boundingBox();
      expect(tabBounds!.height).toBeLessThanOrEqual(stripBounds!.height + 1);
      await dialog.getByRole("button", { name: "Reset display settings" }).scrollIntoViewIfNeeded();
      if (textScale === 2 && spacing === 0.5) {
        const screenshot = testInfo.outputPath("large-text-compact-spacing.png");
        await page.screenshot({ path: screenshot });
        await testInfo.attach("large-text-compact-spacing", { path: screenshot, contentType: "image/png" });
      }
      await page.keyboard.press("Escape");

      const input = composerInput(page);
      await expect.poll(() => input.evaluate((el) => parseFloat(getComputedStyle(el).fontSize))).toBe(16 * textScale);
      await expect.poll(() => input.evaluate((el) => ({
        height: el.clientHeight,
        content: el.scrollHeight,
        overflow: getComputedStyle(el).overflowY,
      }))).toMatchObject({ overflow: "scroll" });
      const bubble = page.getByText(message, { exact: true });
      expect(await bubble.evaluate((el) => parseFloat(getComputedStyle(el).fontSize))).toBe(16 * textScale);
      await expectContained(bubble);
      await expectContained(page.getByRole("main"));
      const send = await page.getByRole("button", { name: "Send message", exact: true }).boundingBox();
      expect(send!.x).toBeGreaterThanOrEqual(0);
      expect(send!.x + send!.width).toBeLessThanOrEqual(width);
      expect(send!.y + send!.height).toBeLessThanOrEqual(900);

      await page.reload();
      await expect.poll(() => composerInput(page).evaluate((el) => parseFloat(getComputedStyle(el).fontSize))).toBe(16 * textScale);
      const reopened = await openDisplay(page);
      await expect(reopened.getByRole("slider", { name: "Text size", exact: true })).toHaveValue(String(textScale));
      await expect(reopened.getByRole("slider", { name: "Spacing", exact: true })).toHaveValue(String(spacing));
      await reopened.getByRole("button", { name: "Reset display settings" }).click();
      await expect(reopened.getByRole("slider", { name: "Text size", exact: true })).toHaveValue("1");
      await expect(reopened.getByRole("slider", { name: "Spacing", exact: true })).toHaveValue("1");
    });
  }
}
