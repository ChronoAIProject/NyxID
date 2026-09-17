import { expect, test, type Page } from "@playwright/test";
import { gunzipSync } from "node:zlib";
import {
  managedBot,
  managedBootstrap,
  mockDashboard,
} from "./managed-onboarding-fixtures";
import type { TelegramNewRequest } from "../src/schemas/telegram-new";

const requestId = "3c638c7f-210a-44fc-9b67-f6c878d67c54";
const orgId = "cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65";

const pendingRequest: TelegramNewRequest = {
  id: requestId,
  status: "waiting_telegram",
  revision: 1,
  owner_user_id: orgId,
  label: "Saved support",
  auto_connect: true,
  expires_at: "2099-09-16T10:00:00Z",
  telegram_bot_id: null,
  bot_username: null,
  channel_bot_id: null,
};

async function reviewFixture(
  page: Page,
  request: TelegramNewRequest | null = pendingRequest,
) {
  const state = {
    request,
    available: true,
    fail: false,
    reads: [] as string[],
    writes: [] as string[],
    managed: [] as string[],
  };
  await page.route("https://connect.facebook.net/**", (route) =>
    route.fulfill({
      contentType: "application/javascript",
      body: "window.FB = { init() {}, login() {} }; window.fbAsyncInit?.();",
    }),
  );
  await mockDashboard(page);
  await page.route("**/api/v1/orgs", (route) =>
    route.fulfill({
      json: {
        orgs: [{ id: orgId, display_name: "Support team", your_role: "admin" }],
      },
    }),
  );
  await page.route("**/api/v1/channel-bots/managed-onboarding/*", (route) => {
    state.managed.push(route.request().url());
    return route.fulfill({
      json: route.request().url().endsWith("/x")
        ? { available: true, flow: "oauth_connection", provider_slug: "x" }
        : managedBootstrap,
    });
  });
  await page.route("**/api/v1/channel-bots/telegram-new**", (route) => {
    const url = new URL(route.request().url());
    if (route.request().method() === "GET") {
      state.reads.push(url.search);
      return route.fulfill(
        state.fail
          ? {
              status: 503,
              json: {
                error: "unavailable",
                message: "Configuration temporarily unavailable",
                error_code: 5000,
              },
            }
          : {
              json: {
                available: state.available,
                manager_username: "NyxSetupBot",
                request: state.request,
              },
            },
      );
    }
    state.writes.push(`${route.request().method()} ${url.pathname}`);
    if (route.request().method() === "DELETE") {
      state.request = null;
      return route.fulfill({ json: {} });
    }
    return route.fulfill({
      json: {
        request: state.request,
        launch_url: "https://t.me/NyxSetupBot?start=review-challenge",
      },
    });
  });
  await page.route("**/api/v1/channel-bots", (route) => {
    if (route.request().method() !== "GET")
      state.writes.push("unexpected create-bot submission");
    return route.fulfill({ json: { bots: [], total: 0 } });
  });
  return state;
}

async function selectPlatform(page: Page, current: string, next: string) {
  await page
    .getByRole("dialog")
    .getByRole("combobox", { name: "Platform" })
    .filter({ hasText: new RegExp(`^${current}$`) })
    .click();
  await page.getByRole("option", { name: next, exact: true }).click();
}

