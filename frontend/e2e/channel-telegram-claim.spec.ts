import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import type { TelegramNewRequest } from "../src/schemas/telegram-new";
const code = "ABCDE-FGHJK-LMNPQ-RSTUV";
const id = "3c638c7f-210a-44fc-9b67-f6c878d67c54";
const org = "cf8812f3-ff2a-46a9-8c6b-63f7bc1f5d65";
const entry = `/channel-bots?connect=telegram-new&claim=${code}`;

async function mockClaim(page: Page) {
  let request: TelegramNewRequest | null = null;
  const previews: unknown[] = [];
  const redeems: unknown[] = [];
  await page.route("**/api/v1/orgs", (route) =>
    route.fulfill({
      json: {
        orgs: [{ id: org, display_name: "Support team", your_role: "admin" }],
      },
    }),
  );
  await page.route("**/api/v1/channel-bots/telegram-new**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith("/claims/preview")) {
      previews.push(route.request().postDataJSON());
      await route.fulfill({
        json: {
          bot_username: "CustomerBot",
          expires_at: new Date(Date.now() + 15 * 60_000).toISOString(),
        },
      });
    } else if (url.pathname.endsWith("/claims/redeem")) {
      const body = route.request().postDataJSON();
      redeems.push(body);
      request = {
        id,
        label: body.label,
        owner_user_id: body.target_org_id ?? "test-user",
        status: "ready",
        revision: 0,
        auto_connect: true,
        telegram_bot_id: "900",
        bot_username: "CustomerBot",
        channel_bot_id: null,
        expires_at: new Date(Date.now() + 15 * 60_000).toISOString(),
      };
      await route.fulfill({ status: 202, json: request });
    } else if (route.request().method() === "GET") {
      await route.fulfill({
        json: { available: true, manager_username: "ManagerBot", request },
      });
    } else {
      throw new Error(
        "Claim screen must not initiate creation or use legacy connect",
      );
    }
  });
  return { previews, redeems };
}

for (const width of [390, 1440]) {
  test(`Telegram-first code previews then connects once at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 950 });
    await mockDashboard(page);
    const calls = await mockClaim(page);
    const referrers: string[] = [];
    page.on("request", (request) =>
      referrers.push(request.headers()["referer"] ?? ""),
    );
    await page.goto(entry);
    await expect(
      page.getByRole("heading", { name: "Connect @CustomerBot" }),
    ).toBeVisible();
    expect(page.url()).not.toContain(code);
    expect(new URL(page.url()).searchParams.has("claim")).toBe(false);
    expect(calls.previews).toEqual([{ code }]);
    expect(calls.redeems).toEqual([]);
    expect(
      await page.evaluate(() =>
        JSON.stringify({ localStorage, sessionStorage, state: history.state }),
      ),
    ).not.toContain(code);
    expect(referrers.some((referrer) => referrer.includes(code))).toBe(false);
    await page.getByLabel("Bot label in NyxID").fill("Customer support");
    await page.getByRole("combobox", { name: "Connect to" }).click();
    await page.getByRole("option", { name: "Support team" }).click();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page
      .getByRole("region", { name: "Connect Telegram bot" })
      .screenshot({ path: `/tmp/nyxbot-claim-setup-${width}.png` });
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`request_id=${id}`));
    await expect(page.getByText("Connecting @CustomerBot…")).toBeVisible();
    expect(calls.redeems).toEqual([
      { code, label: "Customer support", target_org_id: org },
    ]);
    expect(
      await page.evaluate(() =>
        JSON.stringify({ localStorage, sessionStorage }),
      ),
    ).not.toContain(code);
  });
}

test("Telegram-first claim survives a real sign-in redirect without appearing in return_to", async ({
  page,
}) => {
  await mockDashboard(page);
  const calls = await mockClaim(page);
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
              created_at: "2026-09-08T00:00:00Z",
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
  await page.goto(entry);
  await expect(page).toHaveURL(/\/login\?/);
  await expect(
    page.getByRole("heading", { name: "Welcome back" }),
  ).toBeVisible();
  const returnTo = new URL(page.url()).searchParams.get("return_to");
  expect(returnTo).toContain("claim_entry=true");
  expect(returnTo).not.toContain(code);
  expect(await page.evaluate(() => JSON.stringify(sessionStorage))).toContain(
    code,
  );
  expect(await page.evaluate(() => JSON.stringify(localStorage))).not.toContain(
    code,
  );
  expect(calls.previews).toEqual([]);
  await page.getByPlaceholder("you@example.com").fill("test@example.com");
  await page.getByPlaceholder("Enter your password").fill("TestPassword123!");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Connect @CustomerBot" }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => JSON.stringify({ localStorage, sessionStorage })),
  ).not.toContain(code);
  expect(calls.previews).toEqual([{ code }]);
  expect(calls.redeems).toEqual([]);
  await page.getByRole("button", { name: "Back to Channel Bots" }).click();
  await page.goBack();
  expect(page.url()).not.toContain(code);
});
