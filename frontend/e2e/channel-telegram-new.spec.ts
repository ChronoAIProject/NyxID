import { expect, test, type Page } from "@playwright/test";
import { managedBot, mockDashboard } from "./managed-onboarding-fixtures";
import type { TelegramNewRequest } from "../src/schemas/telegram-new";

const requestId = "3c638c7f-210a-44fc-9b67-f6c878d67c54";
const orgId = "cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65";

for (const width of [1440, 390]) {
  test(`Telegram setup survives reload and a closed browser page at ${width}px`, async ({
    page,
    context,
  }) => {
    await page.setViewportSize({ width, height: 950 });
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
    await dialog.getByLabel("Label", { exact: true }).fill("Mobile support");
    await dialog.getByRole("combobox", { name: "Scope" }).click();
    await page.getByRole("option", { name: "Support team" }).click();
    await expect(page).toHaveURL(new RegExp(orgId));
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
    ).toBeInViewport();
    if (dimensions.scrollHeight > dimensions.clientHeight) {
      await expect
        .poll(() => dialog.evaluate((element) => element.scrollTop))
        .toBeGreaterThan(0);
    }
    await page.screenshot({
      path: `/tmp/nyxbot-creation-setup-bottom-${width}.png`,
    });
    await dialog.evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.screenshot({ path: `/tmp/nyxbot-creation-setup-${width}.png` });

    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page).not.toHaveURL(/connect=telegram-new/);
    await page.reload();
    await expect(dialog).toHaveCount(0);
    await page.getByRole("button", { name: "Resume Telegram setup" }).click();
    await expect(dialog).toBeVisible();
    await expect(dialog.getByLabel("Label", { exact: true })).toBeDisabled();

    await page.close();
    const returned = await context.newPage();
    await returned.setViewportSize({ width, height: 950 });
    await returned.emulateMedia({ colorScheme: "dark" });
    await mockSetup(returned);
    await returned.goto("/channel-bots");
    await returned
      .getByRole("button", { name: "Resume Telegram setup" })
      .click();
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
  await dialog.getByLabel("Bot Token", { exact: true }).fill("existing-secret");
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