for (const width of [1440, 390]) {
  test(`Telegram setup survives reload and a closed browser page at ${width}px`, async ({
    page,
    context,
  }) => {
    const height = width === 390 ? 844 : 900;
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ colorScheme: "dark" });
    let request: TelegramNewRequest | null = null;
    const starts: unknown[] = [];
    const launches: string[] = [];
    const connects: unknown[] = [];
    const cancellations: string[] = [];
    const managedRequests: string[] = [];
    page.on("request", (request) => {
      if (request.url().includes("managed-onboarding/telegram-new"))
        managedRequests.push(request.url());
    });
    // Keep the external handoff inside the browser harness.
    await context.route("https://t.me/**", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: "Telegram creation form",
      }),
    );
    async function mockSetup(target: Page) {
      await mockDashboard(target);
      await target.route(`**/api/v1/channel-bots/${requestId}`, (route) =>
        route.fulfill({
          json: {
            ...managedBot,
            id: requestId,
            platform: "telegram-new",
            label: "Mobile support",
            platform_bot_username: "MobileSupportBot",
            status: "active",
            webhook_registered: true,
            managed_setup: null,
          },
        }),
      );
      await target.route("**/api/v1/orgs", (route) =>
        route.fulfill({
          json: {
            orgs: [
              { id: orgId, display_name: "Support team", your_role: "admin" },
            ],
          },
        }),
      );
      await target.route(
        "**/api/v1/channel-bots/telegram-new**",
        async (route) => {
          const url = new URL(route.request().url());
          const path = url.pathname;
          const method = route.request().method();
          if (method === "GET") {
            await route.fulfill({
              json: {
                available: true,
                manager_username: "NyxSetupBot",
                request:
                  request?.status === "connected" &&
                  url.searchParams.get("request_id") !== request.id
                    ? null
                    : request,
              },
            });
          } else if (method === "DELETE") {
            cancellations.push(path);
            request = null;
            await route.fulfill({ json: {} });
          } else if (path.endsWith("/connect")) {
            connects.push(route.request().postDataJSON());
            await route.fulfill({
              status: 500,
              json: { message: "Automatic setup must not call connect" },
            });
          } else if (path.endsWith("/launch")) {
            launches.push(path);
            await route.fulfill({
              json: {
                request,
                launch_url: "https://t.me/NyxSetupBot?start=renewed-challenge",
              },
            });
          } else {
            const body = route.request().postDataJSON() as {
              label: string;
              target_org_id?: string;
              auto_connect: boolean;
            };
            starts.push(body);
            request = {
              id: requestId,
              status: "waiting_telegram",
              revision: 1,
              owner_user_id: body.target_org_id ?? "test-user",
              label: body.label,
              expires_at: new Date(Date.now() + 15 * 60_000).toISOString(),
              telegram_bot_id: null,
              bot_username: null,
              channel_bot_id: null,
              auto_connect: body.auto_connect,
              connection_error: null,
            };
            await route.fulfill({
              json: {
                request,
                launch_url: "https://t.me/NyxSetupBot?start=initial-challenge",
              },
            });
          }
        },
      );
    }
    await mockSetup(page);
    await page.goto("/channel-bots");
    await page.getByRole("button", { name: "Add Bot", exact: true }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByLabel("Label", { exact: true }).fill("Support");
    await dialog
      .getByRole("combobox")
      .filter({ hasText: "Telegram bot token" })
      .click();
    await page.getByRole("option", { name: "Telegram", exact: true }).click();
    await expect(page).toHaveURL(/connect=telegram-new/);
    await expect(dialog).toBeVisible();
    await expect(
      dialog.getByRole("heading", { name: "Add Channel Bot" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", {
        name: "Channel Bots",
        exact: true,
        includeHidden: true,
      }),
    ).toBeAttached();
    const historyLength = await page.evaluate(() => history.length);
    await dialog.getByLabel("Label", { exact: true }).fill("");
    await dialog
      .getByLabel("Label", { exact: true })
      .pressSequentially("Mobile support");
    await dialog.getByRole("combobox", { name: "Scope" }).click();
    await page.getByRole("option", { name: "Support team" }).click();
    await expect(page).toHaveURL(new RegExp(orgId));
    expect(await page.evaluate(() => history.length)).toBe(historyLength);
    await page.reload();
    await expect(dialog.getByLabel("Label", { exact: true })).toHaveValue(
      "Mobile support",
    );
    await expect(dialog.getByRole("combobox", { name: "Scope" })).toContainText(
      "Support team",
    );
    const popupOpened = page.waitForEvent("popup");
    await page
      .getByRole("button", { name: "Continue in Telegram", exact: true })
      .click();
    const popup = await popupOpened;
    await expect(popup).toHaveURL(
      "https://t.me/NyxSetupBot?start=initial-challenge",
    );
    await popup.close();
    await expect(
      page.getByRole("link", { name: "Open Telegram" }),
    ).toHaveAttribute(
      "href",
      "https://t.me/NyxSetupBot?start=initial-challenge",
    );
    expect(starts).toEqual([
      { label: "Mobile support", target_org_id: orgId, auto_connect: true },
    ]);
    await expect(page).toHaveURL(new RegExp(`request_id=${requestId}`));
    await expect(
      page
        .getByRole("list", { name: "Telegram setup steps" })
        .getByRole("listitem"),
    ).toHaveCount(2);
    await expect(dialog.getByLabel("Label", { exact: true })).toBeDisabled();
    await expect(
      dialog.getByRole("combobox", { name: "Scope" }),
    ).toBeDisabled();
    await page.reload();
    const reopened = page.waitForEvent("popup");
    await page.getByRole("button", { name: "Reopen Telegram" }).click();
    const resumedPopup = await reopened;
    await expect(resumedPopup).toHaveURL(
      "https://t.me/NyxSetupBot?start=renewed-challenge",
    );
    await resumedPopup.close();
    await expect(
      page.getByRole("link", { name: "Open Telegram" }),
    ).toHaveAttribute(
      "href",
      "https://t.me/NyxSetupBot?start=renewed-challenge",
    );
    expect(launches).toHaveLength(1);
    expect(starts).toHaveLength(1);
    expect(
      await page.evaluate(() =>
        JSON.stringify({ localStorage, sessionStorage }),
      ),
    ).not.toContain("challenge");
    expect(page.url()).not.toContain("challenge");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    expect(await page.locator("html").getAttribute("class")).toContain(
      "theme-dark",
    );
    const scrollOwners = await dialog.evaluate(
      (element) =>
        [element, ...element.querySelectorAll("*")].filter(
          (node) =>
            node.scrollHeight > node.clientHeight &&
            /auto|scroll/.test(getComputedStyle(node).overflowY),
        ).length,
    );
    expect(scrollOwners).toBeLessThanOrEqual(1);
    const backgroundScroll = await page.evaluate(() => window.scrollY);
    await dialog
      .getByRole("combobox")
      .filter({ hasText: /^Telegram$/ })
      .focus();
    const primary = dialog.getByRole("button", { name: "Reopen Telegram" });
    for (
      let tab = 0;
      tab < 12 &&
      !(await primary.evaluate((node) => node === document.activeElement));
      tab++
    ) {
      await page.keyboard.press("Tab");
    }
    await expect(primary).toBeFocused();
    await expect(primary).toBeInViewport({ ratio: 1 });
    const cancel = dialog.getByRole("button", {
      name: "Cancel setup",
      exact: true,
    });
    for (
      let tab = 0;
      tab < 12 &&
      !(await cancel.evaluate((node) => node === document.activeElement));
      tab++
    ) {
      await page.keyboard.press("Tab");
    }
    await expect(cancel).toBeFocused();
    await expect(cancel).toBeInViewport({ ratio: 1 });
    expect(await page.evaluate(() => window.scrollY)).toBe(backgroundScroll);
    await page.screenshot({
      path: test.info().outputPath(`telegram-review-${width}x${height}-bottom.png`),
    });
    await dialog.evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.screenshot({
      path: test.info().outputPath(`telegram-review-${width}x${height}-top.png`),
    });
    await page.setViewportSize({ width, height: 700 });
    await expect
      .poll(() =>
        dialog.evaluate((element) => element.getBoundingClientRect().height),
      )
      .toBeLessThanOrEqual(700 * 0.9 + 1);
    const dimensions = await dialog.evaluate((element) => ({
      height: element.getBoundingClientRect().height,
      width: element.clientWidth,
      scrollWidth: element.scrollWidth,
      scrollHeight: element.scrollHeight,
      clientHeight: element.clientHeight,
      overflowY: getComputedStyle(element).overflowY,
    }));
    expect(dimensions.height).toBeLessThanOrEqual(700 * 0.9 + 1);
    expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.width);
    expect(dimensions.overflowY).toBe("auto");
    await dialog.hover();
    await page.mouse.wheel(0, 2000);
    await dialog
      .getByRole("button", { name: "Cancel setup", exact: true })
      .scrollIntoViewIfNeeded();
    await expect(
      dialog.getByRole("button", { name: "Cancel setup", exact: true }),
    ).toBeInViewport({ ratio: 1 });
    if (dimensions.scrollHeight > dimensions.clientHeight) {
      await expect
        .poll(() => dialog.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(0);
    }
    await page.screenshot({
      path: test.info().outputPath(`nyxbot-creation-setup-bottom-${width}.png`),
    });
    await dialog.evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.screenshot({ path: test.info().outputPath(`nyxbot-creation-setup-${width}.png`) });

    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page).not.toHaveURL(/connect=telegram-new/);
    await page.reload();
    await expect(dialog).toHaveCount(0);
    await page.getByRole("button", { name: "Resume Telegram setup" }).click();
    await expect(dialog).toBeVisible();
    await expect(dialog.getByLabel("Label", { exact: true })).toBeDisabled();

    const pendingUrl = page.url();
    await page.close();
    const returned = await context.newPage();
    await returned.setViewportSize({ width, height });
    await returned.emulateMedia({ colorScheme: "dark" });
    await mockSetup(returned);
    await returned.goto(pendingUrl);
    await expect(
      returned.getByRole("dialog").getByLabel("Label", { exact: true }),
    ).toHaveValue("Mobile support");
    await expect(
      returned.getByRole("dialog").getByRole("combobox", { name: "Scope" }),
    ).toContainText("Support team");
    request = {
      ...request!,
      status: "ready",
      revision: 3,
      telegram_bot_id: "900",
      bot_username: "MobileSupportBot",
    };
    await returned
      .getByRole("button", { name: "Check progress", exact: true })
      .click();
    await expect(
      returned.getByText("Connecting @MobileSupportBot…"),
    ).toBeVisible();
    await expect(
      returned.getByRole("button", { name: "Connect bot", exact: true }),
    ).toHaveCount(0);
    expect(connects).toEqual([]);
    const savedUrl = returned.url();
    await returned.close();
    // The server completes the request with no browser open.
    request = {
      ...request,
      status: "connected",
      revision: 6,
      channel_bot_id: requestId,
    };
    const completed = await context.newPage();
    await mockSetup(completed);
    await completed.goto(savedUrl);
    await expect(completed).toHaveURL(
      new RegExp(`/channel-bots/${requestId}$`),
    );
    expect(connects).toEqual([]);
    expect(starts).toHaveLength(1);
    expect(cancellations).toEqual([]);
    expect(managedRequests).toEqual([]);
  });
}

