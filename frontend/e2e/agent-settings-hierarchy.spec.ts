import { expect, test } from "@playwright/test";
import { openAssistant } from "./helpers";
import { clarityTheme, disclosureIcons, sectionAddress } from "./settings-clarity-fixtures";
import {
  AGENT_DETAILS_SECTIONS,
  AGENT_DETAILS_TITLES,
} from "../src/lib/assistant/agent-details-sections";

// Each case captures every retained page as well as exercising navigation.
test.setTimeout(90_000);
test.use({ locale: "en-US", timezoneId: "UTC" });
test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date("2026-10-15T12:00:00Z"));
});

for (const width of [1280, 390])
  for (const mode of ["light", "dark"] as const) {
    test(`Agent hierarchy ${width}px ${mode}`, async ({ page }, info) => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await clarityTheme(page, width, mode);
      await openAssistant(page, { faults: { nyxagentEnabled: true } });
      await page.evaluate(() => {
        const original = globalThis.__nyxidAssistantHttpMock;
        globalThis.__nyxidAssistantHttpMock = request => request.endpoint.endsWith("/voice-service")
          ? Response.json({ service: null })
          : original?.(request);
      });
      const original = page.url();
      const originalLength = await page.evaluate(() => history.length);
      await page
        .getByRole("region", { name: "What NyxBot remembers" })
        .getByRole("button", { name: "Manage" })
        .click();
      const sheet = page.getByRole("dialog", { name: "Agent details", exact: true });
      expect(await page.evaluate(() => history.length)).toBe(originalLength);
      await sheet.getByRole("tab", { name: "Access", exact: true }).click();
      expect(await page.evaluate(() => history.length)).toBe(originalLength);
      await sheet.getByRole("button", { name: "Close agent details", exact: true }).click();
      await expect(page).toHaveURL(original);
      expect(await page.evaluate(() => history.length)).toBe(originalLength);
      await page.getByRole("region", { name: "What NyxBot remembers" }).getByRole("button", { name: "Manage" }).click();
      await sheet.getByRole("tab", { name: "Overview", exact: true }).click();
      const id = await page.evaluate(
        () =>
          (
            window as unknown as {
              __nyxApp: { router: { state: { location: { search: { agent: string } } } } };
            }
          ).__nyxApp.router.state.location.search.agent,
      );
      await page.evaluate(() => {
        const { authStore } = (
          window as unknown as {
            __nyxApp: { authStore: typeof import("../src/stores/auth-store").useAuthStore };
          }
        ).__nyxApp;
        const user = authStore.getState().user!;
        authStore.setState({
          user: {
            ...user,
            capabilities: {
              ...user.capabilities,
              enabled_features: [
                ...(user.capabilities?.enabled_features ?? []),
                "assistant:agent-learning",
                "assistant:voice-agent-key",
              ],
            },
          },
        });
      });
      await expect(
        sheet.locator('button[data-drawer-page-trigger^="overview/"]:visible'),
      ).toHaveCount(3);
      await expect(sheet.getByRole("textbox", { name: "Display name", exact: true })).toBeVisible();
      await disclosureIcons(sheet, true);
      await expect(sheet.getByRole("combobox", { name: "Agent access mode" })).toContainText("Workspace setting (Auto)");
      await page.screenshot({ path: info.outputPath("overview.png"), animations: "disabled" });
      for (const section of AGENT_DETAILS_SECTIONS.filter(
        (section) => section !== "grants/services",
      )) {
        await sectionAddress(page, "/assistant", { agent: id, details: section });
        const title = section === "memory" ? "Memory" : AGENT_DETAILS_TITLES[section];
        await expect(
          sheet.getByRole("heading", { name: title, exact: true }).first(),
        ).toBeVisible();
        if (section !== "memory" && section !== "channels")
          await disclosureIcons(sheet, section === "overview" || section === "grants");
        if (section === "grants") {
          const voice = sheet.getByRole("region", { name: "Device voice", exact: true });
          await expect(voice.getByRole("button", { name: "Create voice connection" })).toBeEnabled();
          await expect(voice.locator("[data-grouped-list]")).toBeVisible();
          await expect(voice.locator("[data-grouped-footer]")).toContainText("Keys set to all services include it");
          await expect(voice.getByRole("alert")).toHaveCount(0);
        }
        expect(await sheet.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
          true,
        );
        await page.screenshot({
          path: info.outputPath(`${section.replaceAll("/", "-")}.png`),
          animations: "disabled",
        });
      }
      const specialist = await page.evaluate(async () => {
        const modulePath = "/src/lib/assistant/nyxbot-api.ts";
        const { nyxBotApi } = await import(modulePath);
        return (await nyxBotApi.agents()).agents.find(
          (agent: { kind: string }) => agent.kind === "specialist",
        ).id as string;
      });
      for (const section of ["grants/services"] as const) {
        await sectionAddress(page, "/assistant", { agent: specialist, details: section });
        await expect(
          sheet.getByRole("heading", { name: AGENT_DETAILS_TITLES[section], exact: true }).first(),
        ).toBeVisible();
        await page.screenshot({
          path: info.outputPath(`${section.replaceAll("/", "-")}.png`),
          animations: "disabled",
        });
      }
      await sectionAddress(page, "/assistant", { agent: id, details: "overview" });
      await sheet.getByRole("textbox", { name: "Display name", exact: true }).fill("Draft name");
      await sheet.getByRole("tab", { name: "Access", exact: true }).click();
      await expect(sheet.getByRole("tab", { name: "Access", exact: true })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      await sheet.getByRole("tab", { name: "Overview", exact: true }).click();
      await expect(sheet.getByRole("tab", { name: "Overview", exact: true })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      await expect(sheet.getByRole("textbox", { name: "Display name", exact: true })).toHaveValue(
        "Draft name",
      );
      await sheet.getByRole("button", { name: "Discard", exact: true }).click();
      await sheet.getByRole("button", { name: /^Persona & notes/ }).click();
      await page.reload();
      await expect(
        sheet.getByRole("heading", { name: "Persona & notes", exact: true }),
      ).toBeFocused();
      await sheet.getByRole("button", { name: "Overview", exact: true }).click();
      await expect(sheet.locator('[data-drawer-page-trigger="overview/persona"]')).toBeFocused();
      await sheet.getByRole("button", { name: "Close agent details", exact: true }).click();
      await page.getByRole("button", { name: "Search", exact: true }).first().click();
      await page.getByRole("combobox").fill("Channel progress reply");
      await page.getByRole("option", { name: "Overview NyxBot", exact: true }).click();
      await expect(sheet.getByRole("switch", { name: "Channel progress reply" })).toBeVisible();
      const openedLength = await page.evaluate(() => history.length);
      await sheet.getByRole("button", { name: /^Persona & notes/ }).click();
      await sheet.getByRole("button", { name: "Overview", exact: true }).click();
      expect(await page.evaluate(() => history.length)).toBe(openedLength);
      await page.goBack();
      await expect(sheet).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }
