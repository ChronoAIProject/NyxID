import { expect, test, type Page } from "@playwright/test";

import { loginInventory } from "../src/lib/__fixtures__/login-inventory";

async function fixture(
  page: Page,
  authenticated: boolean,
  mfa = false,
  capability = true,
  inventory = loginInventory(),
) {
  const requests: { path: string; body: unknown }[] = [];
  const identity = {
    id: "11111111-1111-4111-8111-111111111111",
    flow: "device",
    user_code: "ABCDEFGH",
    keep_signed_in: false,
    verified: false,
    mfa_required: false,
    expires_at: new Date(Date.now() + 600000).toISOString(),
    user: { id: "user", email: "human@example.com", display_name: "Human" },
  };
  const approvalPath = `/api/v1/auth/approval/${identity.id}`;
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
    if (
      path.startsWith("/api/v1/auth/device/") ||
      path.startsWith("/api/v1/auth/approval")
    ) {
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
        supports_grant_choice: capability,
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
      body = inventory.options;
    } else if (path === "/api/v1/keys") {
      body = { keys: inventory.connections };
    } else if (path === "/api/v1/catalog") {
      body = { entries: inventory.catalog };
    } else if (
      [
        "/api/v1/auth/device/approve",
        "/api/v1/auth/device/approve-agent-key",
        "/api/v1/auth/device/deny",
      ].includes(path)
    ) {
      body = { ok: true };
    } else if (path === "/api/v1/auth/approval") {
      identity.keep_signed_in = route.request().postDataJSON().keep_signed_in;
      body = identity;
    } else if (path === approvalPath) {
      body = identity;
    } else if (
      path === `${approvalPath}/password` ||
      path === `${approvalPath}/mfa`
    ) {
      identity.mfa_required = mfa && path.endsWith("/password");
      identity.verified = !identity.mfa_required;
      authenticated = identity.verified && identity.keep_signed_in;
      body = identity;
    } else if (path === `${approvalPath}/inventory`) {
      body = {
        options: inventory.options,
        catalog: { entries: inventory.catalog },
      };
    } else if (
      path === `${approvalPath}/approve` ||
      path === `${approvalPath}/deny`
    ) {
      body = { ok: true };
    } else if (path.startsWith("/api/v1/auth/social/")) {
      const params = new URL(route.request().url()).searchParams;
      expect(params.get("approval_id")).toBe(identity.id);
      expect(new URL(params.get("return_to")!).origin).toBe(
        new URL(page.url()).origin,
      );
      identity.verified = true;
      authenticated = identity.keep_signed_in;
      await route.fulfill({
        status: 302,
        headers: { location: "/login/device?user_code=ABCDEFGH" },
      });
      return;
    } else if (path === "/api/v1/public/config") {
      body = {
        telemetry_dsn: null,
        telemetry_share_analytics: false,
        email_auth_enabled: true,
        social_providers: ["google"],
      };
    } else {
      status = 404;
      body = { message: "Not found" };
    }
    await route.fulfill({ status, json: body });
  });
  return requests;
}

