import { expect, test } from "@playwright/test";
import { openAssistant, sendMessage } from "./helpers";

test("search lists recent chats and jumps to them", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "List connected services");
  await expect(page.getByText("Your connected services are ready.", { exact: false })).toBeVisible();
  const chat = new URL(page.url()).searchParams.get("c");
  expect(chat).toMatch(/^nyxa-/);
  await page.goto("/assistant?mock=1");

  await page.getByRole("button", { name: "Search", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Search" });
  await expect(dialog.getByText("Recent chats", { exact: true })).toBeVisible();
  // Before typing, only chats and pages are offered.
  await expect(dialog.getByRole("option", { name: /Settings: Display/ })).toHaveCount(0);
  await dialog.getByRole("option").filter({ hasText: "List connected services" }).first().click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => new URL(page.url()).searchParams.get("c")).toBe(chat);
});

test("search reaches pages, settings and theme commands, and opens with the keyboard", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });

  await page.keyboard.press("ControlOrMeta+k");
  const dialog = page.getByRole("dialog", { name: "Search" });
  const input = dialog.getByRole("combobox");
  await expect(input).toBeFocused();
  await input.fill("plugins");
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect(page).toHaveURL(/\/assistant\/plugins/);

  await page.getByRole("button", { name: "Search", exact: true }).click();
  await input.fill("text size");
  await expect(dialog.getByRole("option")).toHaveCount(1);
  await dialog.getByRole("option", { name: /Settings: Display/ }).click();
  const settings = page.getByRole("dialog", { name: "Account Settings" });
  await expect(settings.getByRole("tab", { name: "Display" })).toHaveAttribute("data-state", "active");
  await page.keyboard.press("Escape");
  await expect(settings).toHaveCount(0);

  await page.keyboard.press("ControlOrMeta+k");
  await input.fill("theme dark");
  await page.keyboard.press("Enter");
  await expect(page.locator("html")).toHaveClass(/theme-dark/);

  await page.keyboard.press("ControlOrMeta+k");
  await input.fill("no such thing anywhere");
  await expect(dialog.getByText("No results.")).toBeVisible();
});

test("search opens from the mobile top bar", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.getByRole("button", { name: "Search", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Search" });
  await dialog.getByRole("combobox").fill("approvals");
  await dialog.getByRole("option", { name: /Approvals/ }).click();
  await expect(page).toHaveURL(/\/assistant\/approvals/);
});
