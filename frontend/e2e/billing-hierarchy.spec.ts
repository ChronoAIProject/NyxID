import { expect, test } from "@playwright/test";
import { billingApi } from "./billing-api-fixtures";
import { clarityTheme, disclosureIcons, sectionAddress } from "./settings-clarity-fixtures";
import { BILLING_SECTIONS, BILLING_SECTION_TITLES } from "../src/lib/billing-sections";

// Each case captures every retained page as well as exercising navigation.
test.setTimeout(90_000);

for (const width of [1280, 390])
  for (const mode of ["light", "dark"] as const) {
    test(`Billing hierarchy ${width}px ${mode}`, async ({ page }, info) => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await clarityTheme(page, width, mode);
      await billingApi(page);
      await page.goto("/billing");
      const initialLength = await page.evaluate(() => history.length);
      await expect(page.locator("button[data-drawer-page-trigger]:visible")).toHaveCount(5);
      await expect(page.getByRole("table")).toHaveCount(0);
      await disclosureIcons(page.locator(".billing-page"), true);
      await page.screenshot({ path: info.outputPath("billing-root.png"), animations: "disabled" });
      await page.locator('[data-drawer-page-trigger="billing/benefits"]').click();
      await page.locator('[data-drawer-page-trigger="billing/benefits/allowances"]').click();
      await expect(
        page.getByRole("heading", { name: "Free allowances", exact: true }).first(),
      ).toBeFocused();
      await page.reload();
      await page.getByRole("button", { name: "Benefits & allowances", exact: true }).click();
      await expect(
        page.locator('[data-drawer-page-trigger="billing/benefits/allowances"]'),
      ).toBeFocused();
      expect(await page.evaluate(() => history.length)).toBe(initialLength);
      await page.getByRole("tab", { name: "Usage", exact: true }).click();
      expect(await page.evaluate(() => history.length)).toBe(initialLength + 1);
      await page.goBack();
      await expect(page.getByRole("heading", { name: "Benefits & allowances", exact: true })).toBeVisible();
      await expect(page.getByRole("tab", { name: "Billing", exact: true })).toHaveAttribute("data-state", "active");
      await page.goForward();
      await expect(page.getByRole("tab", { name: "Usage", exact: true })).toHaveAttribute("data-state", "active");
      await expect(page.getByRole("dialog", { name: "Add credits", exact: true })).toHaveCount(0);
      for (const section of BILLING_SECTIONS) {
        await sectionAddress(page, "/billing", {
          tab: section.split("/")[0],
          section: section.includes("/") ? section : undefined,
        });
        if (section.includes("/"))
          await expect(
            page
              .getByRole("heading", { name: BILLING_SECTION_TITLES[section], exact: true })
              .first(),
          ).toBeVisible();
        await disclosureIcons(page.locator(".billing-page"), !section.includes("/"));
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
          true,
        );
        await page.screenshot({
          path: info.outputPath(`${section.replaceAll("/", "-")}.png`),
          animations: "disabled",
        });
      }
      await sectionAddress(page, "/assistant", {
        panel: "billing",
        panelTab: "usage",
        panelSection: "usage/charts/funding",
        panelPeriod: "7d",
      });
      const panel = page
        .getByRole("dialog")
        .filter({ has: page.getByRole("tablist", { name: "Billing and usage" }) });
      await expect(panel).toBeVisible();
      await panel.getByRole("button", { name: "Usage charts", exact: true }).click();
      await panel.getByRole("button", { name: "Usage", exact: true }).click();
      await expect(page).toHaveURL(/panelPeriod=7d/);
      const panelLength = await page.evaluate(() => history.length);
      await panel.getByRole("tab", { name: "Billing", exact: true }).click();
      expect(await page.evaluate(() => history.length)).toBe(panelLength + 1);
      await page.goBack();
      await expect(panel.getByRole("tab", { name: "Usage", exact: true })).toHaveAttribute("data-state", "active");
      await page.goForward();
      await expect(panel.getByRole("tab", { name: "Billing", exact: true })).toHaveAttribute("data-state", "active");
      await panel.getByRole("button", { name: "Close", exact: true }).click();
      await page.getByRole("button", { name: "Search", exact: true }).first().click();
      await page.getByRole("combobox").fill("Past usage invoices");
      await page
        .getByRole("option", { name: "This period's bill Billing & Usage", exact: true })
        .click();
      await expect(
        page.getByRole("dialog", { name: "This period's bill", exact: true }),
      ).toBeVisible();
      await page.goBack();
      await expect(page.getByRole("dialog")).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }

for (const shell of ["studio", "assistant"] as const) {
  test(`Billing ${shell} opening and consumed checkout keep browser history`, async ({ page }) => {
    await billingApi(page);
    await page.goto("/billing?tab=usage");
    await expect(page.getByRole("tab", { name: "Usage", exact: true })).toHaveAttribute("data-state", "active");
    const previous = page.url();
    const initialLength = await page.evaluate(() => history.length);
    const path = shell === "assistant" ? "/assistant" : "/billing";
    const search = shell === "assistant" ? { panel: "billing", panelTab: "billing" } : { tab: "billing" };
    await sectionAddress(page, path, search, false);
    await expect(page.getByRole("tab", { name: "Billing", exact: true })).toHaveAttribute("data-state", "active");
    expect(await page.evaluate(() => history.length)).toBe(initialLength + 1);
    await page.goBack();
    await expect(page).toHaveURL(previous);
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.goForward();
    await expect(page.getByRole("tab", { name: "Billing", exact: true })).toHaveAttribute("data-state", "active");
    await sectionAddress(page, path, { ...search, [shell === "assistant" ? "panelAction" : "action"]: "topup" }, false);
    const checkout = page.getByRole("dialog", { name: "Add credits", exact: true });
    await expect(checkout).toBeVisible();
    await expect(page).not.toHaveURL(/(?:action|panelAction)=/);
    await checkout.getByRole("button", { name: "Cancel", exact: true }).click();
    if (shell === "assistant")
      await page.getByRole("dialog", { name: "Billing & Usage", exact: true }).getByRole("button", { name: "Close", exact: true }).click();
    else
      await page.getByRole("tab", { name: "Usage", exact: true }).click();
    await page.goBack();
    await expect(page.getByRole("tab", { name: "Billing", exact: true })).toHaveAttribute("data-state", "active");
    await expect(checkout).toHaveCount(0);
    await expect(page).not.toHaveURL(/(?:action|panelAction)=/);
    await page.goForward();
    await expect(checkout).toHaveCount(0);
    await page.goBack();
    await expect(page.getByRole("tab", { name: "Billing", exact: true })).toHaveAttribute("data-state", "active");
    await expect(checkout).toHaveCount(0);
  });
}