async function verify(page: Page) {
  await expect(
    page.getByRole("region", { name: "Request details" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
}
async function restricted(page: Page) {
  await verify(page);
  await page.getByRole("radio", { name: /Restricted Agent Key/ }).check();
  await page.getByRole("button", { name: "Continue to approval" }).click();
}
const hints =
  "user_code=abcd%20efgh&login_type=agent&permissions=read,proxy&service_permissions=github::repo:read";

test("one dropdown chooses permissions and an explicit connection when accounts are ambiguous", async ({
  page,
}, info) => {
  const inventory = loginInventory();
  const other = {
    ...inventory.connections[0]!,
    id: "work-github",
    label: "GitHub work",
    slug: "github-work",
    permission_snapshot: "w".repeat(64),
  };
  inventory.connections.push(other);
  inventory.options.connections = inventory.connections;
  inventory.options.services.push({
    id: other.id,
    name: "GitHub work",
    owner_id: "user",
  });
  inventory.catalog.push({
    slug: "calendar",
    name: "Calendar",
    scope_catalog: [
      {
        scope: "calendar:read",
        label: "Read calendar",
        description: "Read events",
      },
    ],
  });
  const requests = await fixture(page, true, false, true, inventory);
  await page.goto(`/login/device?${hints}&key_name=Build+agent&key_source=new`);
  await restricted(page);
  await page
    .getByRole("button", { name: "Create new Agent Key", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Create & continue" }),
  ).toBeDisabled();
  await expect(
    page.locator('details[aria-label="GitHub access"] summary'),
  ).toContainText("Choose account");
  const inlineAccount = page.locator('details[aria-label="GitHub access"]');
  await inlineAccount.locator("summary").focus();
  await page.keyboard.press("Enter");
  const workAccount = inlineAccount.getByRole("checkbox", {
    name: "Grant connection GitHub work",
  });
  await workAccount.check();
  await expect(
    page.getByRole("button", { name: "Create & continue" }),
  ).toBeEnabled();
  await expect(workAccount).toBeFocused();
  await workAccount.uncheck();
  await inlineAccount.locator("summary").click();
  await expect(
    page.getByRole("button", { name: "Create & continue" }),
  ).toBeDisabled();
  expect(requests.filter((r) => r.path.includes("approve"))).toEqual([]);
  const search = page.getByRole("textbox", {
    name: "Search permissions & connections",
  });
  const settings = page.getByText("Customize", {
    exact: true,
  });
  await settings.click();
  const allConnections = page.getByRole("checkbox", {
    name: "All current and future services",
    exact: true,
  });
  await allConnections.check();
  await search.fill("GitHub");
  await expect(
    page.getByRole("checkbox", { name: "Grant connection GitHub personal" }),
  ).toBeDisabled();
  await search.press("ArrowDown");
  await expect(
    page.getByRole("checkbox", { name: "GitHub: All 2 permissions" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await allConnections.uncheck();
  await search.fill("GitHub");
  await expect(
    page.getByRole("checkbox", { name: "Grant connection GitHub personal" }),
  ).not.toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: "Grant connection GitHub work" }),
  ).not.toBeChecked();
  await page
    .getByRole("checkbox", { name: "Grant connection GitHub work" })
    .check();
  await search.fill("Read calendar");
  await expect(page.getByText("Not connected", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("checkbox", { name: /Grant connection/ }),
  ).toHaveCount(0);
  await search.fill("github-work");
  await expect(
    page.getByRole("checkbox", { name: "Grant connection GitHub work" }),
  ).toBeChecked();
  expect(requests.filter((r) => r.path.includes("approve"))).toEqual([]);
  for (const [width, height] of [
    [1280, 900],
    [390, 844],
  ]) {
    await page.setViewportSize({ width: width!, height: height! });
    await search.scrollIntoViewIfNeeded();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: info.outputPath(`combined-picker-${width}.png`),
      fullPage: true,
    });
  }
  await search.press("Escape");
  await expect(search).toBeFocused();
  await expect(
    page.locator('details[aria-label="GitHub access"] summary'),
  ).toContainText("GitHub work");
  await page.getByRole("button", { name: "Create & continue" }).click();
  await expect(
    page.getByText("Approved — return to the requesting device"),
  ).toBeVisible();
  expect(requests.filter((r) => r.path.includes("approve"))).toEqual([
    {
      path: "/api/v1/auth/device/approve-agent-key",
      body: expect.objectContaining({
        selection: expect.objectContaining({
          allowed_service_ids: ["work-github"],
          connection_snapshots: [
            { service_id: "work-github", permission_snapshot: "w".repeat(64) },
          ],
        }),
      }),
    },
  ]);
});

test("adding another service changes the new key draft without creating a service or approving early", async ({
  page,
}, info) => {
  const inventory = loginInventory();
  const slack = {
    ...inventory.connections[0]!,
    id: "slack-work",
    label: "Slack workspace",
    slug: "slack-work",
    catalog_service_slug: "slack",
    granted_scopes: ["channels:read"],
    permission_snapshot: "s".repeat(64),
  };
  inventory.connections.push(slack);
  inventory.options.connections = inventory.connections;
  inventory.options.services.push({
    id: slack.id,
    name: "Slack workspace",
    owner_id: "user",
  });
  inventory.catalog.push({ slug: "slack", name: "Slack" });
  const requests = await fixture(page, true, false, true, inventory);
  const writes: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (
      request.method() === "POST" &&
      !["/api/v1/auth/device/preview", "/api/v1/auth/device/options"].includes(
        path,
      )
    )
      writes.push(path);
  });
  await page.goto(`/login/device?${hints}&key_name=Build+agent&key_source=new`);
  await restricted(page);
  await page.getByRole("button", { name: "Reader", exact: true }).click();
  await page.getByText("Customize", { exact: true }).click();
  const previousExpiry = new Date(Date.now() + 86400000)
    .toISOString()
    .slice(0, 16);
  await page
    .getByLabel("Login credential expiry (optional)")
    .fill(previousExpiry);
  await page.getByRole("button", { name: "Change key", exact: true }).click();
  await page
    .getByRole("button", { name: "Create new Agent Key", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Build agent" }),
  ).toBeVisible();
  const search = page.getByRole("textbox", {
    name: "Search permissions & connections",
  });
  await page.getByText("Customize", { exact: true }).click();
  await search.fill("Slack");
  await page
    .getByRole("checkbox", { name: "Grant connection Slack workspace" })
    .check();
  await search.press("Escape");
  await page.getByText("Customize", { exact: true }).click();
  const review = page.getByRole("region", { name: "Authorize access" });
  await expect(
    review.locator('details[aria-label="Slack access"] summary'),
  ).toContainText("Slack workspace");
  await expect(
    review.getByText("Creates new key", { exact: true }),
  ).toBeVisible();
  await expect(review.locator("details[open]")).toHaveCount(0);
  await expect(
    review.getByText("Matched + 2 extras", { exact: true }),
  ).toBeVisible();
  const slackRow = review.locator('details[aria-label="Slack access"]');
  await slackRow.locator("summary").click();
  await expect(
    slackRow.getByRole("heading", { name: "Extra access included" }),
  ).toBeVisible();
  await slackRow.locator("summary").click();
  await expect(
    slackRow.getByRole("heading", { name: "Extra access included" }),
  ).toBeHidden();
  expect(writes).toEqual([]);
  for (const [width, height] of [
    [1280, 900],
    [390, 844],
  ]) {
    await page.setViewportSize({ width: width!, height: height! });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: info.outputPath(`additional-service-${width}.png`),
      fullPage: true,
    });
    expect((await review.boundingBox())!.height).toBeLessThan(600);
    await review.screenshot({
      path: info.outputPath(`compact-review-${width}.png`),
    });
  }
  await page.getByRole("button", { name: "Create & continue" }).click();
  await expect(
    page.getByText("Approved — return to the requesting device"),
  ).toBeVisible();
  expect(writes).toEqual(["/api/v1/auth/device/approve-agent-key"]);
  expect(
    requests.find((r) => r.path.endsWith("/approve-agent-key"))?.body,
  ).toEqual(
    expect.objectContaining({
      selection: expect.objectContaining({
        kind: "new",
        allowed_service_ids: ["svc", "slack-work"],
        connection_snapshots: [
          { service_id: "svc", permission_snapshot: "c".repeat(64) },
          { service_id: "slack-work", permission_snapshot: "s".repeat(64) },
        ],
      }),
    }),
  );
});

