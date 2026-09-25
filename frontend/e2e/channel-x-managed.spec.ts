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
    "tweet.write",
    "users.read",
    "dm.read",
    "dm.write",
    "media.write",
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
  x_events: ["dm"],
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

for (const width of [1440, 390]) {
  test(`X shows encrypted activity with zero ordinary messages at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    await mockDashboard(page);
    const conversation = { id: "route", channel_bot_id: bot.id, platform: "x", platform_conversation_id: "*", platform_conversation_type: "private", agent_api_key_id: "agent", default_agent: true, allow_agent_initiated: false, is_active: true, created_at: bot.created_at, updated_at: bot.updated_at };
    const activity = { id: "activity", conversation_id: "route", platform_conversation_id: "chat:20:10", platform_event_id: "e4f4d3fc-8bbf-4928-92eb-e5058d6bb6f6", sender_platform_id: "20", kind: "encrypted_chat", provider_event_type: "chat.received", content_availability: "encrypted", reply_supported: false, callback_status: "not_enabled", received_at: new Date().toISOString(), occurred_at: null };
    const response = { activities: [activity], total: 1, retention_days: 30, routes: [{ conversation_id: "route", count: 1, last_activity: activity }], page: 1, per_page: 20 };
    await page.route("**/api/v1/channel-bots/connected-x", (route) => route.fulfill({ json: { ...bot, conversations_count: 1, x_events: ["dm", "chat"], webhook_registered: true, webhook_ingestion: true } }));
    await page.route("**/api/v1/channel-conversations?*", (route) => route.fulfill({ json: { conversations: [conversation], total: 1 } }));
    await page.route("**/api/v1/channel-conversations/route", (route) => route.fulfill({ json: conversation }));
    await page.route("**/api/v1/channel-conversations/route/messages?*", (route) => route.fulfill({ json: { messages: [], total: 0, page: 1, per_page: 50 } }));
    await page.route("**/api/v1/**/activities?*", (route) => route.fulfill({ json: response }));
    let enabled = false;
    await page.route("**/api/v1/channel-conversations/route/activity-callback", async (route) => {
      if (route.request().method() === "PUT") { enabled = route.request().postDataJSON().enabled as boolean; await route.fulfill({ status: 204 }); }
      else await route.fulfill({ json: { declared: true, enabled, version: 1, kinds: ["encrypted_chat"] } });
    });
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/channel-bots/connected-x");
    await expect(page.getByText("1 received activity in the last 30 days")).toBeVisible();
    await expect(page.getByText("Agent notification not enabled")).toBeVisible();
    await expect(page.getByText(/Encrypted chat ·/).filter({ visible: true })).toBeVisible();
    await expect(page.getByRole("checkbox", { name: "Encrypted chat notifications" })).toBeChecked();
    await page.screenshot({ path: `/tmp/nyx-typed-activity-bot-${width}.png`, fullPage: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await page.goto("/channel-bots/connected-x/conversations/route");
    await expect(page.getByRole("heading", { name: "Channel activity", exact: true })).toBeVisible();
    await expect(page.getByText("1 received activity in the last 30 days")).toBeVisible();
    await expect(page.getByText(/No ordinary messages in this route/)).toBeVisible();
    await page.getByRole("button", { name: "Enable typed callbacks" }).click();
    await expect(page.getByRole("button", { name: "Disable typed callbacks" })).toBeVisible();
    expect(enabled).toBe(true);
    await page.screenshot({ path: `/tmp/nyx-typed-activity-route-${width}.png`, fullPage: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    expect(errors).toEqual([]);
  });
}

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
    const updates: unknown[] = [];
    await page.route("**/api/v1/channel-bots/connected-x", async (route) => {
      if (route.request().method() === "PATCH") {
        const update = route.request().postDataJSON() as { x_events: string[] };
        updates.push(update);
        currentBot = {
          ...currentBot,
          x_events: update.x_events,
          webhook_ingestion: true,
          webhook_registered: true,
        };
      }
      await route.fulfill({ json: currentBot });
    });
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
      currentBot = {
        ...currentBot,
        status: "active",
        poll_error_count: 0,
        error: null,
        next_poll_at: "2026-09-08T00:10:00Z",
      };
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
      dialog.getByRole("button", { name: "Connect X (Twitter) account" }),
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
    await dialog
      .getByRole("button", { name: "Connect X (Twitter) account" })
      .click();
    await expect(page).toHaveURL(/channel-bots\/connected-x$/);
    expect(starts).toEqual([{ label: bot.label }]);
    expect(completes).toEqual([
      { connection_id: connection, label: bot.label },
    ]);
    await expect(
      page.getByRole("heading", { name: "Connected account", exact: true }),
    ).toBeVisible();
    await expect(page.getByText("Last polled", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Finish webhook setup", { exact: true }),
    ).toHaveCount(0);
    currentBot = {
      ...currentBot,
      webhook_ingestion: false,
      status: "failed",
      poll_error_count: 5,
      error: "Reconnect the account to resume polling",
    };
    await page.reload();
    await expect(
      page.getByText(currentBot.error, { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Consecutive errors", { exact: true }).locator(".."),
    ).toContainText("5");
    await page.getByRole("button", { name: "Reconnect", exact: true }).click();
    await expect.poll(() => reconnects.length).toBe(1);
    expect(reconnects[0]).toEqual({ connection_id: connection });
    await expect(
      page.getByText("Account reconnected", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Reconnect the account to resume polling", {
        exact: true,
      }),
    ).toHaveCount(0);
    await expect(
      page.getByText("Consecutive errors", { exact: true }).locator(".."),
    ).toContainText("0");
    await expect(
      page.getByText("Next poll", { exact: true }).locator(".."),
    ).toContainText("2026-09-08T00:10:00Z");
    await expect(
      page.getByText("Cursor", { exact: true }).locator(".."),
    ).toContainText("100");
    const save = page.getByRole("button", { name: "Save event subscriptions" });
    await expect(
      page.getByRole("checkbox", { name: "Direct messages" }),
    ).toBeChecked();
    await expect(save).toBeDisabled();
    await page.getByRole("checkbox", { name: "Direct messages" }).uncheck();
    await expect(save).toBeDisabled();
    await expect(page.getByRole("alert")).toContainText("Select at least one");
    await page.getByRole("checkbox", { name: "Direct messages" }).check();
    await page.getByRole("checkbox", { name: "Mentions", exact: true }).check();
    await page.getByRole("checkbox", { name: "Replies to my posts" }).check();
    await expect(save).toBeEnabled();
    await save.click();
    await expect(
      page.getByText("X event subscriptions updated", { exact: true }),
    ).toBeVisible();
    expect(updates).toEqual([{ x_events: ["dm", "mentions", "replies"] }]);
    await expect(save).toBeDisabled();
    await page.reload();
    await expect(
      page.getByRole("checkbox", { name: "Mentions", exact: true }),
    ).toBeChecked();
    await expect(
      page.getByRole("checkbox", { name: "Replies to my posts" }),
    ).toBeChecked();
    await page
      .getByRole("heading", { name: "Events to receive" })
      .locator("..")
      .screenshot({
        path: `/tmp/nyx-x-events-${String(viewport.width)}.png`,
      });
    await page.getByRole("button", { name: "Delete", exact: true }).click();
    await expect(
      page.getByRole("dialog").getByText(/platform account stays connected/),
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
    page
      .getByRole("dialog")
      .getByRole("button", { name: "Connect X (Twitter) account" }),
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
  await page
    .getByRole("button", { name: "Connect X (Twitter) account" })
    .click();
  await expect(
    page.getByText(/Account authorization failed or was cancelled/),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Connect X (Twitter) account" }),
  ).toBeEnabled();
});

test("X event permission errors preserve choices and partial setup errors refresh channel state", async ({
  page,
}) => {
  await mockDashboard(page);
  let currentBot = { ...bot, error: null as string | null };
  let consentGranted = false;
  await page.route("**/channel-bots/managed-onboarding/x", (route) =>
    route.fulfill({ json: bootstrap }),
  );
  await page.route("**/api/v1/channel-bots/connected-x", async (route) => {
    if (route.request().method() === "PATCH") {
      if (!consentGranted) {
        await route.fulfill({
          status: 400,
          json: {
            message: "Reconnect the X account to grant tweet.write",
            error_code: 1000,
          },
        });
        return;
      }
      currentBot = {
        ...currentBot,
        x_events: route.request().postDataJSON().x_events as string[],
        status: "failed",
        error: "Webhook setup did not complete. Select Verify to retry.",
      };
      await route.fulfill({
        status: 502,
        json: { message: "X subscription setup failed", error_code: 10005 },
      });
      return;
    }
    await route.fulfill({ json: currentBot });
  });
  await page.goto("/channel-bots/connected-x");
  await page.getByRole("checkbox", { name: "Mentions", exact: true }).check();
  const save = page.getByRole("button", { name: "Save event subscriptions" });
  await save.click();
  await expect(
    page.getByText("Reconnect the X account to grant tweet.write", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("checkbox", { name: "Mentions", exact: true }),
  ).toBeChecked();
  await expect(save).toBeEnabled();
  consentGranted = true;
  await save.click();
  await expect(
    page.getByText("Webhook setup did not complete. Select Verify to retry.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(save).toBeDisabled();
  await page.route("**/channel-bots/connected-x/verify", async (route) => {
    currentBot = { ...currentBot, status: "active", error: null };
    await route.fulfill({
      json: {
        status: "active",
        platform_bot_id: "10",
        platform_bot_username: "support",
      },
    });
  });
  await page.getByRole("button", { name: "Verify Bot", exact: true }).click();
  await expect(
    page.getByText("Webhook setup did not complete. Select Verify to retry.", {
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("checkbox", { name: "Mentions", exact: true }),
  ).toBeChecked();
});
