import { expect, test } from "@playwright/test";
import { composerInput, openAssistant } from "./helpers";

const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=", "base64");

for (const width of [1280, 390, 320]) {
  test(`attachments stay in one scrollable row at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, { faults: { nyxagentEnabled: true, modelControlsEnabled: false } });
    await page.getByLabel("Choose attachments").setInputFiles([
      { name: "Reference.png", mimeType: "image/png", buffer: png },
      ...Array.from({ length: 7 }, (_, index) => ({
        name: `Project notes ${index + 1}.md`, mimeType: "text/markdown", buffer: Buffer.from("# Project notes\nA useful document to review."),
      })),
    ]);
    await composerInput(page).evaluate((element) => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", Array.from({ length: 10 }, (_, index) => `Research note ${index + 1}: keep the composer compact.`).join("\n"));
      element.dispatchEvent(new ClipboardEvent("paste", { clipboardData, bubbles: true }));
    });
    const strip = page.getByRole("list", { name: "Attachments", exact: true });
    await expect(strip.getByRole("status")).toHaveText(Array(9).fill("Ready"));
    await expect(page.getByRole("button", { name: "Send message", exact: true })).toBeEnabled();
    const layout = await strip.evaluate((element) => ({
      width: element.clientWidth, scrollWidth: element.scrollWidth,
      height: element.getBoundingClientRect().height,
      tops: Array.from(element.children, (child) => child.getBoundingClientRect().top),
    }));
    expect(layout.scrollWidth).toBeGreaterThan(layout.width);
    const leftFade = page.locator('[data-attachment-fade="left"]');
    const rightFade = page.locator('[data-attachment-fade="right"]');
    // A new paste reveals the last card, including when files were added in a batch.
    await expect.poll(() => strip.evaluate((element) =>
      element.scrollWidth - element.clientWidth - element.scrollLeft,
    )).toBeLessThanOrEqual(1);
    await expect(leftFade).toHaveCSS("opacity", "1");
    await expect(rightFade).toHaveCSS("opacity", "0");
    await strip.evaluate((element) => { element.scrollLeft = 0; });
    await expect(leftFade).toHaveCSS("opacity", "0");
    await expect(rightFade).toHaveCSS("opacity", "1");
    expect(layout.height).toBeLessThanOrEqual(120);
    expect(new Set(layout.tops).size).toBe(1);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await page.getByRole("button", { name: "Preview Reference.png" }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await page.getByRole("button", { name: "Close preview" }).click();
    // Removing an attachment must preserve the user's scroll position.
    await page.getByRole("button", { name: "Remove Reference.png" }).click();
    await expect(strip.getByRole("listitem")).toHaveCount(8);
    expect(await strip.evaluate((element) => element.scrollLeft)).toBe(0);
    await strip.focus();
    await page.keyboard.press("ArrowRight");
    await expect.poll(() => strip.evaluate((element) => element.scrollLeft)).toBeGreaterThan(0);
    await strip.evaluate((element) => { element.scrollLeft = element.scrollWidth; });
    await expect(leftFade).toHaveCSS("opacity", "1");
    await expect(rightFade).toHaveCSS("opacity", "0");
    await page.getByRole("button", { name: "Remove Project notes 7.md" }).click();
    await expect(strip.getByRole("listitem")).toHaveCount(7);
    await expect(strip.getByRole("alert")).toHaveCount(0);
    await composerInput(page).fill("Review these attachments.");
    await strip.evaluate((element) => { element.scrollLeft = 0; });
    await page.screenshot({ path: testInfo.outputPath(`attachments-${width}.png`) });
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    await expect(strip).toHaveCount(0);
    await expect(page).toHaveURL(/c=nyxa-/);
    // The uploaded metadata survives the turn projection into the transcript.
    await expect(page.getByText("Project notes 1.md", { exact: true }).first()).toBeVisible();
  });
}
