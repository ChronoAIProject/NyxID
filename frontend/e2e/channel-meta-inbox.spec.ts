import { expect } from "@playwright/test";
import { test } from "./production-policy-fixture";
import { platformFixture } from "../src/test/fixtures/channel-platforms";
import { managedBot, mockDashboard } from "./managed-onboarding-fixtures";

const journeys = [
  { platform: "messenger", retryState: null, held: false },
  { platform: "instagram", retryState: null, held: false },
  { platform: "messenger", retryState: "completed", held: false },
  { platform: "instagram", retryState: "installing", held: false },
  { platform: "instagram", retryState: null, held: true },
] as const;

for (const { platform, retryState, held } of journeys) {
  const journey = held
    ? "shows when restored inbox access is waiting for explicit resume"
    : retryState
      ? "recovers the selected inbox after a subscription error and reload"
      : "consent returns to the selected customer inbox";
  test(`${platform} ${journey}`, async ({ page, context }) => {
    await mockDashboard(page);
    const session = "6d00d880-3ae5-4188-95d4-79e6746b7f0d";
    const provider =
      platform === "messenger" ? "www.facebook.com" : "www.instagram.com";
    const authorizePath =
      platform === "messenger" ? "/v25.0/dialog/oauth" : "/oauth/authorize";
    const displayName = platform === "messenger" ? "Messenger" : "Instagram";
    const inbox = { id: "20000000000000002", name: "Customer Support" };
    const state = "test-inbox-state";
    let authorized = false;
    const finishRequests: unknown[] = [];
    let subscriptionsVerified = false;
    await page.route("**/api/v1/channel-platforms", (route) =>
      route.fulfill({
        json: {
          platforms: [
            {
              ...platformFixture(platform),
              display_name: displayName,
              capabilities: {
                initiated_send: false,
                reply_to: false,
                thread: false,
                edit: false,
                media: { inbound: [], outbound: [] },
              },
              managed_onboarding: {
                flow: "meta_oauth",
                provider: platform,
                bootstrap_fields: [],
                completion_fields: ["asset_id", "exclusive_routing_verified"],
              },
            },
          ],
        },
      }),
    );
    await page.route(
      `**/api/v1/channel-bots/managed-onboarding/${platform}`,
      (route) =>
        route.fulfill({
          json: {
            available: true,
            flow: "meta_oauth",
            provider_slug: platform,
          },
        }),
    );
    await page.route(`**/api/v1/channel-bots/meta/${platform}/start`, (route) =>
      route.fulfill({
        json: {
          session_id: session,
          authorization_url: `https://${provider}${authorizePath}?state=${state}`,
        },
      }),
    );
    // The popup has its own Page. Intercept its signed-in identity and code exchange at the context level.
    await context.route("**/api/v1/users/me", (route) =>
      route.fulfill({
        json: {
          id: "test-user",
          email: "test@example.com",
          display_name: "Test User",
          is_active: true,
          email_verified: true,
          created_at: managedBot.created_at,
        },
      }),
    );
    await context.route(`https://${provider}${authorizePath}?**`, (route) =>
      route.fulfill({
        status: 302,
        headers: {
          location: `${new URL(page.url()).origin}/channel-bots/meta-callback/${platform}?code=inbox-code&state=${state}`,
        },
      }),
    );
    await context.route(
      `**/api/v1/channel-bots/meta/${platform}/exchange`,
      async (route) => {
        expect(route.request().postDataJSON()).toEqual({
          code: "inbox-code",
          state,
        });
        authorized = true;
        await route.fulfill({
          json: {
            session_id: session,
            platform,
            status: "assets",
            bot_id: null,
            assets: [inbox],
          },
        });
      },
    );
    await page.route(
      `**/api/v1/channel-bots/meta-sessions/${session}`,
      (route) =>
        route.fulfill({
          json: {
            session_id: session,
            platform,
            status: subscriptionsVerified
              ? "completed"
              : retryState && finishRequests.length
                ? retryState
                : authorized
                  ? "assets"
                  : "pending",
            selected_asset_id: finishRequests.length ? inbox.id : null,
            subscription_state: subscriptionsVerified
              ? held
                ? "suspended"
                : "active"
              : finishRequests.length
                ? "pending"
                : null,
            bot_id: finishRequests.length ? "connected-inbox" : null,
            assets:
              authorized && !finishRequests.length
                ? platform === "messenger"
                  ? [{ id: "10000000000000001", name: "Other Page" }, inbox]
                  : [inbox]
                : [],
          },
        }),
    );
    await page.route(
      `**/api/v1/channel-bots/meta-sessions/${session}/finish`,
      async (route) => {
        finishRequests.push(route.request().postDataJSON());
        if (retryState && finishRequests.length === 1) {
          await route.fulfill({
            status: 502,
            json: {
              error: "channel_platform_error",
              error_code: 10005,
              message:
                "Meta subscription readback failed; select Verify to retry",
            },
          });
          return;
        }
        subscriptionsVerified = true;
        await route.fulfill({
          json: {
            id: "connected-inbox",
            label: "Support",
            platform,
            platform_bot_username: inbox.name,
            status: held ? "suspended" : "active",
            credential_source: "platform",
            webhook_ingestion: true,
            webhook_url: `https://api.nyxid.example/api/v1/webhooks/channel/${platform}/platform`,
          },
        });
      },
    );
    await page.goto(`/channel-bots/connect/${platform}?label=Support`);
    await page
      .getByRole("button", { name: /sign in and grant inbox access/i })
      .click();
    await page.getByRole("radio", { name: /Customer Support/ }).check();
    const connect = page.getByRole("button", {
      name: /connect selected inbox/i,
    });
    await expect(connect).toBeDisabled();
    await page.getByRole("checkbox", { name: /exclusive/i }).check();
    await connect.click();
    const finishRequest = {
      asset_id: inbox.id,
      exclusive_routing_verified: true,
    };
    if (retryState) {
      await expect(
        page.getByText(/Meta subscription readback failed/),
      ).toBeVisible();
      await page.reload();
      const retry = page.getByRole("button", { name: "Retry verification" });
      await expect(retry).toBeVisible();
      await expect(
        page.getByRole("status").filter({ hasText: inbox.id }),
      ).toBeVisible();
      await expect(
        page.getByRole("link", { name: "Open connected inbox" }),
      ).toHaveCount(0);
      await expect(
        page.getByRole("button", { name: "Open channel bot" }),
      ).toHaveCount(0);
      expect(finishRequests).toEqual([finishRequest]);
      await retry.click();
    }
    await expect
      .poll(() => finishRequests)
      .toEqual(retryState ? [finishRequest, finishRequest] : [finishRequest]);
    if (held) {
      await expect(
        page
          .getByRole("status")
          .filter({ hasText: "Resume delivery to complete setup." }),
      ).toBeVisible();
      await expect(
        page.getByRole("link", { name: "Open connected inbox" }),
      ).toBeVisible();
      return;
    }
    await expect(
      page
        .getByRole("link", { name: /open channel bot/i })
        .or(page.getByRole("button", { name: /open channel bot/i })),
    ).toBeVisible();
  });
}

