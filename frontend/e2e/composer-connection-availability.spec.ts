import { expect, test, type Page } from "@playwright/test";
import type { KeyInfo } from "../src/types/keys";

async function openPicker(page: Page) {
  await page.getByRole("button", { name: "Add context" }).click();
  await page.getByRole("menuitem", { name: "Use a connection" }).click();
  return page.getByRole("dialog", { name: "Connection for this message" });
}

async function update(page: Page, patch: Partial<KeyInfo>) {
  await page.evaluate(async (value) => {
    const { updateConnection } =
      await import("/e2e/fixtures/composer-connections-state.ts");
    updateConnection("work", value);
  }, patch);
}

test.beforeEach(async ({ page }) => {
  await page.context().grantPermissions(["local-network-access"]);
  await page.route("**/api/**", (route) => route.abort());
  await page.goto("/e2e/fixtures/composer-connections.html");
});

test("disabled choices expose reasons and keyboard skips them; unverified, refresh and fallback stay selectable", async ({
  page,
}, testInfo) => {
  const dialog = await openPicker(page);
  const missing = dialog.getByRole("button", { name: /Missing credential/ });
  await expect(missing).toBeDisabled();
  await expect(missing).toHaveAccessibleDescription("Credential missing");
  await missing.click({ force: true });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: /Node without fallback/ }),
  ).toBeDisabled();
  await expect(
    dialog.getByRole("button", { name: /Unavailable platform/ }),
  ).toBeDisabled();
  for (const label of [
    "Unverified account",
    "Refreshable OAuth",
    "Node with fallback",
  ]) {
    await expect(
      dialog.getByRole("button", { name: new RegExp(label) }),
    ).toBeEnabled();
  }
  await page.screenshot({
    path: testInfo.outputPath("connection-availability.png"),
    fullPage: true,
    animations: "disabled",
  });
  await dialog.getByRole("textbox", { name: "Search connections" }).focus();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: /Work documents/ }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByRole("group", { name: "Connection for this message" }),
  ).toContainText("Work documents");
});

test("a focused option becoming unavailable cannot be selected by Enter or Space", async ({
  page,
}) => {
  const dialog = await openPicker(page);
  const work = dialog.getByRole("button", { name: /Work documents/ });
  await work.focus();
  await update(page, { credential_missing: true });
  await expect(work).toBeDisabled();
  await expect(work).toHaveAccessibleDescription("Credential missing");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Space");
  await expect(dialog).toBeVisible();
  await expect(
    page.getByRole("group", { name: "Connection for this message" }),
  ).toHaveCount(0);
  await dialog.getByRole("textbox", { name: "Search connections" }).focus();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: /Personal documents/ }),
  ).toBeFocused();
});

test("live selection updates its label, refuses mouse and keyboard sends on invalidation, and switches only explicitly", async ({
  page,
}, testInfo) => {
  const dialog = await openPicker(page);
  await dialog.getByRole("button", { name: /Work documents/ }).click();
  const chip = page.getByRole("group", { name: "Connection for this message" });
  await update(page, { label: "Renamed account" });
  await expect(chip).toContainText("Renamed account");
  await expect(chip).not.toContainText("Work documents");
  await update(page, { status: "refresh_failed" });
  await expect(chip.getByRole("status")).toContainText("Reconnect needed");
  const input = page.locator("[data-composer-textbox]");
  await input.fill("Keep this draft");
  await page.getByRole("button", { name: "Send message" }).click();
  await expect(input).toHaveText("Keep this draft");
  await input.press("Enter");
  await expect(input).toHaveText("Keep this draft");
  await expect(page.getByLabel("Submitted messages")).toHaveText("[]");
  await page.screenshot({
    path: testInfo.outputPath("selected-connection-unavailable.png"),
    fullPage: true,
    animations: "disabled",
  });
  await openPicker(page);
  await dialog.getByRole("button", { name: /Personal documents/ }).click();
  await input.press("Enter");
  await expect(page.getByLabel("Submitted messages")).toHaveText(
    JSON.stringify(["/service personal\nKeep this draft"]),
  );
  await expect(chip).toHaveCount(0);
});