test("public preview preserves hints and identity login requires fresh explicit consent", async ({
  page,
  context,
}) => {
  const requests = await fixture(page, false, true);
  await page.goto(
    `/login/device?${hints}&key_source=new&key_name=Build+agent&expiry_days=30&platform=codex`,
  );
  await expect(page.getByText("ABCD-EFGH", { exact: true })).toBeVisible();
  await page.getByText("Request details", { exact: true }).click();
  await expect(page.getByText("home-agent", { exact: true })).toBeVisible();
  expect(requests.filter((r) => r.path.includes("/approval"))).toEqual([]);
  expect(await context.cookies()).toEqual([]);
  expect(
    await page.evaluate(() => ({
      local: localStorage.length,
      session: sessionStorage.length,
    })),
  ).toEqual({ local: 0, session: 0 });
  await page.getByRole("button", { name: "Continue with email" }).click();
  await page.getByLabel("Email", { exact: true }).fill("human@example.com");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: "Verify & continue" }).click();
  await page.getByLabel("Authenticator code").fill("123456");
  await page.getByRole("button", { name: "Verify & continue" }).click();
  await expect(page).toHaveURL(/\/login\/device\?/);
  expect(new URL(page.url()).searchParams.get("key_name")).toBe("Build agent");
  expect(requests.filter((r) => r.path.includes("approve"))).toEqual([]);
  await page.getByRole("radio", { name: /Restricted Agent Key/ }).check();
  await page.getByRole("button", { name: "Continue to approval" }).click();
  await page.getByRole("button", { name: "Create new Agent Key" }).click();
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
    "Build agent",
  );
  const expiryDate = await page.evaluate(() =>
    new Date(Date.now() + 30 * 86400000).toISOString().slice(0, 10),
  );
  const expiryLabel = new Date(`${expiryDate}T00:00:00`).toLocaleDateString(
    "en-US",
    { month: "long", day: "numeric", year: "numeric" },
  );
  await page.getByText("Customize", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: expiryLabel, exact: true }),
  ).toBeVisible();
  await expect(
    page.locator('details[aria-label="GitHub access"] summary'),
  ).toContainText("GitHub personal");
  await page.getByRole("button", { name: "Create & continue" }).click();
  await expect(
    page.getByText("Approved — return to the requesting device"),
  ).toBeVisible();
  expect(requests.filter((r) => r.path.includes("approve"))).toEqual([
    {
      path: "/api/v1/auth/approval/11111111-1111-4111-8111-111111111111/approve",
      body: expect.objectContaining({
        selection: expect.objectContaining({
          kind: "new",
          name: "Build agent",
          expires_at: `${expiryDate}T23:59:59.000Z`,
          allowed_service_ids: ["svc"],
          platform: "codex",
        }),
      }),
    },
  ]);
  expect(
    requests.find((r) => r.path.includes("approve"))?.body,
  ).not.toHaveProperty("credential_expires_at");
});