test("declining Instagram consent leaves the inbox disconnected", async ({
  page,
}) => {
  await mockDashboard(page);
  const state = "cancelled-inbox-state";
  let exchanges = 0;
  let installs = 0;
  await page.route(
    "**/api/v1/channel-bots/meta-sessions/*/finish",
    async (route) => {
      installs++;
      await route.fulfill({ status: 409 });
    },
  );
  await page.route(
    "**/api/v1/channel-bots/meta/instagram/exchange",
    async (route) => {
      expect(route.request().postDataJSON()).toEqual({ state, code: null });
      exchanges++;
      await route.fulfill({
        json: {
          session_id: "6d00d880-3ae5-4188-95d4-79e6746b7f0d",
          platform: "instagram",
          status: "cancelled",
          bot_id: null,
          assets: [],
        },
      });
    },
  );
  await page.goto(
    `/channel-bots/meta-callback/instagram?state=${state}&error=access_denied`,
  );
  await expect(page.getByRole("status")).toHaveText(
    /cancelled|canceled|declined/i,
  );
  await expect(page).toHaveURL(/\/meta-callback\/instagram$/);
  expect(exchanges).toBe(1);
  expect(installs).toBe(0);
});

test("expired NyxID session clears the Meta callback without forwarding its code to login", async ({
  page,
}) => {
  await mockDashboard(page);
  await page.route("**/api/v1/users/me", (route) =>
    route.fulfill({ status: 401, json: { error: "unauthorized" } }),
  );
  let exchanges = 0;
  await page.route(
    "**/api/v1/channel-bots/meta/messenger/exchange",
    (route) => {
      exchanges++;
      return route.fulfill({ status: 401 });
    },
  );
  await page.goto(
    "/channel-bots/meta-callback/messenger?state=inbox-state&code=expired-session-code",
  );
  await expect(page.getByRole("status")).toHaveText(/sign in to NyxID again/i);
  await expect(page).toHaveURL(/\/meta-callback\/messenger$/);
  expect(exchanges).toBe(0);
});

test("an existing managed inbox can reconnect while new enrollments are closed", async ({
  page,
  context,
}) => {
  await mockDashboard(page);
  const botId = "existing-support";
  await page.route("**/api/v1/channel-platforms", (route) =>
    route.fulfill({ json: { platforms: [] } }),
  );
  await page.route(`**/api/v1/channel-bots/${botId}`, (route) =>
    route.fulfill({
      json: {
        ...managedBot,
        id: botId,
        platform: "messenger",
        label: "Existing support",
        platform_bot_id: "222",
        managed_setup: null,
        webhook_registered: false,
        status: "failed",
      },
    }),
  );
  let pinned: unknown;
  await context.route(
    "https://www.facebook.com/v25.0/dialog/oauth?**",
    (route) => route.fulfill({ body: "Provider consent" }),
  );
  await page.route(
    "**/api/v1/channel-bots/meta/messenger/start",
    async (route) => {
      pinned = route.request().postDataJSON();
      await route.fulfill({
        json: {
          session_id: "6d00d880-3ae5-4188-95d4-79e6746b7f0d",
          authorization_url:
            "https://www.facebook.com/v25.0/dialog/oauth?state=test",
        },
      });
    },
  );
  await page.goto(`/channel-bots/${botId}`);
  await page.locator("summary").filter({ hasText: "Reconnect inbox" }).click();
  await page
    .getByRole("button", { name: "Reconnect inbox", exact: true })
    .click();
  await expect
    .poll(() => pinned)
    .toEqual({ label: "Existing support", reconnect_bot_id: botId });
});