test("pending Telegram bot detail resumes in the modal and platform changes keep it open", async ({
  page,
}) => {
  const writes: string[] = [];
  await mockDashboard(page);
  await page.route(`**/api/v1/channel-bots/${requestId}`, (route) =>
    route.fulfill({
      json: {
        ...managedBot,
        id: requestId,
        platform: "telegram-new",
        label: "Saved support",
        status: "pending",
        managed_setup: null,
      },
    }),
  );
  await page.route("**/api/v1/channel-bots/telegram-new**", async (route) => {
    if (route.request().method() !== "GET")
      writes.push(route.request().method());
    await route.fulfill({
      json: {
        available: true,
        manager_username: "NyxSetupBot",
        request: {
          id: requestId,
          status: "waiting_telegram",
          revision: 1,
          owner_user_id: "test-user",
          label: "Saved support",
          auto_connect: true,
          expires_at: new Date(Date.now() + 15 * 60_000).toISOString(),
          telegram_bot_id: null,
          bot_username: null,
          channel_bot_id: null,
        },
      },
    });
  });
  await page.route("**/api/v1/channel-bots", async (route) => {
    if (route.request().method() !== "GET")
      writes.push(route.request().method());
    await route.fulfill({ json: { bots: [], total: 0 } });
  });
  await page.goto(`/channel-bots/${requestId}`);
  await page.getByRole("link", { name: "Continue Telegram setup" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel("Label", { exact: true })).toHaveValue(
    "Saved support",
  );
  await expect(dialog.getByLabel("Label", { exact: true })).toBeDisabled();
  await dialog
    .getByRole("combobox")
    .filter({ hasText: /^Telegram$/ })
    .click();
  await page
    .getByRole("option", { name: "Telegram bot token", exact: true })
    .click();
  await expect(dialog).toBeVisible();
  await expect(page).not.toHaveURL(/connect=telegram-new/);
  await expect(dialog.getByLabel("Label", { exact: true })).toBeEnabled();
  await dialog.getByLabel("Bot token", { exact: true }).fill("existing-secret");
  await dialog
    .getByRole("combobox")
    .filter({ hasText: "Telegram bot token" })
    .click();
  await page.getByRole("option", { name: "Telegram", exact: true }).click();
  await expect(dialog.getByLabel("Label", { exact: true })).toBeDisabled();
  await expect(page).toHaveURL(new RegExp(`request_id=${requestId}`));
  await dialog.locator("form").dispatchEvent("submit");
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(writes).toEqual([]);
  expect(page.url()).not.toContain("existing-secret");
});

for (const inFlight of [false, true]) {
  test(`platform switching clears credentials with in-flight setup ${inFlight}`, async ({
    page,
  }) => {
    const state = await reviewFixture(page, inFlight ? pendingRequest : null);
    await page.goto("/channel-bots?connect=telegram-new&label=Draft");
    const dialog = page.getByRole("dialog");
    await expect(
      dialog.getByRole("list", { name: "Telegram setup steps" }),
    ).toBeVisible();
    await selectPlatform(page, "Telegram", "Slack");
    await dialog
      .getByLabel("Bot token", { exact: true })
      .fill("slack-private-token");
    await dialog.getByLabel("Signing Secret").fill("slack-signing-secret");
    await selectPlatform(page, "Slack", "Telegram");
    if (inFlight)
      await expect(dialog.getByLabel("Label", { exact: true })).toBeDisabled();
    else
      await expect(dialog.getByLabel("Label", { exact: true })).toBeEnabled();
    await selectPlatform(page, "Telegram", "WhatsApp");
    await expect(
      dialog.getByRole("button", { name: "Connect with Meta" }),
    ).toBeVisible();
    await dialog.getByText("Advanced: use your own credentials").click();
    await expect(
      dialog.getByLabel("Access token", { exact: true }),
    ).toHaveValue("");
    await expect(
      dialog.getByLabel("Meta App Secret", { exact: true }),
    ).toHaveValue("");
    await selectPlatform(page, "WhatsApp", "X (Twitter)");
    await expect(
      dialog.getByRole("button", { name: "Connect X (Twitter) account" }),
    ).toBeVisible();
    await selectPlatform(page, "X \\(Twitter\\)", "Slack");
    await expect(dialog.getByLabel("Bot token", { exact: true })).toHaveValue(
      "",
    );
    await expect(dialog.getByLabel("Signing Secret")).toHaveValue("");
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    expect(state.writes).toEqual([]);
    expect(state.managed.some((url) => url.includes("telegram-new"))).toBe(
      false,
    );
    expect(page.url()).not.toContain("secret");
    expect(
      await page.evaluate(() =>
        JSON.stringify({ localStorage, sessionStorage }),
      ),
    ).not.toContain("slack-");
  });
}

test("switching away stops request polling without cancelling or auto-navigating", async ({
  page,
}) => {
  const state = await reviewFixture(page);
  await page.goto(`/channel-bots?connect=telegram-new&request_id=${requestId}`);
  await page.bringToFront();
  await expect(
    page.getByRole("button", { name: "Reopen Telegram" }),
  ).toBeVisible();
  // The page's separate summary query may refresh on window focus. Only the
  // selected request's query polls, and switching platforms must stop that query.
  const selectedReads = () => state.reads.filter((query) =>
    new URLSearchParams(query).get("request_id") === requestId,
  ).length;
  const initial = selectedReads();
  await expect
    .poll(selectedReads, { timeout: 5000 })
    .toBeGreaterThan(initial);
  await selectPlatform(page, "Telegram", "Slack");
  await page.waitForTimeout(500);
  const stopped = selectedReads();
  // Observe more than two real polling intervals while the request remains active.
  await page.waitForTimeout(4500);
  expect(selectedReads()).toBe(stopped);
  await expect(
    page.getByRole("dialog").getByLabel("Signing Secret"),
  ).toBeVisible();
  expect(state.writes).toEqual([]);
});

for (const dismissal of ["Escape", "backdrop", "Close"]) {
  test(`${dismissal} dismisses without cancelling, then the banner resumes`, async ({
    page,
  }) => {
    const state = await reviewFixture(page);
    await page.goto(
      `/channel-bots?connect=telegram-new&request_id=${requestId}`,
    );
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByLabel("Label", { exact: true })).toHaveValue(
      pendingRequest.label,
    );
    if (dismissal === "Escape") await page.keyboard.press("Escape");
    else if (dismissal === "backdrop") await page.mouse.click(5, 5);
    else
      await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await page.clock.install();
    await page.clock.runFor(6500);
    await expect(dialog).toHaveCount(0);
    await expect(page).not.toHaveURL(/connect=telegram-new/);
    await page.getByRole("button", { name: "Resume Telegram setup" }).click();
    await expect(dialog.getByLabel("Label", { exact: true })).toHaveValue(
      pendingRequest.label,
    );
    await expect(dialog.getByLabel("Label", { exact: true })).toBeDisabled();
    expect(state.writes).toEqual([]);
  });
}

