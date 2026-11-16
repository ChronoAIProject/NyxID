import { expect, test } from "@playwright/test";
import { openAccountMenu, openAssistant } from "./helpers";

for (const viewport of [{ width: 1280, height: 900 }, { width: 390, height: 844 }]) {
  for (const mode of ["light", "dark"] as const) {
    test(`NyxBot hierarchy at ${viewport.width}px in ${mode}`, async ({ page }, testInfo) => {
      await page.setViewportSize(viewport);
      await page.addInitScript((theme) => localStorage.setItem("nyxid.theme", JSON.stringify({ state: { mode: theme }, version: 0 })), mode);
      await page.route("**/api/v1/triggers", route => route.fulfill({ json: { triggers: [] } }));
      await page.route("**/api/v1/assistant/nyxagent/notifications/summary", route => route.fulfill({ json: { unread_count: 0, needs_decision_count: 0, by_source: [] } }));
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await page.evaluate(() => {
        const { authStore } = (window as unknown as { __nyxApp: { authStore: typeof import("../src/stores/auth-store").useAuthStore } }).__nyxApp;
        const user = authStore.getState().user!;
        authStore.setState({ user: { ...user, capabilities: { ...user.capabilities, enabled_features: [...(user.capabilities?.enabled_features ?? []), "assistant:notifications"] } } });
      });
      await expect(page.locator("html")).toHaveClass(new RegExp(`theme-${mode}`));
      await openAccountMenu(page);
      await page.getByRole("menuitem", { name: "NyxBot settings", exact: true }).click();
      const panel = page.locator('[role="dialog"]').filter({ has: page.locator("[data-form-viewport]") });
      await expect(panel).toBeVisible();
      await expect(panel).toHaveAccessibleName("NyxBot settings");
      await expect(panel.getByRole("heading", { level: 2 })).toHaveCount(1);
      await expect(panel.getByRole("button", { name: /^Permissions/ })).toBeVisible();
      const rows = panel.locator("button[data-drawer-page-trigger]");
      await expect(rows).toHaveCount(5);
      expect(await rows.evaluateAll(elements => new Set(elements.map(row => row.closest("section"))).size)).toBe(3);
      for (const [index, title] of ["Permissions", "Tasks & specialists", "Automations", "Notifications", "Chats & channels"].entries()) {
        await expect(rows.nth(index)).toContainText(title);
        await expect(rows.nth(index).locator("svg")).toHaveCount(2);
      }
      await expect(panel.getByRole("status")).toHaveCount(0);
      await expect(panel.getByRole("alert")).toHaveCount(0);
      await expect(panel.getByText("NyxBot runs with full access to your connected services and account; specialists use only what you or NyxBot grant them. These settings apply to all of your agents.", { exact: true })).toBeVisible();
      await expect(panel.getByRole("spinbutton")).toHaveCount(0);
      await page.screenshot({ path: testInfo.outputPath("root.png"), animations: "disabled" });
      const bounds = await panel.boundingBox();
      expect(bounds!.width).toBeLessThanOrEqual(viewport.width);
      await panel.getByRole("button", { name: "Tasks & specialists", exact: true }).click();
      await panel.getByRole("button", { name: "Specialists", exact: true }).click();
      await expect(panel).toHaveAccessibleName("Specialists");
      await expect(panel.getByRole("heading", { level: 2 })).toHaveCount(1);
      await expect(panel.getByRole("heading", { name: "Specialists", exact: true })).toBeFocused();
      const field = panel.getByRole("spinbutton", { name: "Live specialists" });
      await field.fill("10");
      await expect(panel.getByRole("button", { name: "Save changes", exact: true })).toBeVisible();
      await page.screenshot({ path: testInfo.outputPath("specialists-draft.png"), animations: "disabled" });
      await panel.getByRole("button", { name: "Tasks & specialists", exact: true }).click();
      const guard = page.getByRole("dialog", { name: "Discard unsaved changes?" });
      await expect(guard).toBeVisible();
      await guard.getByRole("button", { name: "Keep editing" }).click();
      await expect(field).toHaveValue("10");
      await panel.getByRole("button", { name: "Discard", exact: true }).click();
      await panel.getByRole("button", { name: "Tasks & specialists", exact: true }).click();
      await expect(panel.getByRole("button", { name: "Specialists", exact: true })).toBeFocused();
      await panel.getByRole("button", { name: "NyxBot settings", exact: true }).click();
      await panel.getByRole("button", { name: /^Permissions/ }).click();
      await expect(panel.getByRole("radio", { name: /^Auto / })).toHaveAttribute("aria-checked", "true");
      await page.screenshot({ path: testInfo.outputPath("permissions.png"), animations: "disabled" });
      await panel.getByRole("radio", { name: /^Full access / }).click();
      await panel.getByRole("button", { name: /^Specialist access requests/ }).click();
      await expect(panel).toHaveAccessibleName("Specialist access requests");
      await panel.getByRole("button", { name: "Permissions", exact: true }).click();
      await expect(panel.getByRole("radio", { name: /^Full access / })).toHaveAttribute("aria-checked", "true");
      await panel.getByRole("button", { name: "Discard", exact: true }).click();
      await panel.getByRole("button", { name: "NyxBot settings", exact: true }).click();
      await page.goBack();
      await expect(panel).not.toBeVisible();
      await expect.poll(() => page.evaluate(() => {
        const { router } = (window as unknown as { __nyxApp: { router: { state: { status: string; resolvedLocation?: { href: string }; location: { href: string } } } } }).__nyxApp;
        return router.state.status === "idle" && router.state.resolvedLocation?.href === router.state.location.href;
      })).toBe(true);
      await page.getByRole("button", { name: "Search", exact: true }).first().click();
      await page.getByRole("combobox").fill("Runs per day");
      await page.getByRole("option", { name: "Timezone and budgets NyxBot settings", exact: true }).click();
      await expect(panel).toHaveAccessibleName("Timezone and budgets");
      await expect(panel.getByRole("spinbutton", { name: "Runs per day", exact: true })).toBeVisible();
      await expect(page).toHaveURL(/panel=nyxbot.*panelTab=automations(?:%2F|%252F|\/)budgets/);
      // Reload the nested address: the parent stays beneath it and Back walks up.
      await page.reload();
      await expect(panel).toHaveAccessibleName("Timezone and budgets");
      await panel.getByRole("button", { name: "Automations", exact: true }).click();
      await expect(panel).toHaveAccessibleName("Automations");
      await panel.getByRole("button", { name: "NyxBot settings", exact: true }).click();
      await expect(panel).toHaveAccessibleName("NyxBot settings");
    });
  }
}
