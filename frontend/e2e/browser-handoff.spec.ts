import { expect, test, type Page } from "@playwright/test";

const code = "JKLM-NPQR";

async function fixture(page: Page, delay = false) {
  let release: (() => void) | null = null;
  let polled = false;
  let first = true;
  let expiry = "";
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    let status = 200;
    let json: unknown = {};
    if (path === "/api/v1/users/me") {
      status = 401;
      json = { error_code: 1001, message: "Not signed in" };
    } else if (path === "/api/v1/auth/device/v2/request") {
      json = {
        device_code: "nyx_adc2_fixture-private-poller",
        user_code: "2-ABCD-EFGH",
        verification_uri: "https://nyxid.example/login/device",
        verification_uri_complete:
          "https://nyxid.example/login/device?user_code=2-ABCD-EFGH",
        interval: 5,
        expires_in: 600,
      };
    } else if (path === "/api/v1/auth/device/v2/poll-web" && first) {
      first = false;
      polled = true;
      if (delay)
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      expiry = new Date(Date.now() + 2000).toISOString();
      json = {
        ok: false,
        auth_kind: "agent_key",
        login_code: { request_id: "handoff-fixture", code, expires_at: expiry },
      };
    } else if (path === "/api/v1/auth/device/v2/poll-web") {
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
  return { polled: () => polled, release: () => release?.() };
}

test("restricted browser handoff stays signed out and clears its expired code", async ({
  page,
  context,
}) => {
  await fixture(page);
  await page.goto("/login");
  await page
    .getByRole("button", { name: "Continue with the NyxID app", exact: true })
    .click();
  await expect(page.getByText(code, { exact: true })).toBeVisible({
    timeout: 10000,
  });
  expect(page.url()).toContain("/login");
  expect(await context.cookies()).toEqual([]);
  expect(
    await page.evaluate(() => JSON.stringify({ localStorage, sessionStorage })),
  ).not.toContain(code);
  await expect(page.getByText(code, { exact: true })).toHaveCount(0, {
    timeout: 5000,
  });
  await expect(
    page.getByRole("button", { name: "Generate new code", exact: true }),
  ).toBeVisible();
});

test("closing and reopening ignores the prior handoff response", async ({
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
  await expect(page.getByText("2-ABCD-EFGH", { exact: true })).toBeVisible();
  state.release();
  await page.waitForTimeout(300);
  await expect(page.getByText(code, { exact: true })).toHaveCount(0);
  await expect(page.getByText("2-ABCD-EFGH", { exact: true })).toBeVisible();
});