for (const scenario of ["unavailable", "error"]) {
  test(`${scenario} stays inline and its retry does not submit the containing form`, async ({
    page,
  }) => {
    const state = await reviewFixture(page, null);
    state.available = scenario !== "unavailable";
    state.fail = scenario === "error";
    await page.goto("/channel-bots?connect=telegram-new&label=Draft");
    const dialog = page.getByRole("dialog");
    await expect(
      dialog.getByText(
        scenario === "error"
          ? "Configuration temporarily unavailable"
          : /An administrator needs to configure a manager bot/,
      ),
    ).toBeVisible();
    let submits = 0;
    await page.exposeFunction("recordSubmit", () => submits++);
    await dialog.locator("form").evaluate((form) =>
      form.addEventListener("submit", () => {
        void (
          window as unknown as { recordSubmit: () => Promise<void> }
        ).recordSubmit();
      }),
    );
    state.available = true;
    state.fail = false;
    await dialog
      .getByRole("button", {
        name: scenario === "error" ? "Check progress" : "Check configuration",
      })
      .click();
    await expect(
      dialog.getByRole("button", { name: "Continue in Telegram" }),
    ).toBeVisible();
    expect(submits).toBe(0);
    expect(state.writes).toEqual([]);
  });
}

test("Cancel setup unlocks fields and the next start uses the edited destination", async ({
  page,
  context,
}) => {
  const state = await reviewFixture(page);
  const starts: unknown[] = [];
  await context.route("https://t.me/**", (route) =>
    route.fulfill({ body: "Telegram" }),
  );
  await page.route("**/api/v1/channel-bots/telegram-new", (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    const input = route.request().postDataJSON();
    starts.push(input);
    state.request = {
      ...pendingRequest,
      label: input.label,
      owner_user_id: input.target_org_id ?? "test-user",
    };
    return route.fulfill({
      json: {
        request: state.request,
        launch_url: "https://t.me/NyxSetupBot?start=review-challenge",
      },
    });
  });
  await page.goto(`/channel-bots?connect=telegram-new&request_id=${requestId}`);
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByLabel("Label", { exact: true })).toHaveValue(
    pendingRequest.label,
  );
  let submits = 0;
  await page.exposeFunction("recordSubmit", () => submits++);
  await dialog.locator("form").evaluate((form) =>
    form.addEventListener("submit", () => {
      void (
        window as unknown as { recordSubmit: () => Promise<void> }
      ).recordSubmit();
    }),
  );
  await dialog
    .getByRole("button", { name: "Check progress", exact: true })
    .click();
  const reopen = page.waitForEvent("popup");
  await dialog.getByRole("button", { name: "Reopen Telegram" }).click();
  await (await reopen).close();
  await dialog
    .getByRole("button", { name: "Cancel setup", exact: true })
    .click();
  await expect(dialog.getByLabel("Label", { exact: true })).toBeEnabled();
  await expect(page).not.toHaveURL(/request_id=/);
  await dialog
    .getByLabel("Label", { exact: true })
    .fill("Personal replacement");
  await dialog.getByRole("combobox", { name: "Scope" }).click();
  await page.getByRole("option", { name: "Personal", exact: true }).click();
  const popup = page.waitForEvent("popup");
  await dialog.getByRole("button", { name: "Continue in Telegram" }).click();
  await (await popup).close();
  await expect(dialog.getByLabel("Label", { exact: true })).toBeDisabled();
  expect(starts).toEqual([
    { label: "Personal replacement", auto_connect: true },
  ]);
  expect(state.writes.filter((write) => write.startsWith("DELETE"))).toEqual([
    `DELETE /api/v1/channel-bots/telegram-new/requests/${requestId}`,
  ]);
  expect(submits).toBe(0);
});

