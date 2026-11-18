import { expect, test } from "@playwright/test";
import type { AssistantHttpMockHandler } from "../src/lib/assistant/assistant-http";
import { openAssistant } from "./helpers";
import { billingApi } from "./billing-api-fixtures";
import { clarityTheme, sectionAddress, settingsApi } from "./settings-clarity-fixtures";

test.use({ locale: "en-US", timezoneId: "UTC" });

test("settles rapid settings navigation and live refreshes under CPU and network delay", async ({ page, context }) => {
  test.setTimeout(90_000);
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.stack ?? error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const cpu = await context.newCDPSession(page);
  await cpu.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  await cpu.send("Network.enable");
  await cpu.send("Network.emulateNetworkConditions", {
    offline: false, latency: 120, downloadThroughput: 1_000_000, uploadThroughput: 500_000,
  });
  await billingApi(page);
  await clarityTheme(page, 390, "dark");
  await settingsApi(page);
  await page.addInitScript(() => {
    let handler: AssistantHttpMockHandler | undefined;
    let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
    let timezone = "UTC";
    let maximum = 8;
    const encoder = new TextEncoder();
    const probe = {
      reads: 0,
      writes: 0,
      publish() {
        timezone = "Europe/Paris";
        maximum = 12;
        if (!stream) throw new Error("The owner live stream is not connected");
        stream.enqueue(encoder.encode('data: {"type":"settings"}\n\n'));
      },
    };
    Object.assign(window, { __settingsStabilityProbe: probe });
    Object.defineProperty(window, "__nyxidAssistantHttpMock", {
      configurable: true,
      get: () => handler && (async (request: Parameters<AssistantHttpMockHandler>[0]) => {
        if (request.endpoint === "/assistant/nyxagent/live") {
          return new Response(new ReadableStream<Uint8Array>({
            start(controller) {
              stream = controller;
              controller.enqueue(encoder.encode('data: {"type":"ready"}\n\n'));
            },
            cancel() { stream = undefined; },
          }), { headers: { "Content-Type": "text/event-stream" } });
        }
        const response = await handler!(request);
        if (request.endpoint !== "/assistant/nyxagent/settings" || !response) return response;
        probe.reads += 1;
        // Delay the mock boundary too: these reads do not traverse the network.
        await new Promise(resolve => setTimeout(resolve, 250));
        return new Response(JSON.stringify({ ...await response.json(), timezone, max_live_subagents: maximum }), {
          headers: { "Content-Type": "application/json" },
        });
      }),
      set: (value: AssistantHttpMockHandler) => { handler = value; },
    });
  });
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await page.evaluate(() => {
    const { authStore, router } = (window as unknown as { __nyxApp: {
      authStore: typeof import("../src/stores/auth-store").useAuthStore;
      router: { subscribe: (event: "onBeforeNavigate", callback: () => void) => void };
    } }).__nyxApp;
    const user = authStore.getState().user!;
    authStore.setState({ user: { ...user, capabilities: { ...user.capabilities, billing_available: true } } });
    router.subscribe("onBeforeNavigate", () => {
      (window as unknown as { __settingsStabilityProbe: { writes: number } }).__settingsStabilityProbe.writes += 1;
    });
  });
  const panel = page.locator('[role="dialog"]').filter({ has: page.locator("[data-form-viewport]") });
  // Removed and gated ids must converge, rather than repeatedly replacing the URL.
  for (const [removed, canonical] of [
    ["permissions/specialists", "permissions"], ["tasks/groups", "tasks"],
    ["automations/budgets", "automations"], ["channels/adapters", "channels"],
    ["notifications", undefined],
  ] as const) {
    await sectionAddress(page, "/assistant", { panel: "nyxbot", panelTab: removed });
    await expect.poll(() => new URL(page.url()).searchParams.get("panelTab")).toBe(canonical ?? null);
    await expect(panel).toBeVisible();
  }
  await sectionAddress(page, "/assistant", { panel: "nyxbot", panelTab: "tasks" });
  await expect(page).toHaveURL(/panelTab=tasks(?:&|$)/);
  const field = panel.getByRole("spinbutton", { name: "Live specialists", exact: true });
  await field.fill("10");
  await page.evaluate(async () => {
    const { router } = (window as unknown as { __nyxApp: { router: {
      navigate: (options: unknown) => Promise<void>;
    } } }).__nyxApp;
    const pending: Promise<void>[] = [];
    for (const panelTab of ["automations", "permissions", "tasks", "channels", "tasks"]) {
      pending.push(router.navigate({ to: "/assistant", search: { mock: 1, panel: "nyxbot", panelTab }, replace: true }));
      (window as unknown as { __settingsStabilityProbe: { publish: () => void } }).__settingsStabilityProbe.publish();
      await new Promise(resolve => setTimeout(resolve, 30));
    }
    await Promise.all(pending);
  });
  await expect(field).toHaveValue("10");
  await sectionAddress(page, "/assistant", { panel: "nyxbot", panelTab: "automations" });
  await expect(panel.getByRole("combobox", { name: "Timezone", exact: true })).toContainText("Europe/Paris");
  await sectionAddress(page, "/assistant", { panel: "nyxbot", panelTab: "tasks" });
  await expect(field).toHaveValue("10");
  await panel.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(field).toHaveValue("12");
  await sectionAddress(page, "/assistant", { panel: "settings", panelTab: "display/colors/links" });
  await expect(panel).toHaveAccessibleName("Display");
  await sectionAddress(page, "/assistant", { panel: "billing", panelTab: "usage", panelSection: "usage/charts/services" });
  await expect(page.getByRole("heading", { name: "Usage charts", exact: true })).toBeVisible();
  await sectionAddress(page, "/assistant", {});
  await page.getByRole("button", { name: "Search", exact: true }).first().click();
  await page.getByRole("combobox").fill("Channel progress reply");
  await page.getByRole("option", { name: "Overview NyxBot", exact: true }).click();
  const sheet = page.getByRole("dialog", { name: "Agent details" });
  await expect(sheet.getByRole("switch", { name: "Channel progress reply" })).toBeVisible();
  for (const tab of ["Access", "Skills", "Memory", "Overview"]) {
    await sheet.getByRole("tab", { name: tab, exact: true }).click();
    await expect(sheet.getByRole("tab", { name: tab, exact: true })).toHaveAttribute("aria-selected", "true");
  }
  await sheet.getByRole("button", { name: /^Persona & notes/ }).click();
  await expect(sheet.getByRole("heading", { name: "Persona & notes", exact: true })).toBeVisible();
  await sheet.getByRole("button", { name: "Overview", exact: true }).click();
  // A loop might leave the final screen visible; assert that route writes stop too.
  await expect.poll(() => page.evaluate(() => {
    const { router } = (window as unknown as { __nyxApp: { router: { state: {
      status: string; location: { href: string }; resolvedLocation?: { href: string };
    } } } }).__nyxApp;
    return router.state.status === "idle" && router.state.location.href === router.state.resolvedLocation?.href;
  })).toBe(true);
  const address = page.url();
  const activity = () => page.evaluate(() => {
    const { reads, writes } = (window as unknown as { __settingsStabilityProbe: { reads: number; writes: number } }).__settingsStabilityProbe;
    return { reads, writes };
  });
  const settledActivity = await activity();
  await page.waitForTimeout(1000);
  expect(page.url()).toBe(address);
  expect(await activity()).toEqual(settledActivity);
  expect(errors).toEqual([]);
});
