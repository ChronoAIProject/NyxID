import { expect, test } from "@playwright/test";
import { openAccountMenu, openAssistant } from "./helpers";
import { clarityTheme, disclosureIcons, sectionAddress, settingsApi } from "./settings-clarity-fixtures";
import {
  ACCOUNT_SETTINGS_SECTIONS,
  ACCOUNT_SETTINGS_TITLES,
} from "../src/lib/account-settings-sections";

// Each case captures every retained page as well as exercising navigation.
test.setTimeout(90_000);
test.use({ locale: "en-US", timezoneId: "UTC" });
test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date("2026-10-15T12:00:00Z"));
});

for (const width of [1280, 768, 390])
  for (const mode of ["light", "dark"] as const) {
    test(`Account hierarchy ${width}px ${mode}`, async ({ page }, info) => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await clarityTheme(page, width, mode);
      await settingsApi(page);
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      const initialLength = await page.evaluate(() => history.length);
      await openAccountMenu(page);
      await page.getByRole("menuitem", { name: "Settings", exact: true }).click();
      const panel = page
        .locator('[role="dialog"]')
        .filter({ has: page.locator('[aria-label="Account settings"]') });
      await expect(panel).toHaveAccessibleName("Account Settings");
      const wide = width >= 768;
      const navigation = panel.getByRole("navigation", { name: "Account Settings", exact: true });
      if (wide) {
        await expect(navigation).toBeVisible();
        await expect(navigation.getByRole("button", { name: "Profile", exact: true })).toHaveAttribute("aria-current", "page");
        await expect(panel.getByRole("textbox", { name: "Name", exact: true })).toHaveValue("Reader");
        expect(new URL(page.url()).searchParams.has("panelTab")).toBe(false);
      } else {
        await expect(navigation).toHaveCount(0);
        await expect(panel.locator("button[data-drawer-page-trigger]:visible")).toHaveCount(6);
        await disclosureIcons(panel, true);
      }
      await page.screenshot({ path: info.outputPath("root.png"), animations: "disabled" });
      await (wide ? navigation.getByRole("button", { name: "Display", exact: true }) : panel.getByRole("button", { name: /^Display/ })).click();
      await panel.getByRole("tab", { name: "Colors", exact: true }).click();
      await expect(panel).toHaveAccessibleName("Account Settings");
      await page.reload();
      await expect(panel).toHaveAccessibleName("Account Settings");
      if (wide) {
        await navigation.getByRole("button", { name: "Profile", exact: true }).click();
        await expect(panel.getByRole("heading", { name: "Profile", exact: true })).toBeFocused();
      } else {
        await panel.getByRole("button", { name: "Account Settings", exact: true }).click();
        await expect(panel.locator('[data-drawer-page-trigger="display"]')).toBeFocused();
      }
      expect(await page.evaluate(() => history.length)).toBe(initialLength + 1);
      await page.goBack();
      await expect(panel).toHaveCount(0);
      await page.goForward();
      await expect(panel).toHaveAccessibleName("Account Settings");
      for (const section of ACCOUNT_SETTINGS_SECTIONS) {
        await sectionAddress(page, "/assistant", { panel: "settings", panelTab: section });
        await expect(panel).toHaveAccessibleName("Account Settings");
        if (!wide) await disclosureIcons(panel, false);
        await expect(
          panel
            .getByRole("heading", { name: section.startsWith("display") ? "Display" : ACCOUNT_SETTINGS_TITLES[section], exact: true })
            .first(),
        ).toBeVisible();
        expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
          true,
        );
        if (wide && section === "display/colors") {
          const before = await navigation.boundingBox();
          const scroll = panel.locator('[data-grouped-section="display"] [data-compact-scroll]');
          await scroll.evaluate(element => { element.scrollTop = 300; });
          await expect.poll(async () => (await navigation.boundingBox())!.y).toBe(before!.y);
          await scroll.evaluate(element => { element.scrollTop = 0; });
        }
        if (section === "security") {
          await expect(panel.getByRole("status")).toHaveCount(0);
          await expect(
            panel.getByText(
              "No active onboarding bindings. Use /help in the manager’s private chat to start onboarding.",
              { exact: true },
            ),
          ).toBeVisible();
        }
        await page.screenshot({
          path: info.outputPath(`${section.replaceAll("/", "-")}.png`),
          animations: "disabled",
        });
      }
      if (wide) {
        await navigation.getByRole("searchbox", { name: "Search settings" }).fill("Maximum composer height");
        await expect(navigation.getByRole("button", { name: "Maximum composer height Display", exact: true })).toBeVisible();
        await page.screenshot({ path: info.outputPath("search.png"), animations: "disabled" });
        await navigation.getByRole("button", { name: "Maximum composer height Display", exact: true }).click();
        await expect(panel.getByRole("slider", { name: "Maximum composer height" })).toBeFocused();
        await expect(panel.getByRole("slider", { name: "Maximum composer height" })).toHaveAttribute("data-search-highlight", "");
        // Keyboard selection moves through sidebar items and activates into the content heading.
        await navigation.getByRole("button", { name: "Profile", exact: true }).focus();
        await page.keyboard.press("ArrowDown");
        await page.keyboard.press("Enter");
        await expect(panel.getByRole("heading", { name: "Security", exact: true })).toBeFocused();
      }
      await sectionAddress(page, "/assistant", { panel: "settings", panelTab: "profile" });
      await panel.getByRole("textbox", { name: "Name", exact: true }).fill("Retained draft");
      await sectionAddress(page, "/assistant", { panel: "settings", panelTab: "display" });
      await sectionAddress(page, "/assistant", { panel: "settings", panelTab: "profile" });
      await expect(panel.getByRole("textbox", { name: "Name", exact: true })).toHaveValue(
        "Retained draft",
      );
      if (wide) {
        await page.setViewportSize({ width: 390, height: 900 });
        await expect(navigation).toHaveCount(0);
        await expect(panel.getByRole("textbox", { name: "Name", exact: true })).toHaveValue("Retained draft");
        await page.setViewportSize({ width, height: 900 });
        await expect(navigation).toBeVisible();
        await expect(panel.getByRole("textbox", { name: "Name", exact: true })).toHaveValue("Retained draft");
      }
      await panel.getByRole("button", { name: "Discard", exact: true }).click();
      await panel.getByRole("button", { name: "Close", exact: true }).click();
      await page.getByRole("button", { name: "Search", exact: true }).first().click();
      await page.getByRole("combobox").fill("Maximum composer height");
      await page.getByRole("option", { name: "Layout Account Settings", exact: true }).click();
      await expect(panel).toHaveAccessibleName("Account Settings");
      await page.goBack();
      await expect(panel).toHaveCount(0);
      await page.goto("/settings?mock=1&tab=display");
      await expect(
        page.getByRole("heading", { name: "Account Settings", exact: true }),
      ).toBeVisible();
      await expect(
        page.getByRole("heading", { name: "Display", exact: true }),
      ).toBeVisible();
      const studioWide = await page.locator("[data-section-workspace]").evaluate(element => element.getBoundingClientRect().width >= 672);
      if (studioWide) {
        const studioNav = page.getByRole("navigation", { name: "Account Settings", exact: true });
        await expect(studioNav).toBeVisible();
        for (const category of ["Profile", "Security", "Sessions", "Privacy", "Display", "MCP"]) {
          await studioNav.getByRole("button", { name: category, exact: true }).click();
          await expect(page.getByRole("heading", { name: category, exact: true }).last()).toBeVisible();
          await page.screenshot({ path: info.outputPath(`studio-${category.toLowerCase()}.png`), animations: "disabled" });
        }
        await page.goto("/settings?mock=1");
        await expect(page.getByRole("textbox", { name: "Name", exact: true })).toBeVisible();
        expect(new URL(page.url()).searchParams.has("tab")).toBe(false);
      }
      else await expect(page.getByRole("navigation", { name: "Account Settings", exact: true })).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }
