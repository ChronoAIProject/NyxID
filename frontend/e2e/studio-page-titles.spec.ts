import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";

// Exercise the real router and layout; authentication and data use HTTP fixtures.
test.beforeEach(async ({ page }) => {
  await mockDashboard(page, {
    enabledFeatures: ["inference:platform-models"],
  });
  const baseURL = test.info().project.use.baseURL ?? page.url();
  const gatewayURL = new URL("/api/v1/llm", baseURL).toString();
  await page.route("**/api/v1/keys", (route) =>
    route.fulfill({ json: { keys: [] } }),
  );
  await page.route("**/api/v1/triggers", (route) =>
    route.fulfill({ json: { triggers: [] } }),
  );
  await page.route("**/api/v1/providers", (route) =>
    route.fulfill({ json: { providers: [] } }),
  );
  await page.route("**/api/v1/llm/status", (route) =>
    route.fulfill({
      json: {
        providers: [],
        gateway_url: gatewayURL,
        supported_models: [],
      },
    }),
  );
  await page.route("**/api/v1/providers/my-tokens", (route) =>
    route.fulfill({ json: { tokens: [] } }),
  );
  await page.route("**/api/v1/onboarding/templates", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/onboarding/setups", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/onboarding/bindings", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/onboarding/setups/archived**", (route) =>
    route.fulfill({
      json: { setups: [], next_before_ms: null, next_before_id: null },
    }),
  );
  await page.route("**/api/v1/services/service-1", (route) =>
    route.fulfill({
      status: 404,
      contentType: "application/json",
      json: { error: "not_found", message: "Service not found" },
    }),
  );
  await page.route("**/api/v1/providers/provider-1", (route) =>
    route.fulfill({
      status: 404,
      contentType: "application/json",
      json: { error: "not_found", message: "Provider not found" },
    }),
  );
  await page.route("**/api/v1/llm/models**", (route) =>
    route.fulfill({ json: { models: [], next_after: null } }),
  );
});

for (const { path, title, heading } of [
  {
    path: "/devices/onboard",
    title: "nyxid - device onboard",
    heading: "Onboard device",
  },
  { path: "/triggers", title: "nyxid - triggers", heading: "Triggers" },
  { path: "/models", title: "nyxid - platform models", heading: "Platform Models" },
  { path: "/onboarding/setups", title: "nyxid - bot setups", heading: "Your bot setups" },
  { path: "/services", title: "nyxid - services", heading: "Services" },
  { path: "/providers", title: "nyxid - providers", heading: "Providers" },
]) {
  test(`${path} has its title on direct load and refresh`, async ({ page }) => {
    const pageErrors: string[] = [];
    const requestFailures: string[] = [];
    const appOrigin = new URL(test.info().project.use.baseURL ?? page.url()).origin;
    page.on("pageerror", (error) => pageErrors.push(error.message));
    page.on("requestfailed", (request) => {
      if (new URL(request.url()).origin !== appOrigin) return;
      requestFailures.push(`${request.method()} ${request.url()}: ${request.failure()?.errorText ?? "unknown"}`);
    });
    await page.goto(path, { waitUntil: "domcontentloaded" });
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
    await expect(page).toHaveTitle(title);
    expect(pageErrors).toEqual([]);
    expect(requestFailures).toEqual([]);

    await page.reload({ waitUntil: "domcontentloaded" });
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
    await expect(page).toHaveTitle(title);
    expect(pageErrors).toEqual([]);
    expect(requestFailures).toEqual([]);
  });
}

