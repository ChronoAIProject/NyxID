import { expect, test } from "@playwright/test";
import {
  mockDashboard,
  managedBot,
  managedBootstrap,
} from "./managed-onboarding-fixtures";
import { channelPlatforms } from "./channel-platform-fixtures";
import type { TelegramNewRequest } from "../src/schemas/telegram-new";

const futurePlatform = {
  ...channelPlatforms.find((platform) => platform.platform === "lark")!,
  platform: "future-chat",
  display_name: "Future Chat",
  registration: {
    ...channelPlatforms.find((platform) => platform.platform === "lark")!
      .registration,
    fields: [
      {
        ...channelPlatforms.find((platform) => platform.platform === "lark")!
          .registration.fields[0]!,
        name: "field1",
        label: "Workspace identifier",
        required: true,
        secret: false,
      },
      {
        ...channelPlatforms.find((platform) => platform.platform === "lark")!
          .registration.fields[1]!,
        name: "field2",
        label: "Workspace credential",
        required: true,
        secret: true,
      },
    ],
  },
};

for (const platform of [...channelPlatforms, futurePlatform].filter(
  (descriptor) =>
    !descriptor.managed_only && descriptor.registration.fields.length,
)) {
  test(`standalone ${platform.platform} validates and submits every catalog field`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await mockDashboard(page);
    await page.route("**/api/v1/channel-platforms", (route) =>
      route.fulfill({
        json: { platforms: [...channelPlatforms, futurePlatform] },
      }),
    );
    const values = Object.fromEntries(
      platform.registration.fields.map((field) => [
        field.name,
        ["phone_number_id", "waba_id", "app_id", "field1"].includes(field.name)
          ? "90071992547409931234"
          : `fixture+/${field.name}=&value`,
      ]),
    );
    const writes: unknown[] = [];
    await page.route("**/api/v1/channel-bots", (route) => {
      writes.push(route.request().postDataJSON());
      return route.fulfill({
        json: {
          ...managedBot,
          id: `created-${platform.platform}`,
          platform: platform.platform,
          webhook_url: `https://nyxid.example/callback/${platform.platform}`,
          setup_instructions: [
            `Complete ${platform.display_name} setup using this callback URL.`,
          ],
          webhook_secret:
            platform.platform === "whatsapp" ? "one-time-verify-token" : null,
        },
      });
    });
    const search = new URLSearchParams({
      label: "Support",
      platform: "ignored-platform",
      unrelated: "ignored",
      ...values,
    });
    await page.goto(`/channel-bots/connect/${platform.platform}?${search}`);
    await expect(page.getByRole("heading", { level: 1 })).toContainText(
      "Create your",
    );
    for (const field of platform.registration.fields) {
      const input = page.getByLabel(
        `${field.label}${field.required ? "" : " (optional)"}`,
        { exact: true },
      );
      await expect(input).toHaveValue(values[field.name]!);
      await expect(input).toHaveAttribute(
        "type",
        field.secret ? "password" : "text",
      );
      if (field.secret)
        await expect
          .poll(() => new URL(page.url()).searchParams.has(field.name))
          .toBe(false);
    }
    expect(writes).toEqual([]);
    const submit = page.getByRole("button", {
      name: "Create channel bot",
      exact: true,
    });
    await expect(submit).toBeEnabled();
    const required = platform.registration.fields.find(
      (field) => field.required,
    )!;
    const requiredInput = page.getByLabel(required.label, { exact: true });
    await requiredInput.fill("");
    await expect(submit).toBeDisabled();
    await requiredInput.fill(values[required.name]!);
    await submit.click();
    await expect(page.getByRole("heading", { level: 1 })).toContainText(
      "is created",
    );
    expect(writes).toEqual([
      { platform: platform.platform, label: "Support", ...values },
    ]);
    await expect(
      page.getByRole("button", { name: "Open channel bot" }),
    ).toBeVisible();
    await expect(
      page.getByText(`https://nyxid.example/callback/${platform.platform}`, {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByText(
        `Complete ${platform.display_name} setup using this callback URL.`,
        { exact: true },
      ),
    ).toBeVisible();
    await expect(page).toHaveURL(
      new RegExp(`/channel-bots/connect/${platform.platform}`),
    );
    if (platform.platform === "whatsapp")
      await expect(
        page.getByText("one-time-verify-token", { exact: true }),
      ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });
}

test("standalone X completes OAuth and waits for the user to open their bot", async ({
  page,
  context,
}) => {
  await mockDashboard(page);
  const connection = "11111111-1111-4111-8111-111111111111";
  const nonce = "22222222-2222-4222-8222-222222222222";
  const starts: unknown[] = [];
  const completions: unknown[] = [];
  await page.route("**/channel-bots/managed-onboarding/x", (route) =>
    route.fulfill({ json: { available: true, flow: "oauth_connection" } }),
  );
  await page.route("**/channel-bots/managed-onboarding/x/start", (route) => {
    starts.push(route.request().postDataJSON());
    return route.fulfill({
      json: {
        connection_id: connection,
        attempt_nonce: nonce,
        authorization_url: `https://x.com/i/oauth2/authorize?state=1cc_${nonce}`,
      },
    });
  });
  await page.route("**/channel-bots/managed-onboarding/x/complete", (route) => {
    completions.push(route.request().postDataJSON());
    return route.fulfill({
      json: { ...managedBot, id: "created-x", platform: "x" },
    });
  });
  await context.route("https://x.com/i/oauth2/authorize?**", (route) =>
    route.fulfill({
      status: 302,
      headers: {
        location: `${new URL(page.url()).origin}/oauth-complete?status=complete&flow=cc&nonce=${nonce}`,
      },
    }),
  );
  await page.goto("/channel-bots/connect/x?label=DM%20Support");
  await expect(
    page.getByRole("button", { name: "Connect X (Twitter) account" }),
  ).toBeEnabled();
  await expect(page.getByRole("button", { name: /Cancel/ })).toHaveCount(0);
  await expect(page.getByLabel("Bot token", { exact: true })).toHaveCount(0);
  expect(starts).toEqual([]);
  await page
    .getByRole("button", { name: "Connect X (Twitter) account" })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Your X (Twitter) channel bot is created",
    }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/channel-bots\/connect\/x/);
  expect(starts).toEqual([{ label: "DM Support" }]);
  expect(completions).toEqual([
    { connection_id: connection, label: "DM Support" },
  ]);
  await expect(
    page.getByRole("button", { name: "Open channel bot" }),
  ).toBeVisible();
});

