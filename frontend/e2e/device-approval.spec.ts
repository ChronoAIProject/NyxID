import { expect, test, type Page } from "@playwright/test";

const key = {
  id: "key",
  name: "Shared agent",
  key_prefix: "nyxid_ag_12345678",
  owner_type: "personal",
  owner_id: "user",
  owner_name: "Human",
  scopes: "read proxy",
  allow_all_services: false,
  allow_all_nodes: false,
  allowed_service_ids: ["service"],
  allowed_node_ids: [],
  allowed_services: [
    { id: "service", name: "OpenAI workspace", owner_id: "user" },
  ],
  allowed_nodes: [],
  expires_at: null,
  rate_limit_per_second: 10,
  rate_limit_burst: 20,
  platform: "codex",
  created_now: false,
};

async function fixture(page: Page, authenticated: boolean) {
  const requests: { path: string; body: unknown }[] = [];
  await page.addInitScript(() => {
    const writes: string[] = [];
    (window as unknown as { storageWrites: string[] }).storageWrites = writes;
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      writes.push(key);
      return original.call(this, key, value);
    };
  });
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    let status = 200;
    let body: unknown = {};
    if (path.startsWith("/api/v1/auth/device/")) {
      requests.push({ path, body: route.request().postDataJSON() });
    }
    if (path === "/api/v1/users/me") {
      status = authenticated ? 200 : 401;
      body = authenticated
        ? {
            id: "user",
            display_name: "Human",
            email: "human@example.com",
            is_active: true,
            role: "user",
            email_verified: true,
          }
        : { error: "unauthorized", error_code: 1001, message: "Not signed in" };
    } else if (path === "/api/v1/auth/device/preview") {
      body = {
        client_label: "workstation",
        client_ip: "203.0.113.10",
        client_ip_attribution: "verified",
        client_country: "SG",
        client_city: "Singapore",
        client_app: "NyxID CLI",
        client_platform: "macOS",
        client_kind: "cli",
        requested_profile: "home-agent",
        status: "pending",
        initiated_at: new Date().toISOString(),
        expires_at: new Date(Date.now() + 600000).toISOString(),
        seconds_remaining: 600,
        interval: 5,
      };
    } else if (path === "/api/v1/auth/device/options") {
      body = {
        keys: [key],
        services: key.allowed_services,
        nodes: [],
        orgs: [],
      };
    } else if (
      [
        "/api/v1/auth/device/approve",
        "/api/v1/auth/device/approve-agent-key",
        "/api/v1/auth/device/deny",
      ].includes(path)
    ) {
      body = { ok: true };
    } else if (path === "/api/v1/public/config") {
      body = { telemetry_dsn: null, telemetry_share_analytics: false };
    } else {
      status = 404;
      body = { message: "Not found" };
    }
    await route.fulfill({ status, json: body });
  });
  return requests;
}

test("public device preview makes no account or storage changes", async ({
  page,
  context,
}) => {
  const requests = await fixture(page, false);
  await page.goto("/login/device");
  await expect(page.getByLabel("User code")).toBeVisible();
  await page.getByLabel("User code").fill("ABCD-EFGH");
  expect(requests).toEqual([]);
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(
    page.getByText("Requested profile: home-agent", { exact: true }),
  ).toBeVisible();
  expect(requests.map((request) => request.path)).toEqual([
    "/api/v1/auth/device/preview",
  ]);
  expect(await context.cookies()).toEqual([]);
  expect(
    await page.evaluate(() => ({
      local: localStorage.length,
      session: sessionStorage.length,
      writes: (window as unknown as { storageWrites: string[] }).storageWrites,
    })),
  ).toEqual({ local: 0, session: 0, writes: [] });
});

for (const grant of ["account", "agent-key"] as const) {
  test(`device request approves ${grant} only after confirmation`, async ({
    page,
  }, info) => {
    const requests = await fixture(page, true);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/login/device?user_code=2abcd%20efgh");
    await expect(page.getByLabel("User code")).toHaveValue("2-ABCD-EFGH");
    await expect(page.getByLabel("User code")).toHaveAttribute("placeholder", "2-XXXX-XXXX");
    await expect(page).toHaveURL(/\/login\/device$/);
    expect(requests).toEqual([]);
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Full account session", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Restricted Agent Key", exact: true }),
    ).toBeVisible();
    await expect(page.getByText("2-ABCD-EFGH", { exact: true })).toBeVisible();
    await page.waitForTimeout(800);
    if (grant === "account") {
      await page
        .getByRole("button", { name: "Full account session", exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "Confirm full account access" }),
      ).toBeVisible();
    } else {
      await page
        .getByRole("button", { name: "Restricted Agent Key", exact: true })
        .click();
      await page.getByRole("radio", { name: "Shared agent" }).check();
      await page.waitForTimeout(800);
      await page.getByRole("button", { name: "Review permissions" }).click();
      await expect(
        page.getByRole("heading", { name: "Confirm effective permissions" }),
      ).toBeVisible();
    }
    await expect(page.getByText("2-ABCD-EFGH", { exact: true })).toBeVisible();
    await expect(page.getByText(/Reject if it does not match/)).toBeVisible();
    expect(
      requests.filter((request) => request.path.includes("/approve")),
    ).toEqual([]);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: info.outputPath(`${grant}-mobile.png`),
      fullPage: true,
    });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.screenshot({
      path: info.outputPath(`${grant}-desktop.png`),
      fullPage: true,
    });
    await page.waitForTimeout(800);
    await page
      .getByRole("button", {
        name: grant === "account" ? "Approve full account session" : "Approve",
        exact: true,
      })
      .click();
    await expect(
      page.getByText("Approved - return to the requesting device", {
        exact: true,
      }),
    ).toBeVisible();
    expect(
      requests.filter((request) => request.path.includes("/approve")),
    ).toEqual([
      {
        path:
          grant === "account"
            ? "/api/v1/auth/device/approve"
            : "/api/v1/auth/device/approve-agent-key",
        body:
          grant === "account"
            ? { user_code: "2ABCDEFGH" }
            : {
                user_code: "2ABCDEFGH",
                selection: { kind: "existing", api_key_id: "key" },
              },
      },
    ]);
  });
}

test("legacy device requests offer only account access", async ({ page }) => {
  const requests = await fixture(page, true);
  await page.goto("/login/device");
  await page.getByLabel("User code").fill("ABCD-EFGH");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Full account session", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Restricted Agent Key", exact: true }),
  ).toHaveCount(0);
  expect(
    requests.filter((request) => request.path.includes("/approve")),
  ).toEqual([]);
});
