import { expect, test } from "@playwright/test";
import { openAccountMenu, openAssistant } from "./helpers";
import { clarityTheme, disclosureIcons, sectionAddress, settingsApi } from "./settings-clarity-fixtures";
import {
  ACCOUNT_SETTINGS_SECTIONS,
  ACCOUNT_SETTINGS_TITLES,
} from "../src/lib/account-settings-sections";

// Each case captures every retained page as well as exercising navigation.
test.setTimeout(90_000);

for (const width of [1280, 390])
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
      await expect(panel.locator("button[data-drawer-page-trigger]")).toHaveCount(6);
      await disclosureIcons(panel, true);
      await page.screenshot({ path: info.outputPath("root.png"), animations: "disabled" });
      await panel.getByRole("button", { name: /^Display/ }).click();
      await panel.getByRole("button", { name: "Colors", exact: true }).click();
      await panel.getByRole("button", { name: "Text colors", exact: true }).click();
      await expect(panel).toHaveAccessibleName("Text colors");
      await page.reload();
      await expect(panel).toHaveAccessibleName("Text colors");
      await panel.getByRole("button", { name: "Colors", exact: true }).click();
      await expect(panel.locator('[data-drawer-page-trigger="display/colors/text"]')).toBeFocused();
      expect(await page.evaluate(() => history.length)).toBe(initialLength + 1);
      await page.goBack();
      await expect(panel).toHaveCount(0);
      await page.goForward();
      await expect(panel).toHaveAccessibleName("Colors");
      for (const section of ACCOUNT_SETTINGS_SECTIONS) {
        await sectionAddress(page, "/assistant", { panel: "settings", panelTab: section });
        await expect(panel).toHaveAccessibleName(ACCOUNT_SETTINGS_TITLES[section]);
        await disclosureIcons(panel, false);
        await expect(
          panel
            .getByRole("heading", { name: ACCOUNT_SETTINGS_TITLES[section], exact: true })
            .first(),
        ).toBeVisible();
        expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
          true,
        );
        await page.screenshot({
          path: info.outputPath(`${section.replaceAll("/", "-")}.png`),
          animations: "disabled",
        });
      }
      await sectionAddress(page, "/assistant", { panel: "settings", panelTab: "profile" });
      await panel.getByRole("textbox", { name: "Name", exact: true }).fill("Retained draft");
      await sectionAddress(page, "/assistant", { panel: "settings", panelTab: "display/text" });
      await sectionAddress(page, "/assistant", { panel: "settings", panelTab: "profile" });
      await expect(panel.getByRole("textbox", { name: "Name", exact: true })).toHaveValue(
        "Retained draft",
      );
      await panel.getByRole("button", { name: "Discard", exact: true }).click();
      await panel.getByRole("button", { name: "Close", exact: true }).click();
      await page.getByRole("button", { name: "Search", exact: true }).first().click();
      await page.getByRole("combobox").fill("Maximum composer height");
      await page.getByRole("option", { name: "Chat layout Account Settings", exact: true }).click();
      await expect(panel).toHaveAccessibleName("Chat layout");
      await page.goBack();
      await expect(panel).toHaveCount(0);
      await page.goto("/settings?mock=1&tab=display%2Ftext");
      await expect(
        page.getByRole("heading", { name: "Account Settings", exact: true }),
      ).toBeVisible();
      await expect(
        page.getByRole("heading", { name: "Text & spacing", exact: true }),
      ).toBeVisible();
      expect(errors).toEqual([]);
    });
  }
