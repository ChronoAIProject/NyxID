import { expect, test } from "@playwright/test";
import { openAccountMenu, openAssistant } from "./helpers";
import { sectionAddress } from "./settings-clarity-fixtures";
import { NYXBOT_SETTINGS_SECTIONS, NYXBOT_SETTINGS_TITLES } from "../src/lib/assistant/nyxbot-settings-sections";

test.setTimeout(90_000);
test.use({ locale: "en-US", timezoneId: "UTC" });

for (const viewport of [{ width: 1280, height: 900 }, { width: 390, height: 844 }]) {
  for (const mode of ["light", "dark"] as const) {
    test(`NyxBot hierarchy at ${viewport.width}px in ${mode}`, async ({ page }, testInfo) => {
      await page.setViewportSize(viewport);
      await page.addInitScript((theme) => localStorage.setItem("nyxid.theme", JSON.stringify({ state: { mode: theme }, version: 0 })), mode);
      await page.clock.setFixedTime(new Date("2026-10-15T12:00:00Z"));
      const triggers = ["Daily review", "Webhook alert", "Weekly summary", "Extra review"].map((label, index) => ({
        id: `11111111-1111-4111-8111-11111111111${index}`,
        user_id: "owner", label, user_service_id: null,
        status: index === 2 ? "disabled" : "active",
        source: index === 1 ? "webhook" : "schedule",
        verification: { mode: "schedule" },
        delivery: { type: "assistant", agent_id: "nyxbot", instruction: "Review updates", deliver_to: { type: "thread" } },
        inbound_url: null, next_run_at: index === 0 ? "2026-10-16T12:00:00Z" : null,
        created_at: "2026-10-01T00:00:00Z", updated_at: "2026-10-01T00:00:00Z",
      }));
      await page.route("**/api/v1/triggers", route => route.fulfill({ json: { triggers } }));
      await page.route("**/api/v1/assistant/nyxagent/notifications/summary", route => route.fulfill({ json: { unread_count: 0, needs_decision_count: 0, by_source: [] } }));
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await page.evaluate(() => {
        const { authStore } = (window as unknown as { __nyxApp: { authStore: typeof import("../src/stores/auth-store").useAuthStore } }).__nyxApp;
        const user = authStore.getState().user!;
        authStore.setState({ user: { ...user, capabilities: { ...user.capabilities, enabled_features: [...(user.capabilities?.enabled_features ?? []), "assistant:notifications", "nyxbot:service-adapters"] } } });
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
      await expect(panel).toHaveAccessibleName("Tasks & specialists");
      await expect(panel.getByRole("heading", { level: 2 })).toHaveCount(1);
      await expect(panel.getByRole("heading", { name: "Tasks & specialists", exact: true })).toBeFocused();
      const field = panel.getByRole("spinbutton", { name: "Live specialists" });
      await field.fill("10");
      await expect(panel.getByRole("button", { name: "Save changes", exact: true })).toBeVisible();
      await page.screenshot({ path: testInfo.outputPath("specialists-draft.png"), animations: "disabled" });
      await panel.getByRole("button", { name: "NyxBot settings", exact: true }).click();
      const guard = page.getByRole("dialog", { name: "Discard unsaved changes?" });
      await expect(guard).toBeVisible();
      await guard.getByRole("button", { name: "Keep editing" }).click();
      await expect(field).toHaveValue("10");
      await panel.getByRole("button", { name: "Discard", exact: true }).click();
      await panel.getByRole("button", { name: "NyxBot settings", exact: true }).click();
      await expect(panel.getByRole("button", { name: "Tasks & specialists", exact: true })).toBeFocused();
      await panel.getByRole("button", { name: /^Permissions/ }).click();
      await expect(panel.getByRole("radio", { name: /^Auto / })).toHaveAttribute("aria-checked", "true");
      await page.screenshot({ path: testInfo.outputPath("permissions.png"), animations: "disabled" });
      await panel.getByRole("radio", { name: /^Full access / }).click();
      await expect(panel.getByRole("radiogroup", { name: "Specialist access requests" })).toBeVisible();
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
      await page.getByRole("option", { name: "Automations NyxBot settings", exact: true }).click();
      await expect(panel).toHaveAccessibleName("Automations");
      await expect(panel.getByRole("spinbutton", { name: "Runs per day", exact: true })).toBeVisible();
      await expect(page).toHaveURL(/panel=nyxbot.*panelTab=automations(?:&|$)/);
      // Reload the consolidated page; Back returns directly to the root.
      await page.reload();
      await expect(panel).toHaveAccessibleName("Automations");
      await panel.getByRole("button", { name: "NyxBot settings", exact: true }).click();
      await expect(panel).toHaveAccessibleName("NyxBot settings");
      await page.evaluate(() => {
        const { authStore } = (window as unknown as { __nyxApp: { authStore: typeof import("../src/stores/auth-store").useAuthStore } }).__nyxApp;
        const user = authStore.getState().user!;
        authStore.setState({ user: { ...user, capabilities: { ...user.capabilities, enabled_features: [...(user.capabilities?.enabled_features ?? []), "assistant:notifications", "nyxbot:service-adapters"] } } });
      });
      for (const section of NYXBOT_SETTINGS_SECTIONS) {
        await sectionAddress(page, "/assistant", { panel: "nyxbot", panelTab: section });
        await expect(panel).toHaveAccessibleName(NYXBOT_SETTINGS_TITLES[section]);
        if (section === "automations") {
          await expect(panel.getByRole("link", { name: /^Daily review/ })).toBeVisible();
          await expect(panel.getByRole("link", { name: "Webhook alert Runs on webhook events" })).toBeVisible();
          await expect(panel.getByRole("link", { name: "Weekly summary Paused" })).toBeVisible();
          await expect(panel.getByRole("button", { name: "Show all", exact: true })).toBeVisible();
          await expect(panel.getByRole("heading", { name: "Automations", exact: true })).toHaveCount(1);
        }
        await page.screenshot({ path: testInfo.outputPath(`${section.replaceAll("/", "-")}.png`), animations: "disabled" });
      }
    });
  }
}