for (const grant of ["account", "agent-key"] as const) {
  test(`three-step ${grant} approval renders at desktop and phone widths`, async ({
    page,
  }, info) => {
    const requests = await fixture(page, true);
    await page.goto(`/login/device?${hints}`);
    await verify(page);
    await page
      .getByRole("radio", {
        name:
          grant === "account" ? /Full account access/ : /Restricted Agent Key/,
      })
      .check();
    await page.getByRole("button", { name: "Continue to approval" }).click();
    if (grant === "agent-key") {
      await expect(page.getByText("Exact match")).toBeVisible();
      await page.getByRole("button", { name: "Reader", exact: true }).click();
      const review = page.getByRole("region", { name: "Authorize access" });
      await expect(review).toHaveCount(1);
      await expect(
        review.getByRole("heading", { name: "Reader", exact: true }),
      ).toBeFocused();
      await page
        .getByRole("button", { name: "Change key", exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "Choose an existing Agent Key" }),
      ).toBeFocused();
      await page.getByRole("button", { name: "Reader", exact: true }).click();
      await expect(
        review.getByText("Existing key", { exact: true }),
      ).toBeVisible();
      await expect(review.locator("details[open]")).toHaveCount(0);
      await expect(
        review.getByText("Effective permissions", { exact: true }),
      ).toBeHidden();
      await review.getByText("Customize", { exact: true }).focus();
      await page.keyboard.press("Enter");
      await expect(
        review.getByText("Effective permissions", { exact: true }),
      ).toBeVisible();
      const loginExpiry = await page.evaluate(() =>
        new Date(Date.now() + 86400000).toISOString().slice(0, 16),
      );
      await page
        .getByLabel("Login credential expiry (optional)")
        .fill(loginExpiry);
      await expect(review).toContainText(
        await page.evaluate(
          (value) => new Date(value).toLocaleDateString(),
          loginExpiry,
        ),
      );
      await review.getByText("Customize", { exact: true }).click();
      await expect(
        review.getByText("Effective permissions", { exact: true }),
      ).toBeHidden();
    }
    expect(requests.filter((r) => r.path.includes("approve"))).toEqual([]);
    for (const [size, width, height] of [
      ["desktop", 1440, 1000],
      ["mobile", 390, 844],
    ] as const) {
      await page.setViewportSize({ width, height });
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await page.screenshot({
        path: info.outputPath(`${grant}-${size}.png`),
        fullPage: true,
      });
    }
    await page
      .getByRole("button", {
        name:
          grant === "account"
            ? "Approve full account access"
            : "Approve access with this key",
        exact: true,
      })
      .click();
    await expect(
      page.getByText("Approved — return to the requesting device"),
    ).toBeVisible();
    expect(requests.filter((r) => r.path.includes("approve"))).toHaveLength(1);
  });
}

