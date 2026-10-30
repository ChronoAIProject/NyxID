// Browser-only acceptance at a controlled HTTP fixture boundary. This does not
// exercise backend authorization, retention workers, provider effects or storage.
import { test, expect } from "@playwright/test";
import { openAssistant } from "./helpers";


for (const width of [1440, 390]) {
  test(`creation menu keyboard and Escape preserve modal dialog and page interaction at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.route("**/api/v1/assistant/nyxagent/machines?*", route => route.fulfill({ json: [] }));
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    if (width === 390) await page.getByRole("button", { name: "Open chats" }).click();
    const nav = page.getByRole("navigation", { name: "Agents and conversations" }).last();
    const trigger = nav.getByRole("button", { name: "Create new agent" });
    await trigger.focus();
    await trigger.press("Enter");
    const personal = page.getByRole("menuitem", { name: "Personal agent", exact: true });
    await expect(personal).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await trigger.press("Enter");
    await personal.press("Enter");
    const dialog = page.getByRole("dialog", { name: "New agent", exact: true });
    await expect(dialog).toBeVisible();
    await expect.poll(() => page.evaluate(() => getComputedStyle(document.body).pointerEvents)).toBe("none");
    await dialog.getByRole("textbox", { name: "Name", exact: true }).fill("keyboard-cancelled-agent");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => getComputedStyle(document.body).pointerEvents)).not.toBe("none");
    // The drawer owns the dialogs; it must remain available after cancellation.
    await expect(trigger).toBeVisible();
    await trigger.click();
    await expect(personal).toBeVisible();
    // Reopening must also lead to a second usable dialog, without a delayed
    // dismiss from the previous menu's exit presence.
    await personal.click();
    await expect(dialog).toBeVisible();
    await dialog.getByRole("textbox", { name: "Name", exact: true }).fill("second-cancelled-agent");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => getComputedStyle(document.body).pointerEvents)).not.toBe("none");
    await nav.getByRole("button", { name: "NyxBot — your personal agent" }).click();
    await expect(page.getByRole("heading", { level: 1, name: /^Good (morning|afternoon|evening), Dannick$/ })).toBeVisible();
    await expect(page.getByRole("button", { name: /^keyboard-cancelled-agent, specialist/ })).toHaveCount(0);
    await expect(page.getByRole("button", { name: /^second-cancelled-agent, specialist/ })).toHaveCount(0);
  });
}