test("Telegram secrets stay out of live telemetry and storage", async ({
  page,
  context,
}) => {
  await reviewFixture(page);
  const telemetry: string[] = [];
  await page.addInitScript(() => {
    // PostHog otherwise drops every event from WebDriver, making this check vacuous.
    Object.defineProperty(navigator, "webdriver", { get: () => false });
    const userAgent = navigator.userAgent.replace("HeadlessChrome", "Chrome");
    Object.defineProperty(navigator, "userAgent", { get: () => userAgent });
    Object.defineProperty(navigator, "userAgentData", { get: () => undefined });
    localStorage.setItem(
      "nyxid.telemetry_consent",
      JSON.stringify({ state: { enabled: true, asked: true }, version: 1 }),
    );
  });
  await page.route("**/api/v1/public/config", (route) =>
    route.fulfill({
      json: {
        telemetry_dsn: "phc_review_fixture",
        telemetry_host: "http://telemetry.test",
      },
    }),
  );
  await page.route("http://telemetry.test/**", (route) => {
    const body = route.request().postDataBuffer();
    if (body) {
      const decoded = (
        body[0] === 0x1f && body[1] === 0x8b ? gunzipSync(body) : body
      ).toString();
      telemetry.push(
        decoded.startsWith("data=")
          ? Buffer.from(
              new URLSearchParams(decoded).get("data")!,
              "base64",
            ).toString()
          : decoded,
      );
    }
    if (new URL(route.request().url()).pathname.endsWith(".js"))
      return route.fulfill({ contentType: "application/javascript", body: "" });
    return route.fulfill({
      json: { status: 1, featureFlags: {} },
      headers: { "Access-Control-Allow-Origin": "*" },
    });
  });
  await context.route("https://t.me/**", (route) =>
    route.fulfill({ body: "Telegram" }),
  );
  await page.goto(`/channel-bots?connect=telegram-new&request_id=${requestId}`);
  const dialog = page.getByRole("dialog");
  const popup = page.waitForEvent("popup");
  await dialog.getByRole("button", { name: "Reopen Telegram" }).click();
  await (await popup).close();
  const fallback = page.waitForEvent("popup");
  await dialog.getByRole("link", { name: "Open Telegram" }).click();
  await (await fallback).close();
  await selectPlatform(page, "Telegram", "Telegram bot token");
  await dialog
    .getByLabel(/^Bot token$/i)
    .fill("private-bot-token-fixture");
  await dialog.getByLabel("Label", { exact: true }).click();
  await expect
    .poll(() => telemetry.join("\n"), { timeout: 12000 })
    .toContain("$autocapture");
  expect(telemetry.join("\n")).toContain("$pageview");
  expect(telemetry.join("\n")).not.toContain("review-challenge");
  expect(telemetry.join("\n")).not.toContain("private-bot-token-fixture");
  expect(page.url()).not.toMatch(/review-challenge|private-bot-token-fixture/);
  expect(
    await page.evaluate(() =>
      JSON.stringify({ localStorage, sessionStorage, state: history.state }),
    ),
  ).not.toMatch(/review-challenge|private-bot-token-fixture/);

  const code = "ABCDE-FGHJK-LMNPQ-RSTUV";
  await page.route(
    "**/api/v1/channel-bots/telegram-new/claims/preview",
    (route) =>
      route.fulfill({
        json: {
          bot_username: "ClaimedBot",
          expires_at: pendingRequest.expires_at,
        },
      }),
  );
  telemetry.length = 0;
  await page.goto(`/channel-bots?connect=telegram-new&claim=${code}`);
  await expect(
    page.getByRole("heading", { name: "Connect @ClaimedBot" }),
  ).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect
    .poll(() => telemetry.join("\n"), { timeout: 12000 })
    .toContain("$pageview");
  expect(telemetry.join("\n")).not.toContain(code);
  expect(page.url()).not.toContain(code);
  expect(
    await page.evaluate(() =>
      JSON.stringify({ localStorage, sessionStorage, state: history.state }),
    ),
  ).not.toContain(code);
});