for (const isolatedPopup of [false, true]) {
  test(`standalone OAuth ${isolatedPopup ? "keeps listening after COOP isolation" : "allows retry after the popup closes"}`, async ({
    page,
    context,
  }) => {
    await mockDashboard(page);
    const nonce = "22222222-2222-4222-8222-222222222222";
    let starts = 0;
    let completions = 0;
    await page.route("**/channel-bots/managed-onboarding/x", (route) =>
      route.fulfill({ json: { available: true, flow: "oauth_connection" } }),
    );
    await page.route("**/channel-bots/managed-onboarding/x/start", (route) => {
      starts++;
      return route.fulfill({
        json: {
          connection_id: "11111111-1111-4111-8111-111111111111",
          attempt_nonce: nonce,
          authorization_url: `https://x.com/i/oauth2/authorize?state=1cc_${nonce}`,
        },
      });
    });
    await page.route(
      "**/channel-bots/managed-onboarding/x/complete",
      (route) => {
        completions++;
        return route.fulfill({ json: { ...managedBot, platform: "x" } });
      },
    );
    await context.route("https://x.com/i/oauth2/authorize?**", (route) =>
      route.fulfill({
        contentType: "text/html",
        headers: isolatedPopup
          ? { "Cross-Origin-Opener-Policy": "same-origin" }
          : {},
        body: "<h1>Provider authorization</h1>",
      }),
    );
    await page.goto("/channel-bots/connect/x");
    const opened = context.waitForEvent("page");
    await page
      .getByRole("button", { name: "Connect X (Twitter) account" })
      .click();
    const popup = await opened;
    await expect(
      popup.getByRole("heading", { name: "Provider authorization" }),
    ).toBeVisible();
    if (!isolatedPopup) await popup.close();
    const retry = page.getByRole("button", { name: "Retry connection" });
    await expect(retry).toBeEnabled();
    expect(completions).toBe(0);
    if (isolatedPopup) {
      await popup.goto(
        `${new URL(page.url()).origin}/oauth-complete?status=complete&flow=cc&nonce=${nonce}`,
      );
      await expect(
        page.getByRole("heading", {
          name: "Your X (Twitter) channel bot is created",
        }),
      ).toBeVisible();
      expect(starts).toBe(1);
      expect(completions).toBe(1);
    } else {
      const reopened = context.waitForEvent("page");
      await retry.click();
      const nextPopup = await reopened;
      await expect(
        nextPopup.getByRole("heading", { name: "Provider authorization" }),
      ).toBeVisible();
      expect(starts).toBe(2);
      await nextPopup.close();
      await expect(retry).toBeEnabled();
    }
    await expect(page.getByRole("button", { name: /Cancel/ })).toHaveCount(0);
  });
}

