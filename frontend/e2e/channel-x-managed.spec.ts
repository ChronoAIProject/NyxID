import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";

const connection = "11111111-1111-4111-8111-111111111111";
const nonce = "22222222-2222-4222-8222-222222222222";
const bootstrap = {
  available: true,
  flow: "oauth_connection",
  provider_slug: "twitter",
  required_scopes: [
    "tweet.read",
    "users.read",
    "dm.read",
    "dm.write",
    "offline.access",
  ],
  authorize_start_url: "/channel-bots/managed-onboarding/x/start",
};
const bot = {
  id: "connected-x",
  platform: "x",
  label: "DM Support",
  platform_bot_id: "10",
  platform_bot_username: "support",
  status: "active",
  is_active: true,
  credential_source: "connection",
  connection_id: connection,
  webhook_ingestion: false,
  webhook_registered: false,
  webhook_url: "",
  conversations_count: 0,
  app_secret_configured: false,
  lark_verification_token_configured: false,
  lark_encrypt_key_configured: false,
  poll_cursor: "100",
  last_polled_at: "2026-09-08T00:00:00Z",
  next_poll_at: "2026-09-08T00:01:00Z",
  poll_error_count: 0,
  created_at: "2026-09-08T00:00:00Z",
  updated_at: "2026-09-08T00:00:00Z",
  user_id: "test-user",
};

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  test(`X OAuth connect and reconnect at ${String(viewport.width)}px`, async ({
    page,
    context,
  }) => {
    await page.setViewportSize(viewport);
    await mockDashboard(page);
    const errors: string[] = [];
    let currentBot = { ...bot, error: null as string | null };
    page.on("pageerror", (error) => errors.push(error.message));
    await page.route("**/channel-bots/managed-onboarding/x", (route) =>
      route.fulfill({ json: bootstrap }),
    );
    await page.route("**/api/v1/channel-bots/connected-x", (route) =>
      route.fulfill({ json: currentBot }),
    );
    const starts: unknown[] = [];
    const completes: unknown[] = [];
    const reconnects: unknown[] = [];
    await page.route(
      "**/channel-bots/managed-onboarding/x/start",
      async (route) => {
        starts.push(route.request().postDataJSON());
        await route.fulfill({
          json: {
            connection_id: connection,
            attempt_nonce: nonce,
            authorization_url: `https://x.com/i/oauth2/authorize?state=1cc_${nonce}`,
          },
        });
      },
    );
    await page.route(
      "**/channel-bots/managed-onboarding/x/complete",
      async (route) => {
        completes.push(route.request().postDataJSON());
        await route.fulfill({ json: bot });
      },
    );
    await page.route("**/channel-bots/connected-x/reconnect", async (route) => {
      reconnects.push(route.request().postDataJSON());
      currentBot = { ...currentBot, status: "active", poll_error_count: 0, error: null,
        next_poll_at: "2026-09-08T00:10:00Z" };
      await route.fulfill({ json: { ok: true } });
    });
    await context.route("https://x.com/i/oauth2/authorize?**", (route) =>
      route.fulfill({
        status: 302,
        headers: {
          location: `${new URL(page.url()).origin}/oauth-complete?status=complete&flow=cc&nonce=${nonce}`,
        },
      }),
    );
    await page.goto("/channel-bots?connect=x&label=DM%20Support");
    const dialog = page.getByRole("dialog");
    await expect(
      dialog.getByRole("button", { name: "Connect X account" }),
    ).toBeVisible();
    await expect(dialog.getByLabel("Bot Token", { exact: true })).toHaveCount(
      0,
    );
    await expect(
      dialog.getByLabel("Access Token", { exact: true }),
    ).toHaveCount(0);
    await dialog.screenshot({
      path: `/tmp/nyx-x-connect-${String(viewport.width)}.png`,
    });
    expect(
      await dialog.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    await dialog.getByRole("button", { name: "Connect X account" }).click();
    await expect(page).toHaveURL(/channel-bots\/connected-x$/);
    expect(starts).toEqual([{ label: bot.label }]);
    expect(completes).toEqual([
      { connection_id: connection, label: bot.label },
    ]);
    await expect(
      page.getByText("Connected X account", { exact: true }),
    ).toBeVisible();
    await expect(page.getByText("Last polled", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Finish webhook setup", { exact: true }),
    ).toHaveCount(0);
    currentBot = { ...currentBot, status: "failed", poll_error_count: 5, error: "Reconnect the account to resume polling" };
    await page.reload();
    await expect(page.getByText(currentBot.error, { exact: true })).toBeVisible();
    await expect(page.getByText("Consecutive errors", { exact: true }).locator("..")).toContainText("5");
    await page.getByRole("button", { name: "Reconnect", exact: true }).click();
    await expect.poll(() => reconnects.length).toBe(1);
    expect(reconnects[0]).toEqual({ connection_id: connection });
    await expect(page.getByText("Account reconnected", { exact: true })).toBeVisible();
    await expect(page.getByText("Reconnect the account to resume polling", { exact: true })).toHaveCount(0);
    await expect(page.getByText("Consecutive errors", { exact: true }).locator("..")).toContainText("0");
    await expect(page.getByText("Next poll", { exact: true }).locator("..")).toContainText("2026-09-08T00:10:00Z");
    await expect(page.getByText("Cursor", { exact: true }).locator("..")).toContainText("100");
    await page.getByRole("button", { name: "Delete", exact: true }).click();
    await expect(
      page.getByRole("dialog").getByText(/OAuth connection stays connected/),
    ).toBeVisible();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Cancel", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.screenshot({
      path: `/tmp/nyx-x-detail-${String(viewport.width)}.png`,
      fullPage: true,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    expect(errors).toEqual([]);
  });
}

test("X is unavailable until platform credentials are configured", async ({
  page,
}) => {
  await mockDashboard(page);
  await page.route("**/channel-bots/managed-onboarding/x", (route) =>
    route.fulfill({ json: { ...bootstrap, available: false } }),
  );
  await page.goto("/channel-bots?connect=x");
  await expect(page.getByRole("dialog").getByRole("status")).toHaveText(
    "Not available until an admin configures X (Twitter).",
  );
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Connect X account" }),
  ).toHaveCount(0);
});

test("X OAuth denial leaves the dialog retryable", async ({
  page,
  context,
}) => {
  await mockDashboard(page);
  await page.route("**/channel-bots/managed-onboarding/x", (route) =>
    route.fulfill({ json: bootstrap }),
  );
  await page.route("**/channel-bots/managed-onboarding/x/start", (route) =>
    route.fulfill({
      json: {
        connection_id: connection,
        attempt_nonce: nonce,
        authorization_url: `https://x.com/i/oauth2/authorize?state=1cc_${nonce}`,
      },
    }),
  );
  await context.route("https://x.com/i/oauth2/authorize?**", (route) =>
    route.fulfill({
      status: 302,
      headers: {
        location: `${new URL(page.url()).origin}/oauth-complete?status=error&flow=cc&code=access_denied&nonce=${nonce}`,
      },
    }),
  );
  await page.goto("/channel-bots?connect=x&label=Support");
  await page.getByRole("button", { name: "Connect X account" }).click();
  await expect(
    page.getByText(/Account authorization failed or was cancelled/),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Connect X account" }),
  ).toBeEnabled();
});