test("legacy capability cannot offer restricted access", async ({ page }) => {
  await fixture(page, true, false, false);
  await page.goto("/login/device?user_code=ABCDEFGH");
  await verify(page);
  await expect(page.getByRole("radio", { name: /Restricted/ })).toHaveCount(0);
  await expect(page.getByRole("radio", { name: /Full account/ })).toBeChecked();
});

test("permission search keeps stable selection, mixed state, keyboard focus and scroll", async ({
  page,
}) => {
  await fixture(page, true);
  await page.goto(`/login/device?${hints}`);
  await restricted(page);
  const search = page.getByRole("textbox", { name: "Search permissions" });
  await search.fill("GitHub");
  await search.press("Enter");
  const group = page.getByRole("checkbox", {
    name: "GitHub: All 2 permissions",
    exact: true,
  });
  await expect(group).toHaveAttribute("data-state", "indeterminate");
  await group.click();
  await expect(group).toBeChecked();
  await expect(search).toHaveValue("GitHub");
  await page
    .getByRole("checkbox", { name: "GitHub: Read repositories", exact: true })
    .focus();
  await page.keyboard.press("Escape");
  await expect(search).toBeFocused();
  await expect(group).toHaveCount(0);
  await page
    .getByRole("button", { name: "Remove GitHub: All 2 permissions" })
    .click();
  expect(
    await page.evaluate(
      () =>
        document.activeElement?.closest(
          '[aria-labelledby="selected-permissions-title"]',
        ) !== null,
    ),
  ).toBe(true);
  await search.focus();
  await page.getByRole("button", { name: "Create new Agent Key" }).focus();
  await expect(
    page.getByRole("checkbox", {
      name: "GitHub: All 2 permissions",
      exact: true,
    }),
  ).toHaveCount(0);
});

test("duplicate and unknown URL parameters fail visibly without making requests", async ({
  page,
}) => {
  const requests = await fixture(page, true);
  await page.goto(
    "/login/device?user_code=ABCDEFGH&login_type=agent&login_type=full&constructor=true",
  );
  await expect(page.getByRole("alert")).toContainText(
    "Unsupported request parameter: constructor",
  );
  await expect(page.getByRole("alert")).toContainText("duplicate login_type");
  await expect(
    page.getByRole("button", { name: "Continue", exact: true }),
  ).toBeDisabled();
  expect(requests).toEqual([]);
});