for (const coexistence of [false, true]) {
  test(`standalone WhatsApp completes ${coexistence ? "Business App" : "Cloud API"} signup`, async ({
    page,
  }) => {
    await mockDashboard(page);
    await page.addInitScript(() => {
      window.FB = {
        init: () => {},
        login: (callback, options) => {
          sessionStorage.setItem("setup-meta-options", JSON.stringify(options));
          callback({ authResponse: { code: "one-use-code" } });
          window.dispatchEvent(
            new MessageEvent("message", {
              origin: "https://www.facebook.com",
              data: JSON.stringify({
                type: "WA_EMBEDDED_SIGNUP",
                event: options.extras.featureType
                  ? "FINISH_WHATSAPP_BUSINESS_APP_ONBOARDING"
                  : "FINISH",
                data: { phone_number_id: "333", waba_id: "444" },
              }),
            }),
          );
        },
      };
    });
    await page.route("**/channel-bots/managed-onboarding/whatsapp", (route) =>
      route.fulfill({ json: managedBootstrap }),
    );
    const completions: unknown[] = [];
    await page.route(
      "**/channel-bots/managed-onboarding/whatsapp/complete",
      (route) => {
        completions.push(route.request().postDataJSON());
        return route.fulfill({
          contentType: "text/event-stream",
          body: `data: ${JSON.stringify({ result: managedBot })}\n\n`,
        });
      },
    );
    await page.goto("/channel-bots/connect/whatsapp?label=WA%20Support");
    await expect(
      page.getByRole("button", { name: "Connect with Meta" }),
    ).toBeEnabled();
    await expect(page.getByRole("button", { name: /Cancel/ })).toHaveCount(0);
    await expect(page.getByLabel("Access token", { exact: true })).toHaveCount(
      0,
    );
    if (coexistence)
      await page
        .getByLabel("I already use the WhatsApp Business app on this number")
        .check();
    expect(completions).toEqual([]);
    await page.getByRole("button", { name: "Connect with Meta" }).click();
    await expect(
      page.getByRole("heading", {
        name: "Your WhatsApp channel bot is created",
      }),
    ).toBeVisible();
    await expect(page).toHaveURL(/\/channel-bots\/connect\/whatsapp/);
    expect(completions).toEqual([
      {
        code: "one-use-code",
        phone_number_id: "333",
        waba_id: "444",
        label: "WA Support",
      },
    ]);
    expect(
      await page.evaluate(
        () =>
          JSON.parse(sessionStorage.getItem("setup-meta-options") ?? "{}")
            .extras,
      ),
    ).toEqual(
      coexistence ? { featureType: "whatsapp_business_app_onboarding" } : {},
    );
    await expect(
      page.getByRole("button", { name: "Open channel bot" }),
    ).toBeVisible();
  });
}

