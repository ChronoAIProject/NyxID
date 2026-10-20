import { test, expect } from "@playwright/test";
import { openAssistant, sendMessage, stopButton } from "./helpers";

test.beforeEach(async ({ page }) => {
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) =>
    route.fulfill({ json: [] }),
  );
});

for (const width of [1440, 390]) {
  test(`copy preserves Markdown and exact-turn receipt after reload at ${width}px`, async ({ page, context }) => {
    await page.setViewportSize({ width, height: 900 });
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 1800 } });
    await sendMessage(page, "Review formatting and receipt");
    await expect(stopButton(page)).toBeVisible();
    await expect(page.getByRole("button", { name: "Copy message", exact: true })).toHaveCount(1);
    await expect(page.getByText("Formatting reviewed", { exact: true })).toBeVisible({ timeout: 8000 });
    await expect(stopButton(page)).not.toBeVisible();
    const receipt = page.getByRole("status").filter({ hasText: "Formatting proposal approved" });
    await expect(receipt).toHaveCount(1);
    await expect(receipt).toContainText("Allowed automatically · Full access");
    await expect(page.getByText("hidden arguments", { exact: false })).toHaveCount(0);
    await page.getByRole("button", { name: "Copy message", exact: true }).last().focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("button", { name: "Copied", exact: true })).toBeVisible();
    const expected = "**Formatting reviewed**\n\n```ts\nconst answer = 42;\n```";
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(expected);
    await page.reload();
    await expect(receipt).toHaveCount(1);
    await expect(page.getByText("Formatting reviewed", { exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => {
      const receipt = [...document.querySelectorAll('[role="status"]')].find((el) => el.textContent?.includes("Formatting proposal approved"));
      const answer = [...document.querySelectorAll("strong")].find((el) => el.textContent === "Formatting reviewed");
      const row = receipt?.closest('[id^="message-"]');
      return Boolean(answer && row && row.id.includes("nyxagent-acknowledgement:") &&
        answer.compareDocumentPosition(row) & Node.DOCUMENT_POSITION_FOLLOWING);
    })).toBe(true);
    await page.getByRole("button", { name: "Copy message", exact: true }).first().click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe("Review formatting and receipt");
  });
}

test("clipboard failure stays retryable in the real chat renderer", async ({ page }) => {
  await page.addInitScript(() => {
    let fail = true;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      writeText: () => fail ? (fail = false, Promise.reject(new Error("clipboard unavailable"))) : Promise.resolve(),
    } });
  });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  await expect(page.getByText("Repository lookup succeeded.")).toBeVisible();
  const copy = page.getByRole("button", { name: "Copy message", exact: true }).last();
  await copy.click();
  await expect(page.getByRole("status").filter({ hasText: "Copy failed. Try Copy message again." })).toBeVisible();
  await expect(copy).toBeEnabled();
  await copy.click();
  await expect(page.getByRole("button", { name: "Copied", exact: true })).toBeVisible();
  await expect(page.getByText("Copy failed. Try Copy message again.")).toHaveCount(0);
});
