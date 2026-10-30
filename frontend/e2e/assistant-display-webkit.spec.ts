import { expect, test } from "@playwright/test";
import { composerInput, openAccountMenu, openAssistant } from "./helpers";

test.use({ browserName: "webkit", viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

test("iOS inputs keep the zoom-prevention minimum and grow with larger text", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  for (const [key, scale] of [["Home", 0.5], ["End", 2]] as const) {
    await openAccountMenu(page);
    await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Account Settings" });
    await dialog.getByRole("tab", { name: "Display", exact: true }).click();
    const slider = dialog.getByRole("slider", { name: "Text size", exact: true });
    await slider.press(key);
    await expect(slider).toHaveValue(String(scale));
    await page.keyboard.press("Escape");
    await expect.poll(() => composerInput(page).evaluate((el) => parseFloat(getComputedStyle(el).fontSize))).toBe(Math.max(16, 16 * scale));
  }
});
