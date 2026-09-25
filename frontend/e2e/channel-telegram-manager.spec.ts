import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import { platformFixtures } from "../src/test/fixtures/channel-platforms";
import type {
  ChannelBotDetail,
  CreateChannelConversationRequest,
} from "../src/types/channels";

const botId = "192ad917-380c-4f48-8131-419fc3c54d39";
const agentId = "0a3b2927-bc51-4efe-8fcf-2f5b0962bb18";
const token = "123456:fixture-manager-token";
const managerError =
  "Telegram manager is not ready. Check Admin Platform Credentials.";

for (const { viewport, retryRegistration } of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
].flatMap((viewport) => [false, true].map((retryRegistration) => ({ viewport, retryRegistration })))) {
  test(`existing Telegram manager ${retryRegistration ? "registration recovery" : "registration"}, public routing, verification and deletion at ${String(viewport.width)}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize(viewport);
    const errors: string[] = [];
    const writes: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("request", (request) => {
      if (request.method() !== "GET")
        writes.push(new URL(request.url()).pathname);
    });
    const bot: ChannelBotDetail = {
      id: botId,
      platform: "telegram",
      label: "Public Telegram manager",
      credential_source: "telegram_manager",
      platform_bot_id: "123456",
      platform_bot_username: "NyxSetupBot",
      user_id: "test-user",
      status: "active",
      is_active: true,
      webhook_registered: true,
      conversations_count: 0,
      app_secret_configured: false,
      lark_verification_token_configured: false,
      lark_encrypt_key_configured: false,
      created_at: "2026-09-20T00:00:00Z",
      updated_at: "2026-09-20T00:00:00Z",
      webhook_url:
        "https://api.nyxid.example/api/v1/webhooks/channel/telegram-new/manager",
      error: null,
    };
    let detail = bot;
    let registered = false;
    let deleted = false;
    let failVerify = false;
    let registrationAttempts = 0;
    let registration: unknown;
    let rename: unknown;
    const routes: CreateChannelConversationRequest[] = [];

    await mockDashboard(page);
    await page.route("**/api/v1/users/me", (route) =>
      route.fulfill({
        json: {
          id: "test-user",
          email: "test@example.com",
          display_name: "Test User",
          is_admin: false,
          is_active: true,
          email_verified: true,
          created_at: bot.created_at,
        },
      }),
    );
    await page.route("**/api/v1/channel-platforms", (route) =>
      route.fulfill({
        json: {
          platforms: platformFixtures.map((platform) =>
            platform.platform === "telegram"
              ? {
                  ...platform,
                  registration: {
                    ...platform.registration,
                    fields: platform.registration.fields.map((field) => ({
                      ...field,
                      patchable: true,
                    })),
                  },
                }
              : platform,
          ),
        },
      }),
    );
    await page.route("**/api/v1/api-keys", (route) =>
      route.fulfill({
        json: {
          keys: [
            {
              id: agentId,
              name: "Public support (restricted)",
              is_active: true,
              callback_url: "https://agent.example/callback",
              allow_all_services: false,
              allow_all_nodes: false,
              allowed_service_ids: [],
              allowed_node_ids: [],
            },
          ],
        },
      }),
    );
    await page.route(/\/api\/v1\/channel-bots(?:\?.*)?$/, (route) => {
      if (route.request().method() === "POST") {
        registration = route.request().postDataJSON();
        registered = true;
        registrationAttempts += 1;
        if (retryRegistration && registrationAttempts === 1) {
          detail = { ...detail, status: "failed", webhook_registered: false, error: managerError };
          return route.fulfill({
            status: 400,
            json: { error: "bad_request", error_code: 1000, message: managerError },
          });
        }
        detail = { ...detail, status: "active", webhook_registered: true, error: null };
        return route.fulfill({
          status: retryRegistration ? 200 : 201,
          json: {
            id: botId,
            platform: bot.platform,
            platform_bot_username: bot.platform_bot_username,
            status: "active",
            credential_source: "telegram_manager",
            webhook_secret: null,
            webhook_url: bot.webhook_url,
          },
        });
      }
      return route.fulfill({
        json: {
          bots: registered && !deleted ? [detail] : [],
          total: registered && !deleted ? 1 : 0,
        },
      });
    });
    await page.route(`**/api/v1/channel-bots/${botId}`, (route) => {
      if (route.request().method() === "PATCH") {
        rename = route.request().postDataJSON();
        detail = {
          ...detail,
          label: route.request().postDataJSON().label as string,
        };
        return route.fulfill({ json: detail });
      }
      if (route.request().method() === "DELETE") {
        deleted = true;
        return route.fulfill({ status: 204 });
      }
      return route.fulfill({ json: detail });
    });
    await page.route(`**/api/v1/channel-bots/${botId}/verify`, (route) => {
      detail = {
        ...detail,
        status: failVerify ? "failed" : "active",
        webhook_registered: !failVerify,
        error: failVerify ? managerError : null,
      };
      return route.fulfill(
        failVerify
          ? {
              status: 409,
              json: {
                error: "conflict",
                error_code: 1000,
                message: managerError,
              },
            }
          : { json: { id: botId, status: "active", webhook_registered: true } },
      );
    });
    await page.route("**/api/v1/channel-conversations**", (route) => {
      if (route.request().method() === "POST") {
        routes.push(
          route.request().postDataJSON() as CreateChannelConversationRequest,
        );
        detail = { ...detail, conversations_count: routes.length };
        return route.fulfill({
          status: 201,
          json: { id: `route-${String(routes.length)}` },
        });
      }
      const conversations =
        new URL(route.request().url()).searchParams.get("bot_id") === botId
          ? routes.map((saved, index) => ({
              ...saved,
              id: `route-${String(index)}`,
              platform: "telegram",
              platform_conversation_id: saved.platform_conversation_id || "*",
              platform_conversation_type: "private",
              is_active: true,
              last_message_at: null,
              created_at: bot.created_at,
              updated_at: bot.updated_at,
            }))
          : [];
      return route.fulfill({
        json: { conversations, total: conversations.length },
      });
    });

    await page.goto("/channel-bots");
    await page
      .getByRole("button", { name: "Add Bot", exact: true })
      .first()
      .click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("combobox", { name: "Platform" }).click();
    await page
      .getByRole("option", { name: "Telegram bot token", exact: true })
      .click();
    await expect(
      dialog.getByText(/To connect an existing Telegram manager/),
    ).toBeVisible();
    await dialog.getByLabel("Label", { exact: true }).fill(bot.label);
    await dialog.getByLabel("Bot token", { exact: true }).fill(token);
    await expect(
      dialog.getByLabel("Bot token", { exact: true }),
    ).toHaveAttribute("type", "password");
    await dialog.getByRole("button", { name: "Add Bot", exact: true }).click();
    if (retryRegistration) {
      await expect(page.getByText(managerError, { exact: true })).toBeVisible();
      await expect(dialog).toBeVisible();
      await dialog.getByRole("button", { name: "Add Bot", exact: true }).click();
    }
    await expect(page).toHaveURL(new RegExp(`/channel-bots/${botId}$`));
    expect(registrationAttempts).toBe(retryRegistration ? 2 : 1);
    expect(registration).toEqual({
      platform: "telegram",
      label: bot.label,
      bot_token: token,
    });
    await expect(
      page.getByText("Telegram manager", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Edit Credentials", { exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Save Credentials" }),
    ).toHaveCount(0);
    await expect(page.locator('input[type="password"]')).toHaveCount(0);
    await expect(page.getByText("Webhook Secret", { exact: true })).toHaveCount(
      0,
    );
    await expect(page.getByText(token, { exact: true })).toHaveCount(0);
    expect(
      await page.evaluate(() => JSON.stringify([localStorage, sessionStorage])),
    ).not.toContain(token);
    await expect(
      page.getByText(
        /All manager chat callbacks carry signed callback authentication/,
      ),
    ).toContainText("They do not carry the owner's X-NyxID-User-Token");
    await expect(
      page.getByText(/Telegram senders are not mapped/),
    ).toContainText("including on exact chat routes");
    await expect(
      page.getByText(/all callback queries remain reserved/),
    ).toContainText("32 concurrent requests");
    await expect(
      page.getByText(/Rotate the manager token in Admin/),
    ).toBeVisible();
    await expect(
      page.getByText('Telegram manager "NyxSetupBot" connected', {
        exact: true,
      }),
    ).toBeHidden({ timeout: 10_000 });

    await page.getByRole("button", { name: "Edit name", exact: true }).click();
    await expect(dialog.getByLabel("Bot name", { exact: true })).toHaveValue(
      bot.label,
    );
    await expect(
      dialog.getByRole("button", { name: "Save name", exact: true }),
    ).toBeDisabled();
    const renamedLabel = "Renamed public Telegram manager";
    await dialog.getByLabel("Bot name", { exact: true }).fill(renamedLabel);
    await dialog
      .getByRole("button", { name: "Save name", exact: true })
      .click();
    await expect(dialog).toHaveCount(0);
    await expect(
      page.getByRole("heading", { name: renamedLabel, exact: true }),
    ).toBeVisible();
    expect(rename).toEqual({ label: renamedLabel });
    await expect(
      page.getByText("Telegram manager", { exact: true }),
    ).toBeVisible();
    await expect(page.locator('input[type="password"]')).toHaveCount(0);
    await expect(
      page.getByText("Bot name updated", { exact: true }),
    ).toBeHidden({ timeout: 10_000 });

    await page.getByRole("button", { name: "Add Route", exact: true }).click();
    await dialog.getByRole("combobox", { name: "Agent (API Key)" }).click();
    await page
      .getByRole("option", { name: "Public support (restricted)", exact: true })
      .click();
    await expect(
      dialog.getByRole("button", { name: "Add Route", exact: true }),
    ).toBeDisabled();
    await dialog.getByLabel("Telegram chat ID", { exact: true }).fill("   ");
    await expect(
      dialog.getByRole("button", { name: "Add Route", exact: true }),
    ).toBeDisabled();
    expect(routes).toHaveLength(0);
    await dialog
      .getByRole("switch", { name: "Use a public default route" })
      .click();
    await expect(
      dialog.getByText(/Choose a dedicated agent with restricted permissions/),
    ).toBeVisible();
    await expect(
      dialog.getByLabel("Telegram chat ID", { exact: true }),
    ).toHaveCount(0);
    await dialog
      .getByRole("heading", { name: "Add Conversation Route" })
      .scrollIntoViewIfNeeded();
    await expect(
      dialog.getByRole("heading", { name: "Add Conversation Route" }),
    ).toBeInViewport();
    await dialog.screenshot({ path: testInfo.outputPath("public-route.png") });
    expect(
      await dialog.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    await dialog
      .getByRole("button", { name: "Add Route", exact: true })
      .click();
    await expect(dialog).toHaveCount(0);
    expect(routes).toEqual([
      { channel_bot_id: botId, agent_api_key_id: agentId, default_agent: true },
    ]);
    await expect(
      viewport.width >= 768
        ? page.getByRole("cell", { name: "Default", exact: true })
        : page.getByText("Default", { exact: true }).filter({ visible: true }),
    ).toBeVisible();

    for (const [index, chatId] of ["123456789", "-1001234567890"].entries()) {
      await page
        .getByRole("button", { name: "Add Route", exact: true })
        .click();
      await expect(
        dialog.getByRole("switch", { name: "Use a public default route" }),
      ).not.toBeChecked();
      await dialog.getByRole("combobox", { name: "Agent (API Key)" }).click();
      await page
        .getByRole("option", {
          name: "Public support (restricted)",
          exact: true,
        })
        .click();

      if (index === 0) {
        for (const wildcard of ["*", " * "]) {
          await dialog
            .getByLabel("Telegram chat ID", { exact: true })
            .fill(wildcard);
          await expect(
            dialog.getByRole("button", { name: "Add Route", exact: true }),
          ).toBeDisabled();
          // Exercise the submit guard independently of the disabled button.
          await dialog
            .locator("form")
            .evaluate((form: HTMLFormElement) => form.requestSubmit());
          await expect(
            dialog.getByText(
              "Enter an exact Telegram chat ID or enable the public default route.",
              { exact: true },
            ),
          ).toBeVisible();
          expect(
            writes.filter((path) => path === "/api/v1/channel-conversations"),
          ).toHaveLength(1);
          expect(routes).toEqual([
            {
              channel_bot_id: botId,
              agent_api_key_id: agentId,
              default_agent: true,
            },
          ]);
        }
      }

      await dialog.getByLabel("Telegram chat ID", { exact: true }).fill(chatId);
      await dialog
        .getByRole("button", { name: "Add Route", exact: true })
        .click();
      await expect(dialog).toHaveCount(0);
      expect(routes[index + 1]).toEqual({
        channel_bot_id: botId,
        agent_api_key_id: agentId,
        default_agent: false,
        platform_conversation_id: chatId,
      });
    }

    await page.getByRole("button", { name: "Verify Bot", exact: true }).click();
    await expect(
      page
        .getByRole("status")
        .filter({ hasText: "Verification complete. Status: Active." }),
    ).toBeVisible();
    failVerify = true;
    await page.getByRole("button", { name: "Verify Bot", exact: true }).click();
    await expect(
      page.getByRole("alert").filter({ hasText: managerError }),
    ).toBeVisible();
    await expect(page.getByText("Failed", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Not registered", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Verification complete. Status: Active.", { exact: true }),
    ).toHaveCount(0);
    await page.reload();
    await expect(
      page.getByRole("alert").filter({ hasText: managerError }),
    ).toBeVisible();
    await page.screenshot({
      path: testInfo.outputPath("verify-failure.png"),
      fullPage: true,
    });
    failVerify = false;
    await page.getByRole("button", { name: "Verify Bot", exact: true }).click();
    await expect(
      page
        .getByRole("status")
        .filter({ hasText: "Verification complete. Status: Active." }),
    ).toBeVisible();
    await expect(
      page.getByRole("alert").filter({ hasText: managerError }),
    ).toHaveCount(0);
    await page.screenshot({
      path: testInfo.outputPath("manager-detail.png"),
      fullPage: true,
    });
    await page
      .getByText(
        /All manager chat callbacks carry signed callback authentication/,
      )
      .scrollIntoViewIfNeeded();
    await page.screenshot({ path: testInfo.outputPath("manager-routing.png") });

    await page.goto("/channel-bots");
    await page
      .getByRole("button", { name: `Delete ${renamedLabel}`, exact: true })
      .click();
    await expect(
      dialog.getByText(
        /Bot creation and the manager webhook will remain available/,
      ),
    ).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    expect(deleted).toBe(false);
    await page
      .getByText(renamedLabel, { exact: true })
      .filter({ visible: true })
      .click();
    await expect(page).toHaveURL(new RegExp(`/channel-bots/${botId}$`));
    await page.getByRole("button", { name: "Delete", exact: true }).click();
    await expect(
      dialog.getByText(
        /Bot creation and the manager webhook will remain available/,
      ),
    ).toBeVisible();
    await expect(
      dialog.getByText(/The manager token stays in Admin/),
    ).toBeVisible();
    await dialog.screenshot({
      path: testInfo.outputPath("delete-connection.png"),
    });
    await dialog.getByRole("button", { name: "Delete", exact: true }).click();
    await expect(page).toHaveURL(/\/channel-bots$/);
    expect(deleted).toBe(true);
    expect(writes).toEqual([
      "/api/v1/channel-bots",
      ...(retryRegistration ? ["/api/v1/channel-bots"] : []),
      `/api/v1/channel-bots/${botId}`,
      "/api/v1/channel-conversations",
      "/api/v1/channel-conversations",
      "/api/v1/channel-conversations",
      ...Array<string>(3).fill(`/api/v1/channel-bots/${botId}/verify`),
      `/api/v1/channel-bots/${botId}`,
    ]);
    expect(errors).toEqual([]);
  });
}
