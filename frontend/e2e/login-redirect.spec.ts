import { expect, test, type Page } from "@playwright/test";

const conversation = "nyxa-4c9f28dd584b44a79bb4f452528d814a";

async function fixture(page: Page, passwordMfa: boolean) {
  let delivered = false;
  const requests: string[] = [];
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    requests.push(path);
    let status = 200;
    let json: unknown;
    if (path === "/api/v1/users/me") {
      status = delivered ? 200 : 401;
      json = delivered
        ? {
            id: "app-person",
            email: "app@example.com",
            display_name: "App person",
            is_active: true,
            role: "user",
            email_verified: true,
            mfa_enabled: true,
          }
        : { error_code: 1001, message: "Not signed in" };
    } else if (path === "/api/v1/public/config") {
      json = {
        email_auth_enabled: true,
        social_providers: [],
        telemetry_dsn: null,
        telemetry_share_analytics: false,
      };
    } else if (path === "/api/v1/auth/login" && passwordMfa) {
      status = 401;
      json = {
        error: "mfa_required",
        error_code: 2002,
        message: "MFA required",
        session_token: "pending-password-mfa",
      };
    } else if (path === "/api/v1/auth/device/request") {
      const origin = new URL(route.request().url()).origin;
      json = {
        device_code: "nyx_adc2_fixture-private-poller",
        user_code: "ABCD-EFGH",
        verification_uri: `${origin}/login/device`,
        verification_uri_complete: `${origin}/login/device?user_code=ABCD-EFGH`,
        interval: 1,
        expires_in: 600,
      };
    } else if (path === "/api/v1/auth/device/poll-web") {
      delivered = true;
      json = { ok: true, auth_kind: "account_session" };
    } else {
      status = 404;
      json = { message: "Not found" };
    }
    await route.fulfill({ status, json });
  });
  return requests;
}

for (const passwordMfa of [false, true]) {
  test(`app login returns to the exact assistant conversation from ${passwordMfa ? "password MFA" : "sign-in options"}`, async ({
    page,
    baseURL,
  }) => {
    const requests = await fixture(page, passwordMfa);
    const target = new URL(`/assistant?c=${conversation}`, baseURL).href;
    await page.goto(`/login?return_to=${encodeURIComponent(target)}`);
    if (passwordMfa) {
      await page.getByPlaceholder("you@example.com").fill("app@example.com");
      await page.getByPlaceholder("Enter your password").fill("correct horse");
      await page.getByRole("button", { name: "Sign in", exact: true }).click();
      await expect(
        page.getByRole("heading", { name: "MFA verification" }),
      ).toBeVisible();
    }
    await page
      .getByRole("button", { name: "Continue with the NyxID app", exact: true })
      .click();
    await expect(page.getByText("ABCD-EFGH", { exact: true })).toBeVisible();
    await expect(page).toHaveURL(
      (url) =>
        url.pathname === "/assistant" &&
        url.searchParams.get("c") === conversation,
      { timeout: 10_000 },
    );
    await page.waitForLoadState("domcontentloaded");
    await expect(
      page.getByRole("button", { name: "Account menu" }),
    ).toBeVisible();
    expect(new URL(page.url()).pathname).toBe("/assistant");
    expect(new URL(page.url()).searchParams.get("c")).toBe(conversation);
    expect(
      requests.filter((path) => path.endsWith("/device/request")),
    ).toHaveLength(1);
    expect(
      requests.filter((path) => path.endsWith("/device/poll-web")),
    ).toHaveLength(1);
    expect(requests).not.toContain("/api/v1/auth/mfa/verify");
  });
}