for (const width of [390, 1440]) {
  test(`a maximum-length unbroken Telegram label fits the modal at ${width}px`, async ({
    page,
  }) => {
    await reviewFixture(page, { ...pendingRequest, label: "A".repeat(128) });
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.emulateMedia({ colorScheme: "dark" });
    await page.goto(
      `/channel-bots?connect=telegram-new&request_id=${requestId}`,
    );
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByLabel("Label", { exact: true })).toHaveValue(
      "A".repeat(128),
    );
    expect(
      await dialog.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
  });
}

test("legacy Finish saved connection does not submit the create-bot form", async ({
  page,
}) => {
  const state = await reviewFixture(page, {
    ...pendingRequest,
    status: "ready",
    auto_connect: false,
    telegram_bot_id: "900",
    bot_username: "LegacyBot",
  });
  const connections: unknown[] = [];
  await page.route(
    `**/api/v1/channel-bots/telegram-new/requests/${requestId}/connect`,
    (route) => {
      connections.push(route.request().postDataJSON());
      state.request = {
        ...state.request!,
        status: "connected",
        channel_bot_id: requestId,
      };
      return route.fulfill({ json: state.request });
    },
  );
  await page.route(`**/api/v1/channel-bots/${requestId}`, (route) =>
    route.fulfill({
      json: { ...managedBot, id: requestId, platform: "telegram-new" },
    }),
  );
  await page.goto(`/channel-bots?connect=telegram-new&request_id=${requestId}`);
  const dialog = page.getByRole("dialog");
  let submits = 0;
  await page.exposeFunction("recordSubmit", () => submits++);
  await dialog.locator("form").evaluate((form) =>
    form.addEventListener("submit", () => {
      void (
        window as unknown as { recordSubmit: () => Promise<void> }
      ).recordSubmit();
    }),
  );
  await dialog.getByRole("button", { name: "Finish saved connection" }).click();
  await expect(page).toHaveURL(new RegExp(`/channel-bots/${requestId}$`));
  expect(connections).toEqual([
    { telegram_bot_id: "900", revision: pendingRequest.revision },
  ]);
  expect(state.writes).toEqual([]);
  expect(submits).toBe(0);
});
