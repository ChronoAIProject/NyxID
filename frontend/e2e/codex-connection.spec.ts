import { expect, test, type Page } from "@playwright/test";

const response = {
  account_id: "8ae56725-6879-49be-a382-30a8b40c334e",
  account_email: "human@example.com",
  provider_id: "bac09f6f-f185-4e71-b0b0-50f7372f70c6",
  provider_slug: "openai",
  connection: { id: "b222e258-e1ba-4862-a5c6-453916ea28df", state_version: 1 },
  status: "usable",
  service_id: "45a3b8d1-c802-40ea-8cb7-398e50db8f16",
  feature: "openai_responses",
};

async function fixture(page: Page) {
  const state = {
    response: structuredClone(response),
    failVerify: false,
    failStatus: false,
  };
  const verifications: {
    connection: typeof response.connection;
    model: string;
  }[] = [];
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    let status = 200;
    let body: unknown = {};
    if (path === "/api/v1/users/me") {
      body = {
        id: response.account_id,
        display_name: "Human",
        email: response.account_email,
        is_active: true,
        role: "user",
        email_verified: true,
        feature_flags: {},
      };
    } else if (path === "/api/v1/providers/codex-connection/verify") {
      verifications.push(request.postDataJSON());
      if (state.failVerify) {
        status = 502;
        body = { error: { message: "Fixture unavailable", code: 1000 } };
      } else {
        body = state.response;
      }
    } else if (path === "/api/v1/providers/codex-connection") {
      if (state.failStatus) {
        status = 502;
        body = { error: { message: "Fixture unavailable", code: 1000 } };
      } else {
        body = state.response;
      }
    } else if (path === "/api/v1/keys") {
      body = { keys: [] };
    } else if (path === "/api/v1/user-services") {
      body = { services: [] };
    } else if (path === "/api/v1/catalog") {
      body = { entries: [] };
    } else if (path === "/api/v1/nodes") {
      body = { nodes: [] };
    } else if (path === "/api/v1/orgs") {
      body = { orgs: [] };
    } else if (path === "/api/v1/public/config") {
      body = { telemetry_dsn: null, telemetry_share_analytics: false };
    } else {
      status = 404;
      body = { message: "Not found" };
    }
    await route.fulfill({ status, json: body });
  });
  await page.goto("/keys");
  await page
    .getByRole("button", { name: "Codex connection", exact: true })
    .click();
  await expect(page.getByText("Usable", { exact: true })).toBeVisible();
  return { state, verifications };
}

test("Codex verification requires confirmation and failure clears usable status", async ({
  page,
}) => {
  const { state, verifications } = await fixture(page);
  state.failVerify = true;
  await page
    .getByRole("button", { name: "Verify connection", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText(response.account_email);
  expect(verifications).toHaveLength(0);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(verifications).toHaveLength(0);
  await page
    .getByRole("button", { name: "Verify connection", exact: true })
    .click();
  await page.getByRole("button", { name: "Verify", exact: true }).click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Connection verification" }),
  ).toBeVisible();
  await expect(page.getByText("Usable", { exact: true })).toHaveCount(0);
  expect(verifications).toEqual([
    { connection: response.connection, model: "gpt-4.1-mini" },
  ]);
});

test("Codex confirmation keeps the reviewed connection when a background read changes it", async ({
  page,
}) => {
  const { state, verifications } = await fixture(page);
  await page
    .getByRole("button", { name: "Verify connection", exact: true })
    .click();
  state.response.connection.state_version = 2;
  await page.evaluate(() =>
    (
      window as unknown as {
        __nyxQueryClient: import("@tanstack/react-query").QueryClient;
      }
    ).__nyxQueryClient.refetchQueries({ queryKey: ["codex-connection"] }),
  );
  await page.getByRole("button", { name: "Verify", exact: true }).click();
  await expect.poll(() => verifications.length).toBe(1);
  expect(verifications[0]?.connection).toEqual(response.connection);
});

test("Codex verification cancels an earlier status response", async ({
  page,
}) => {
  const { state } = await fixture(page);
  state.failVerify = true;
  await page
    .getByRole("button", { name: "Verify connection", exact: true })
    .click();
  let release: (() => void) | undefined;
  await page.route("**/api/v1/providers/codex-connection", async (route) => {
    await new Promise<void>((resolve) => {
      release = resolve;
    });
    await route.fulfill({ json: response }).catch(() => {});
  });
  await page.evaluate(() => {
    void (
      window as unknown as {
        __nyxQueryClient: import("@tanstack/react-query").QueryClient;
      }
    ).__nyxQueryClient.refetchQueries({ queryKey: ["codex-connection"] });
  });
  await expect.poll(() => Boolean(release)).toBe(true);
  await page.getByRole("button", { name: "Verify", exact: true }).click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Connection verification" }),
  ).toBeVisible();
  release!();
  await page.waitForTimeout(500);
  await expect(page.getByText("Usable", { exact: true })).toHaveCount(0);
  await expect(
    page.getByText("Saved, verification pending", { exact: true }),
  ).toBeVisible();
});

for (const width of [1440, 320]) {
  test(`Codex connection panel fits at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await fixture(page);
    await expect(
      page.getByRole("link", { name: "Manage service" }),
    ).toHaveAttribute("href", `/keys/${response.service_id}`);
    await expect(
      page.getByRole("link", { name: "Authorize Codex separately" }),
    ).toHaveAttribute("href", /slug=llm-openai-codex/);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`codex-panel-${width}.png`),
      fullPage: true,
    });
  });
}
