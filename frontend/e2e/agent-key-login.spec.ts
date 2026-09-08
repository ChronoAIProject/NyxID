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

async function fixture(page: Page, signedIn: boolean) {
  const requests: { path: string; body: unknown }[] = [];
  const state = { status: "pending" };
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
    if (path.startsWith("/api/v1/auth/agent-key/"))
      requests.push({ path, body: route.request().postDataJSON() });
    let status = 200;
    let body: unknown = {};
    if (path === "/api/v1/users/me") {
      status = signedIn ? 200 : 401;
      body = signedIn
        ? {
            id: "user",
            display_name: "Human",
            email: "human@example.com",
            is_active: true,
            role: "user",
            email_verified: true,
          }
        : { error: "unauthorized", error_code: 1001, message: "Not signed in" };
    } else if (path.endsWith("/agent-key/preview"))
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
        status: state.status,
        initiated_at: new Date().toISOString(),
        expires_at: new Date(Date.now() + 600000).toISOString(),
        seconds_remaining: 600,
        interval: 5,
      };
    else if (path.endsWith("/agent-key/options"))
      body = {
        keys: [key],
        services: key.allowed_services,
        nodes: [],
        orgs: [],
      };
    else if (
      path.endsWith("/agent-key/approve") ||
      path.endsWith("/agent-key/deny")
    )
      body = { ok: true };
    else if (path === "/api/v1/auth/logout") body = { ok: true };
    else if (path === "/api/v1/public/config")
      body = { telemetry_dsn: null, telemetry_share_analytics: false };
    else {
      status = 404;
      body = { message: "Not found" };
    }
    await route.fulfill({ status, json: body });
  });
  return { requests, state };
}

test("phone QR is public, ignores URL codes and leaves no browser storage or account cookie", async ({
  page,
  context,
}, info) => {
  const { requests, state } = await fixture(page, false);
  await page.goto("/login/agent-key?user_code=ABCD-EFGH");
  await expect(page.getByLabel("User code")).toHaveValue("");
  await page.getByLabel("User code").fill("ABCD-EFGH");
  expect(requests).toHaveLength(0);
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(
    page.getByText("Requested profile: home-agent", { exact: true }),
  ).toBeVisible();
  await page.waitForTimeout(800);
  await page
    .getByRole("button", { name: "Approve from your phone", exact: true })
    .click();
  await expect(page.getByAltText("Agent Key login QR code")).toBeVisible();
  await page.screenshot({
    path: info.outputPath("phone-qr-desktop.png"),
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: info.outputPath("phone-qr-mobile.png"),
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  state.status = "approved";
  await expect(
    page.getByText("Approved - return to the requesting device"),
  ).toBeVisible({ timeout: 10000 });
  await expect(page.getByAltText("Agent Key login QR code")).toHaveCount(0);
  const count = requests.length;
  await page.waitForTimeout(5500);
  expect(requests).toHaveLength(count);
  expect(requests.every((request) => request.path.endsWith("/preview"))).toBe(
    true,
  );
  expect(await context.cookies()).toEqual([]);
  expect(
    await page.evaluate(() => ({
      local: localStorage.length,
      session: sessionStorage.length,
      writes: (window as unknown as { storageWrites: string[] }).storageWrites,
    })),
  ).toEqual({ local: 0, session: 0, writes: [] });
});

for (const selection of ["existing", "new"] as const)
  test(`${selection} key requires final permission confirmation`, async ({
    page,
  }, info) => {
    const { requests } = await fixture(page, true);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/login/agent-key");
    await page.getByLabel("User code").fill("ABCD-EFGH");
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    await expect(
      page.getByText("Requested profile: home-agent", { exact: true }),
    ).toBeVisible();
    await page.waitForTimeout(800);
    await page
      .getByRole("button", { name: "Approve on this computer", exact: true })
      .click();
    await page
      .getByRole("radio", {
        name: selection === "existing" ? "Shared agent" : "Create a new key",
        exact: true,
      })
      .check();
    await page.waitForTimeout(800);
    await page
      .getByRole("button", { name: "Review permissions", exact: true })
      .click();
    await expect(page.getByText("Confirm effective permissions")).toBeVisible();
    expect(requests.some((request) => request.path.endsWith("/approve"))).toBe(
      false,
    );
    await page.screenshot({
      path: info.outputPath(`${selection}-confirm-mobile.png`),
      fullPage: true,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await page.waitForTimeout(800);
    await page.getByRole("button", { name: "Approve", exact: true }).click();
    await expect(
      page.getByText("Approved - return to the requesting device"),
    ).toBeVisible();
    const approvals = requests.filter((request) =>
      request.path.endsWith("/approve"),
    );
    expect(approvals).toHaveLength(1);
    expect(approvals[0]?.body).toMatchObject({
      user_code: "ABCDEFGH",
      selection: {
        kind: selection,
        ...(selection === "new"
          ? {
              allow_all_services: false,
              allow_all_nodes: false,
              scopes: "read proxy",
            }
          : { api_key_id: "key" }),
      },
    });
  });