for (const width of [390, 1440]) {
  test(`channel setup links and query prefills work at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 950 });
    await mockDashboard(page);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/channel-bots/connect");
    await expect(
      page.getByRole("heading", { name: "Channel setup links" }),
    ).toBeVisible();
    await page.getByRole("link", { name: "Set up Lark", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Create your Lark channel bot" }),
    ).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByRole("complementary")).toHaveCount(0);
    await expect(
      page.getByRole("img", { name: "NyxID connects to Lark" }),
    ).toBeVisible();
    await expect(
      page.getByRole("list", { name: "Connection progress" }),
    ).toHaveCount(0);
    await expect(page.getByRole("button", { name: /^Cancel/ })).toHaveCount(0);
    await expect(
      page.getByRole("combobox", { name: "Create for" }),
    ).toHaveCount(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `/tmp/nyxid-channel-setup-${width}.png`,
      fullPage: true,
    });
    const writes: unknown[] = [];
    await page.route("**/api/v1/channel-bots", (route) => {
      writes.push(route.request().postDataJSON());
      return route.fulfill({
        json: {
          ...managedBot,
          id: "prefilled-bot",
          platform: "lark",
          webhook_secret: "one-time-secret",
          webhook_secret_label: "Verify Token",
        },
      });
    });
    await page.goto(
      "/channel-bots/connect/lark?label=Support&app_id=90071992547409931234&app_secret=fixture-secret&verification_token=fixture-verify",
    );
    await expect(page.getByLabel("App ID", { exact: true })).toHaveValue(
      "90071992547409931234",
    );
    await expect(page.getByLabel("App Secret", { exact: true })).toHaveValue(
      "fixture-secret",
    );
    await expect(page).not.toHaveURL(/app_secret|verification_token/);
    expect(writes).toEqual([]);
    await expect(
      page.getByRole("button", { name: "Create channel bot", exact: true }),
    ).toBeEnabled();
    await page
      .getByRole("button", { name: "Create channel bot", exact: true })
      .click();
    await expect(
      page.getByText("one-time-secret", { exact: true }),
    ).toBeVisible();
    expect(writes).toEqual([
      {
        platform: "lark",
        label: "Support",
        app_id: "90071992547409931234",
        app_secret: "fixture-secret",
        verification_token: "fixture-verify",
      },
    ]);
    expect(errors).toEqual([]);
  });

  test(`managed Telegram opens with one click at ${width}px`, async ({
    page,
    context,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: "dark" });
    await mockDashboard(page);
    await page.route("**/api/v1/channel-platforms", (route) =>
      route.fulfill({
        json: {
          platforms: channelPlatforms.map((platform) => ({
            ...platform,
            registration: {
              ...platform.registration,
              documentation_url:
                "https://core.telegram.org/api/bots/managed-bots",
            },
          })),
        },
      }),
    );
    await context.route("https://t.me/**", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: "Telegram setup fixture",
      }),
    );
    let saved: TelegramNewRequest | null = null;
    const writes: unknown[] = [];
    await page.route("**/api/v1/channel-bots/telegram-new**", (route) => {
      if (route.request().method() === "GET")
        return route.fulfill({
          json: {
            available: true,
            manager_username: "NyxSetupBot",
            request: saved,
          },
        });
      writes.push(route.request().postDataJSON());
      saved = {
        id: "3c638c7f-210a-44fc-9b67-f6c878d67c54",
        status: "waiting_telegram",
        revision: 1,
        owner_user_id: "test-user",
        label: "Telegram bot",
        expires_at: "2099-09-16T10:00:00Z",
        telegram_bot_id: null,
        bot_username: null,
        channel_bot_id: null,
        auto_connect: true,
      };
      return route.fulfill({
        json: {
          request: saved,
          launch_url: "https://t.me/NyxSetupBot?start=fixture",
        },
      });
    });
    await page.goto("/channel-bots/connect/telegram-new");
    const button = page.getByRole("button", { name: "Continue in Telegram" });
    await expect(button).toBeEnabled();
    await expect(page.getByLabel("Bot name")).toHaveValue("Telegram bot");
    await expect(
      page.getByRole("combobox", { name: "Create for" }),
    ).toHaveCount(0);
    await expect(page.getByRole("button")).toHaveCount(1);
    await expect(page.getByRole("list")).toHaveCount(0);
    await expect(
      page
        .getByRole("banner")
        .getByRole("link", { name: "Channel setup guide" }),
    ).toBeVisible();
    await page.screenshot({
      path: `/tmp/nyxid-channel-one-click-${width}.png`,
      fullPage: true,
    });
    expect(writes).toEqual([]);
    const popup = context.waitForEvent("page");
    await button.click();
    const telegram = await popup;
    await expect(telegram).toHaveURL("https://t.me/NyxSetupBot?start=fixture");
    expect(writes).toEqual([{ label: "Telegram bot", auto_connect: true }]);
    await telegram.close();
    await expect(
      page.getByRole("button", { name: "Reopen Telegram" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: /Cancel/ })).toHaveCount(0);
    await expect(page).toHaveURL(/request_id=/);
  });
}

test("a shared setup link resumes after sign-in for a user who has not completed AI Services onboarding", async ({
  page,
}) => {
  await mockDashboard(page);
  await page.route("**/api/v1/public/config", (route) =>
    route.fulfill({
      json: {
        social_providers: [],
        invite_required: false,
        registration_enabled: true,
        email_auth_enabled: true,
      },
    }),
  );
  let signedIn = false;
  await page.route("**/api/v1/users/me", (route) =>
    route.fulfill(
      signedIn
        ? {
            json: {
              id: "test-user",
              email: "test@example.com",
              display_name: "Test User",
              is_admin: true,
              is_active: true,
              email_verified: true,
              created_at: managedBot.created_at,
              profile_config: {
                onboarding: { ai_services_completed_at: null },
              },
            },
          }
        : {
            status: 401,
            json: { message: "Sign in required", error_code: 1001 },
          },
    ),
  );
  await page.route("**/api/v1/auth/refresh", (route) =>
    route.fulfill({
      status: 401,
      json: { message: "Sign in required", error_code: 1001 },
    }),
  );
  await page.route("**/api/v1/auth/login", (route) => {
    signedIn = true;
    return route.fulfill({ json: { message: "Signed in" } });
  });
  await page.goto(
    "/channel-bots/connect/discord?label=Support&bot_token=fixture-token&public_key=fixture-key",
  );
  await expect(page).toHaveURL(/\/login\?/);
  const returnTo = new URL(page.url()).searchParams.get("return_to");
  expect(returnTo).toContain("/channel-bots/connect/discord?label=Support");
  await page.getByPlaceholder("you@example.com").fill("test@example.com");
  await page.getByPlaceholder("Enter your password").fill("TestPassword123!");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Create your Discord channel bot" }),
  ).toBeVisible();
  await expect(page.getByLabel("Bot name", { exact: true })).toHaveValue(
    "Support",
  );
  await expect(page.getByLabel("Bot token", { exact: true })).toHaveValue(
    "fixture-token",
  );
  await expect(page).not.toHaveURL(/bot_token/);
  await page.route("**/api/v1/channel-bots", (route) =>
    route.fulfill({ json: managedBot }),
  );
  await page
    .getByRole("button", { name: "Create channel bot", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Your Discord channel bot is created" }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/channel-bots\/connect\/discord/);
  await page.getByRole("button", { name: "Open channel bot" }).click();
  await expect(page).toHaveURL(/\/channel-bots\/managed-whatsapp$/);
  await expect(
    page.getByRole("heading", { name: "Managed Support", exact: true }),
  ).toBeVisible();
});
