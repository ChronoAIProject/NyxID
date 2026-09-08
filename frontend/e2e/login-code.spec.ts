import { expect, test, type Page } from "@playwright/test";

const code = "JKLM-NPQR";
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

async function fixture(page: Page) {
  const state = {
    status: "pending",
    grant: "agent_key",
    expiry: new Date(Date.now() + 300000).toISOString(),
  };
  const requests: { path: string; method: string; body: unknown }[] = [];
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    let status = 200;
    let body: unknown = {};
    if (path.startsWith("/api/v1/auth/login-code")) {
      requests.push({
        path,
        method: request.method(),
        body: request.postData() ? request.postDataJSON() : null,
      });
    }
    if (path === "/api/v1/users/me") {
      body = {
        id: "user",
        display_name: "Human",
        email: "human@example.com",
        is_active: true,
        role: "user",
        email_verified: true,
      };
    } else if (path === "/api/v1/auth/login-code/options") {
      body = {
        keys: [key],
        services: key.allowed_services,
        nodes: [],
        orgs: [],
      };
    } else if (path === "/api/v1/auth/login-code") {
      state.grant = request.postDataJSON().auth_kind;
      body = { request_id: "request-fixture", code, expires_at: state.expiry };
    } else if (path === "/api/v1/auth/login-code/request-fixture/revoke") {
      state.status = "revoked";
      body = { ok: true };
    } else if (path === "/api/v1/auth/login-code/request-fixture") {
      if (request.method() === "DELETE") {
        state.status = "cancelled";
        body = { ok: true };
      } else {
        body = {
          request_id: "request-fixture",
          status: state.status,
          auth_kind: state.grant,
          expires_at: state.expiry,
          redeemed_at: ["redeemed", "revoked"].includes(state.status)
            ? new Date().toISOString()
            : null,
          client_label: "redeeming-workstation",
          client_user_agent: "NyxID CLI",
          client_ip: "203.0.113.25",
          client_ip_attribution: "verified",
          can_revoke: state.status === "redeemed",
        };
      }
    } else if (path === "/api/v1/public/config") {
      body = { telemetry_dsn: null, telemetry_share_analytics: false };
    } else {
      status = 404;
      body = { message: "Not found" };
    }
    await route.fulfill({ status, json: body });
  });
  return { state, requests };
}

async function mint(page: Page, grant: "account" | "agent-key") {
  await page.goto("/login/code");
  await expect(
    page.getByRole("button", { name: "Restricted Agent Key", exact: true }),
  ).toBeVisible();
  if (grant === "account") {
    await page
      .getByRole("button", { name: "Full account session", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "Confirm full account access" }),
    ).toBeVisible();
    await page.waitForTimeout(800);
    await page
      .getByRole("button", { name: "Generate account login code" })
      .click();
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
    await page.waitForTimeout(800);
    await page
      .getByRole("button", { name: "Generate restricted login code" })
      .click();
  }
  await expect(page.getByText(code, { exact: true })).toBeVisible();
}

for (const grant of ["account", "agent-key"] as const) {
  test(`${grant} code clears after redemption and exposes revocation`, async ({
    page,
  }, info) => {
    const { state, requests } = await fixture(page);
    await page.setViewportSize({ width: 390, height: 844 });
    await mint(page, grant);
    expect(
      requests.filter((request) => request.path === "/api/v1/auth/login-code"),
    ).toEqual([
      {
        path: "/api/v1/auth/login-code",
        method: "POST",
        body:
          grant === "account"
            ? { auth_kind: "account_session" }
            : {
                auth_kind: "agent_key",
                selection: { kind: "existing", api_key_id: "key" },
              },
      },
    ]);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: info.outputPath(`${grant}-code-mobile.png`),
      fullPage: true,
    });
    const persisted = await page.evaluate(() =>
      JSON.stringify({ localStorage, sessionStorage }),
    );
    expect(persisted).not.toContain(code);
    await expect
      .poll(() =>
        page.evaluate(() =>
          JSON.stringify(
            (
              window as unknown as {
                __nyxQueryClient: import("@tanstack/react-query").QueryClient;
              }
            ).__nyxQueryClient
              .getMutationCache()
              .getAll()
              .map((item) => item.state),
          ),
        ),
      )
      .not.toContain(code);
    state.status = "redeemed";
    await expect(
      page.getByRole("heading", { name: "Login redeemed" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(page.getByText(code, { exact: true })).toHaveCount(0);
    await expect(
      page.getByText("redeeming-workstation", { exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Revoke login" }).click();
    await expect(
      page.getByRole("button", { name: "Revoke login" }),
    ).toHaveCount(0);
    expect(state.status).toBe("revoked");
  });
}

test("cancelled code disappears and cannot be presented again", async ({
  page,
}) => {
  const { state } = await fixture(page);
  await mint(page, "account");
  await page.getByRole("button", { name: "Cancel code" }).click();
  await expect(page.getByText(code, { exact: true })).toHaveCount(0);
  expect(state.status).toBe("cancelled");
  await expect(page.getByRole("button", { name: "Cancel code" })).toHaveCount(
    0,
  );
});

test("code clears at local expiry while status is unavailable", async ({
  page,
}) => {
  const { state } = await fixture(page);
  state.expiry = new Date(Date.now() + 8000).toISOString();
  await mint(page, "account");
  await page.route("**/api/v1/auth/login-code/request-fixture", (route) =>
    route.abort(),
  );
  await expect(page.getByText(code, { exact: true })).toHaveCount(0, {
    timeout: 12000,
  });
  await expect(page.getByRole("button", { name: "Cancel code" })).toHaveCount(
    0,
  );
});
