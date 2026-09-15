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
    } else if (path.startsWith("/api/v1/auth/social/")) {
      const returnTo = new URL(route.request().url()).searchParams.get(
        "return_to",
      )!;
      expect(new URL(returnTo).origin).toBe(new URL(page.url()).origin);
      expect(encodeURIComponent(returnTo).length).toBeLessThan(3000);
      authenticated = true;
      await route.fulfill({ status: 302, headers: { location: returnTo } });
      return;
    } else if (path === "/api/v1/auth/login") {
      if (mfa) {
        status = 403;
        body = {
          error: "mfa_required",
          error_code: 2002,
          message: "MFA required",
          session_token: "fixture-mfa",
        };
      } else {
        authenticated = true;
        body = { ok: true };
      }
    } else if (path === "/api/v1/auth/mfa/verify") {
      authenticated = true;
      body = { ok: true };
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
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page
    .getByRole("button", { name: "This is my request — continue", exact: true })
    .click();
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
    page.getByRole("region", { name: "Connections to grant" }),
  ).toContainText("0 selected");
  const search = page.getByRole("textbox", {
    name: "Search permissions & connections",
  });
  const settings = page.getByText("Key settings and actual NyxID grant", {
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
  await settings.click();
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
    page.getByRole("region", { name: "Connections to grant" }),
  ).toContainText("1 selected");
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

test("public preview preserves hints and identity login requires fresh explicit consent", async ({
  page,
  context,
}) => {
  const requests = await fixture(page, false, true);
  await page.goto(
    `/login/device?${hints}&key_source=new&key_name=Build+agent&expiry_days=30&platform=codex`,
  );
  await expect(page.getByLabel("User code")).toHaveValue("ABCD-EFGH");
  expect(requests).toEqual([]);
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(page.getByText("Requested profile: home-agent")).toBeVisible();
  expect(await context.cookies()).toEqual([]);
  expect(
    await page.evaluate(() => ({
      local: localStorage.length,
      session: sessionStorage.length,
    })),
  ).toEqual({ local: 0, session: 0 });
  await page.getByRole("link", { name: "Verify identity to continue" }).click();
  await page.getByLabel("Email", { exact: true }).fill("human@example.com");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: /Sign in/i }).click();
  await page.getByLabel(/code/i).fill("123456");
  await page.getByRole("button", { name: /Verify/i }).click();
  await expect(page).toHaveURL(/\/login\/device\?/);
  expect(new URL(page.url()).searchParams.get("key_name")).toBe("Build agent");
  expect(requests.filter((r) => r.path.includes("approve"))).toEqual([]);
  await restricted(page);
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
  await page
    .getByText("Key settings and actual NyxID grant", { exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: expiryLabel, exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Remove connection GitHub personal" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Create & continue" }).click();
  await expect(
    page.getByText("Approved — return to the requesting device"),
  ).toBeVisible();
  expect(requests.filter((r) => r.path.includes("approve"))).toEqual([
    {
      path: "/api/v1/auth/device/approve-agent-key",
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
      await page.getByRole("radio", { name: "Reader", exact: true }).check();
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
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    expect(await page.evaluate(() => sessionStorage.length)).toBe(0);
    await page
      .getByRole("link", { name: "Verify identity to continue" })
      .click();
    expect(await page.evaluate(() => sessionStorage.length)).toBe(long ? 1 : 0);
    await page.getByRole("button", { name: /Google/ }).click();
    await expect(page.getByLabel("User code")).toHaveValue("ABCD-EFGH");
    expect(requests.filter((r) => r.path.includes("approve"))).toEqual([]);
    await restricted(page);
    await page.getByRole("button", { name: "Create new Agent Key" }).click();
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
      "Social Agent",
    );
    await expect(
      page.getByRole("button", { name: "Remove connection GitHub personal" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Create & continue" }).click();
    await expect(
      page.getByText("Approved — return to the requesting device"),
    ).toBeVisible();
    expect(
      await page.evaluate(() =>
        Object.keys(sessionStorage).filter((k) =>
          k.startsWith("nyxid:device-identity:"),
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
  await page
    .getByText("Key settings and actual NyxID grant", { exact: true })
    .click();
  await page
    .getByRole("checkbox", {
      name: "Allow all auto-connected platform services (includes ones added later)",
    })
    .check();
  await expect(
    page.getByText("All current and future auto-connected platform services", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Connections to grant" }),
  ).toContainText("Platform user");
  await expect(
    page.getByRole("region", { name: "Connections to grant" }),
  ).not.toContainText("Platform org");
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
