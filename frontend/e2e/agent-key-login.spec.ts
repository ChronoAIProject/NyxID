import { expect, test, type Page } from "@playwright/test";
import { loginInventory } from "../src/lib/__fixtures__/login-inventory";

async function fixture(page: Page, signedIn: boolean) {
  const requests: { path: string; body: unknown }[] = [];
  const inventory = loginInventory();
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    let status = 200;
    let body: unknown;
    if (path.startsWith("/api/v1/auth/agent-key/"))
      requests.push({ path, body: route.request().postDataJSON() });
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
    } else if (path.endsWith("/agent-key/preview")) {
      body = {
        client_label: "workstation",
        client_ip: "203.0.113.10",
        client_ip_attribution: "verified",
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
    } else if (path.endsWith("/agent-key/options")) body = inventory.options;
    else if (path === "/api/v1/keys") body = { keys: inventory.connections };
    else if (path === "/api/v1/catalog") body = { entries: inventory.catalog };
    else if (
      path.endsWith("/agent-key/approve") ||
      path.endsWith("/agent-key/deny")
    )
      body = { ok: true };
    else if (path === "/api/v1/public/config")
      body = {
        telemetry_dsn: null,
        telemetry_share_analytics: false,
        email_auth_enabled: false,
        social_providers: ["google", "github", "apple"],
      };
    else {
      status = 404;
      body = { message: "Not found" };
    }
    await route.fulfill({ status, json: body });
  });
  return requests;
}

test("an agent login URL shows its request and all production login methods without approving", async ({
  page,
  context,
}, info) => {
  const requests = await fixture(page, false);
  await page.goto(
    "/login/agent-key?user_code=ABCD-EFGH&permissions=read,proxy&key_name=My+agent",
  );
  await expect(page.getByText("ABCD-EFGH", { exact: true })).toBeVisible();
  for (const name of [
    "Continue with Google",
    "Continue with GitHub",
    "Continue with Apple",
    "Continue with the NyxID app",
  ])
    await expect(page.getByRole("button", { name, exact: true })).toBeVisible();
  await expect(page.getByLabel("Authenticator code")).toHaveCount(0);
  await page.getByText("Request details", { exact: true }).click();
  await expect(page.getByText("home-agent", { exact: true })).toBeVisible();
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 900 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: info.outputPath(`agent-login-${width}.png`),
      fullPage: true,
    });
  }
  expect(requests.every((request) => request.path.endsWith("/preview"))).toBe(
    true,
  );
  expect(await context.cookies()).toEqual([]);
  expect(
    await page.evaluate(() => ({
      local: localStorage.length,
      session: sessionStorage.length,
    })),
  ).toEqual({ local: 0, session: 0 });
  expect(new URL(page.url()).searchParams.get("key_name")).toBe("My agent");
});

for (const selection of ["existing", "new"] as const) {
  test(`${selection} Agent Key requires explicit final consent`, async ({
    page,
  }, info) => {
    const requests = await fixture(page, true);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(
      "/login/agent-key?user_code=abcd%20efgh&permissions=read,proxy&service_permissions=github::repo:read",
    );
    await expect(page.getByText("ABCD-EFGH", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    await expect(
      page.getByRole("radio", { name: /Full account access/ }),
    ).toHaveCount(0);
    await page.getByRole("button", { name: "Continue to approval" }).click();
    if (selection === "existing")
      await page.getByRole("button", { name: "Reader", exact: true }).click();
    else {
      await page
        .getByRole("button", { name: "Create new Agent Key", exact: true })
        .click();
      await page.getByText("Customize", { exact: true }).click();
      await page.getByLabel("Name", { exact: true }).fill("CLI Agent");
      await page.getByText("Customize", { exact: true }).click();
    }
    const review = page.getByRole("region", { name: "Authorize access" });
    await expect(review).toBeVisible();
    expect(requests.some((request) => request.path.endsWith("/approve"))).toBe(
      false,
    );
    await page.screenshot({
      path: info.outputPath(`${selection}-confirm-mobile.png`),
      fullPage: true,
    });
    await page
      .getByRole("button", {
        name:
          selection === "existing"
            ? "Approve access with this key"
            : "Create & continue",
        exact: true,
      })
      .click();
    await expect(
      page.getByText("Approved — return to the requesting device"),
    ).toBeVisible();
    const approvals = requests.filter((request) =>
      request.path.endsWith("/approve"),
    );
    expect(approvals).toHaveLength(1);
    expect(approvals[0]?.body).toMatchObject({
      user_code: "ABCDEFGH",
      selection: { kind: selection },
    });
  });
}

test("a malformed agent login URL fails visibly without starting authentication", async ({
  page,
}) => {
  const requests = await fixture(page, false);
  await page.goto("/login/agent-key?user_code=ABCD-EFGH%21");
  await expect(page.getByRole("alert")).toBeVisible();
  expect(requests).toEqual([]);
  await expect(
    page.getByRole("button", { name: "Continue with Google" }),
  ).toHaveCount(0);
});
