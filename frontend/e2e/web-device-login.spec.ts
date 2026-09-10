import { expect, test, type Page } from "@playwright/test";

const userCode = "ABCD-EFGH";

async function fixture(page: Page, delay = false) {
  let release: (() => void) | null = null;
  let polled = false;
  let first = true;
  let signedIn = false;
  const requests: string[] = [];
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    requests.push(path);
    let status = 200;
    let json: unknown = {};
    if (path === "/api/v1/users/me") {
      status = signedIn ? 200 : 401;
      json = signedIn
        ? { id: "user-1", email: "user@example.test", email_verified: true, role: "user" }
        : { error_code: 1001, message: "Not signed in" };
    } else if (path === "/api/v1/auth/device/request") {
      json = {
        device_code: "nyx_adc_fixture-private-poller",
        user_code: userCode,
        verification_uri: "https://nyxid.example/login/device",
        verification_uri_complete: `https://nyxid.example/login/device?user_code=${userCode}`,
        interval: 5,
        expires_in: 600,
      };
    } else if (path === "/api/v1/auth/device/poll-web" && first) {
      first = false;
      polled = true;
      if (delay)
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      signedIn = true;
      json = { ok: true, auth_kind: "account_session" };
    } else if (path === "/api/v1/auth/device/poll-web") {
      status = 400;
      json = { error_code: 11202, message: "Pending" };
    } else if (path === "/api/v1/public/config") {
      json = {
        social_providers: [],
        telemetry_dsn: null,
        telemetry_share_analytics: false,
      };
    } else {
      status = 404;
      json = { message: "Not found" };
    }
    await route.fulfill({ status, json });
  });
  return { polled: () => polled, release: () => release?.(), requests };
}

test("browser QR sign-in uses the account-only device protocol", async ({
  page,
}) => {
  const state = await fixture(page);
  await page.goto("/login");
  await page
    .getByRole("button", { name: "Continue with the NyxID app", exact: true })
    .click();
  await expect(page.getByText(userCode, { exact: true })).toBeVisible({
    timeout: 10000,
  });
  await expect(page).toHaveURL(/\/dashboard/, { timeout: 10000 });
  expect(state.requests).toContain("/api/v1/auth/device/request");
  expect(state.requests).toContain("/api/v1/auth/device/poll-web");
  expect(state.requests.some((path) => path.includes("/auth/device/v2/"))).toBe(false);
});

test("closing and reopening ignores the prior delivery response", async ({
  page,
}) => {
  const state = await fixture(page, true);
  await page.goto("/login");
  await page
    .getByRole("button", { name: "Continue with the NyxID app", exact: true })
    .click();
  await expect.poll(state.polled, { timeout: 10000 }).toBe(true);
  await page
    .getByRole("button", { name: "Back to all sign-in options", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Continue with the NyxID app", exact: true })
    .click();
  await expect(page.getByText(userCode, { exact: true })).toBeVisible();
  state.release();
  await page.waitForTimeout(300);
  expect(page.url()).toContain("/login");
  await expect(page.getByText(userCode, { exact: true })).toBeVisible();
});
