import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
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
    async function mockSetup(target: Page) {
      await mockDashboard(target);
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
          const path = new URL(route.request().url()).pathname;
          const method = route.request().method();
          if (method === "GET") {
            await route.fulfill({
              json: {
                available: true,
                manager_username: "NyxSetupBot",
                request,
              },
            });
          } else if (path.endsWith("/connect")) {
            connects.push(route.request().postDataJSON());
            const connected = {
              ...request,
              status: "connected",
              channel_bot_id: requestId,
            };
            request = null;
            await route.fulfill({ json: connected });
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
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(
      page.getByRole("heading", { name: "Create a Telegram bot" }),
    ).toBeVisible();
    await page.getByLabel("Bot label in NyxID").fill("Mobile support");
    await page.getByRole("combobox", { name: "Connect to" }).click();
    await page.getByRole("option", { name: "Support team" }).click();
    await expect(page).toHaveURL(new RegExp(orgId));
    await page.reload();
    await expect(page.getByLabel("Bot label in NyxID")).toHaveValue(
      "Mobile support",
    );
    await expect(
      page.getByRole("combobox", { name: "Connect to" }),
    ).toContainText("Support team");
    await page.getByRole("button", { name: "Save and continue" }).click();
    await expect(
      page.getByRole("link", { name: "Open Telegram" }),
    ).toHaveAttribute(
      "href",
      "https://t.me/NyxSetupBot?start=initial-challenge",
    );
    expect(starts).toEqual([{ label: "Mobile support", target_org_id: orgId }]);
    await expect(page.getByLabel("Bot label in NyxID")).toBeDisabled();
    await expect(
      page.getByRole("combobox", { name: "Connect to" }),
    ).toBeDisabled();
    await page.reload();
    await page.getByRole("button", { name: "Get Telegram link" }).click();
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
    await page.setViewportSize({ width, height: 2100 });
    await page
      .getByRole("region", { name: "Telegram bot setup" })
      .screenshot({ path: `/tmp/nyx-telegram-setup-${width}.png` });

    await page.close();
    const returned = await context.newPage();
    await returned.setViewportSize({ width, height: 950 });
    await returned.emulateMedia({ colorScheme: "dark" });
    await mockSetup(returned);
    await returned.goto("/channel-bots");
    await returned
      .getByRole("button", { name: "Resume Telegram setup" })
      .click();
    await expect(returned.getByLabel("Bot label in NyxID")).toHaveValue(
      "Mobile support",
    );
    await expect(
      returned.getByRole("combobox", { name: "Connect to" }),
    ).toContainText("Support team");
    request = { ...request!, status: "waiting_consent", revision: 3 };
    await returned
      .getByRole("button", { name: "Check progress", exact: true })
      .click();
    await expect(
      returned.getByText(
        "Your bot has been created. Open the setup chat and tap Approve this bot.",
      ),
    ).toBeVisible();
    request = {
      ...request,
      status: "ready",
      revision: 6,
      telegram_bot_id: "900",
      bot_username: "MobileSupportBot",
    };
    await returned.goto("/channel-bots?connect=telegram-new");
    await expect(returned.getByLabel("Bot label in NyxID")).toHaveValue(
      "Mobile support",
    );
    await expect(
      returned.getByText(
        "You approved @MobileSupportBot in Telegram. Tap Connect bot to finish.",
      ),
    ).toBeVisible();
    await returned
      .getByRole("button", { name: "Connect bot", exact: true })
      .click();
    await expect(returned).toHaveURL(new RegExp(`/channel-bots/${requestId}$`));
    expect(connects).toEqual([{ telegram_bot_id: "900", revision: 6 }]);
    expect(starts).toHaveLength(1);
  });
}