test("titles follow sidebar navigation and browser history", async ({ page }) => {
  const pageErrors: string[] = [];
  const requestFailures: string[] = [];
  const appOrigin = new URL(test.info().project.use.baseURL ?? page.url()).origin;
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("requestfailed", (request) => {
    if (new URL(request.url()).origin !== appOrigin) return;
    requestFailures.push(`${request.method()} ${request.url()}: ${request.failure()?.errorText ?? "unknown"}`);
  });
  await page.goto("/nodes", { waitUntil: "domcontentloaded" });
  await expect(page).toHaveTitle("nyxid - nodes");

  await page.getByRole("link", { name: "Device Onboard", exact: true }).click();
  await expect(page).toHaveURL(/\/devices\/onboard$/);
  await expect(page).toHaveTitle("nyxid - device onboard");

  await page.getByRole("link", { name: "Triggers", exact: true }).click();
  await expect(page).toHaveURL(/\/triggers$/);
  await expect(page).toHaveTitle("nyxid - triggers");

  await page.goBack();
  await expect(page).toHaveURL(/\/devices\/onboard$/);
  await expect(page).toHaveTitle("nyxid - device onboard");
  await page.goBack();
  await expect(page).toHaveURL(/\/nodes$/);
  await expect(page).toHaveTitle("nyxid - nodes");
  await page.goForward();
  await expect(page).toHaveTitle("nyxid - device onboard");
  await page.goForward();
  await expect(page).toHaveTitle("nyxid - triggers");

  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await expect(page).toHaveTitle("nyxid - settings");
  await page.getByRole("link", { name: "Integration Guide", exact: true }).click();
  await expect(page).toHaveTitle("nyxid - integration guide");
  expect(pageErrors).toEqual([]);
  expect(requestFailures).toEqual([]);
});

test("the legacy onboarding URL uses the destination title after redirect", async ({ page }) => {
  await page.goto("/settings/devices/onboard", { waitUntil: "domcontentloaded" });
  await expect(page).toHaveURL(/\/devices\/onboard$/);
  await expect(page.getByRole("heading", { name: "Onboard device", exact: true })).toBeVisible();
  await expect(page).toHaveTitle("nyxid - device onboard");
});

test("clears the dashboard title when leaving DashboardLayout", async ({ page }) => {
  await page.goto("/onboarding/setups", { waitUntil: "domcontentloaded" });
  await expect(page.getByRole("heading", { name: "Your bot setups", exact: true })).toBeVisible();
  await page.getByRole("link", { name: "Choose a starter", exact: true }).click();
  await expect(page).toHaveURL(/\/onboarding$/);
  await expect(page.getByRole("heading", { name: "Your own Telegram assistant", exact: true })).toBeVisible();
  await expect(page).toHaveTitle("nyxid");
});

test("every dashboard route section keeps a specific title, including detail and callback routes", async ({ page }) => {
  const pageErrors: string[] = [];
  const requestFailures: string[] = [];
  const appOrigin = new URL(test.info().project.use.baseURL ?? page.url()).origin;
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("requestfailed", (request) => {
    if (new URL(request.url()).origin !== appOrigin) return;
    requestFailures.push(`${request.method()} ${request.url()}: ${request.failure()?.errorText ?? "unknown"}`);
  });
  const routes = [
    ["/models", "nyxid - platform models"],
    ["/onboarding/setups", "nyxid - bot setups"],
    ["/onboarding/setups/setup-1", "nyxid - bot setups"],
    ["/services", "nyxid - services"],
    ["/services/service-1", "nyxid - services"],
    ["/services/service-1/edit", "nyxid - services"],
    ["/providers", "nyxid - providers"],
    ["/providers/callback", "nyxid - providers"],
    ["/providers/manage", "nyxid - providers"],
    ["/providers/provider-1", "nyxid - providers"],
    ["/providers/provider-1/edit", "nyxid - providers"],
  ] as const;

  for (const [path, title] of routes) {
    await page.goto(path, { waitUntil: "domcontentloaded" });
    await expect(page).toHaveURL(new RegExp(`${path.replaceAll("$", "\\$")}$`));
    await expect(page).toHaveTitle(title);
  }
  const serviceResponsePromise = page.waitForResponse(
    (response) => response.url().endsWith("/api/v1/services/service-1"),
  );
  await page.goto("/services/service-1", { waitUntil: "domcontentloaded" });
  const serviceResponse = await serviceResponsePromise;
  expect(serviceResponse.status()).toBe(404);
  await expect(page).toHaveTitle("nyxid - services");
  const providerResponsePromise = page.waitForResponse(
    (response) => response.url().endsWith("/api/v1/providers/provider-1"),
  );
  await page.goto("/providers/provider-1", { waitUntil: "domcontentloaded" });
  const providerResponse = await providerResponsePromise;
  expect(providerResponse.status()).toBe(404);
  await expect(page).toHaveTitle("nyxid - providers");
  expect(pageErrors).toEqual([]);
  expect(requestFailures).toEqual([]);
});