for (const long of [false, true]) {
  test(`social identity return preserves ${long ? "long" : "short"} request hints`, async ({
    page,
  }) => {
    const requests = await fixture(page, false);
    const params = new URLSearchParams({
      user_code: "ABCDEFGH",
      login_type: "agent",
      key_source: "new",
      key_name: "Social Agent",
      permissions: "read,proxy",
      service_permissions: Array(long ? 160 : 1)
        .fill("github::repo:read")
        .join(","),
    });
    await page.goto(`/login/device?${params}`);
    await expect(
      page.getByRole("button", { name: "Continue with Google" }),
    ).toBeVisible();
    expect(await page.evaluate(() => sessionStorage.length)).toBe(0);
    await page.getByRole("button", { name: /Google/ }).click();
    await expect(
      page.getByRole("button", { name: "Continue to approval" }),
    ).toBeVisible();
    expect(requests.filter((r) => r.path.includes("approve"))).toEqual([]);
    await page.getByRole("radio", { name: /Restricted Agent Key/ }).check();
    await page.getByRole("button", { name: "Continue to approval" }).click();
    await page.getByRole("button", { name: "Create new Agent Key" }).click();
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
      "Social Agent",
    );
    await expect(
      page.locator('details[aria-label="GitHub access"] summary'),
    ).toContainText("GitHub personal");
    await page.getByRole("button", { name: "Create & continue" }).click();
    await expect(
      page.getByText("Approved — return to the requesting device"),
    ).toBeVisible();
    expect(
      await page.evaluate(() =>
        Object.keys(sessionStorage).filter((k) =>
          k.startsWith("nyxid-approval:"),
        ),
      ),
    ).toEqual([]);
  });
}

test("new platform grant discloses future access and binds only same-owner current connections", async ({
  page,
}, testInfo) => {
  const inventory = loginInventory();
  inventory.options.personal_owner_id = "user";
  for (const owner of ["user", "org"]) {
    inventory.options.services.push({
      id: `platform-${owner}`,
      name: `Platform ${owner}`,
      owner_id: owner,
      auto_connected: true,
    });
    inventory.connections.push({
      ...inventory.connections[0]!,
      id: `platform-${owner}`,
      label: `Platform ${owner}`,
      catalog_service_slug: "platform",
      credential_binding: "platform",
      granted_scopes: null,
      permission_snapshot: owner.repeat(64).slice(0, 64),
    });
  }
  inventory.options.connections = inventory.connections;
  inventory.catalog.push({ slug: "platform", name: "Platform" });
  const requests = await fixture(page, true, false, true, inventory);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(
    `/login/device?${hints}&key_source=new&key_name=Platform+agent`,
  );
  await restricted(page);
  await page
    .getByRole("button", { name: "Create new Agent Key", exact: true })
    .click();
  await page.getByText("Customize", { exact: true }).click();
  await page
    .getByRole("checkbox", {
      name: "Allow all auto-connected platform services (includes ones added later)",
    })
    .check();
  await expect(
    page.getByText("Includes future platform services.", { exact: true }),
  ).toBeVisible();
  await page.getByText("Customize", { exact: true }).click();
  const platformRow = page.locator('details[aria-label="Platform access"]');
  await expect(platformRow.locator("summary")).toContainText("Platform user");
  await expect(platformRow.locator("summary")).not.toContainText(
    "Platform org",
  );
  expect(requests.filter((r) => r.path.includes("/approve"))).toEqual([]);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("platform-grant-mobile.png"),
    fullPage: true,
  });
  await page
    .getByRole("button", { name: "Create & continue", exact: true })
    .click();
  await expect(
    page.getByText("Approved — return to the requesting device", {
      exact: true,
    }),
  ).toBeVisible();
  expect(requests.filter((r) => r.path.includes("/approve"))).toEqual([
    {
      path: "/api/v1/auth/device/approve-agent-key",
      body: expect.objectContaining({
        selection: expect.objectContaining({
          allow_auto_connected_services: true,
          allowed_service_ids: ["svc"],
          connection_snapshots: [
            { service_id: "svc", permission_snapshot: "c".repeat(64) },
            {
              service_id: "platform-user",
              permission_snapshot: "user".repeat(64).slice(0, 64),
            },
          ],
        }),
      }),
    },
  ]);
});
