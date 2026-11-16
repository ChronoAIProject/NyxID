import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";

// Exercise the real router and layout; authentication and data use HTTP fixtures.
test.beforeEach(async ({ page }) => {
  await mockDashboard(page);
  await page.route("**/api/v1/keys", (route) =>
    route.fulfill({ json: { keys: [] } }),
  );
  await page.route("**/api/v1/triggers", (route) =>
    route.fulfill({ json: { triggers: [] } }),
  );
});

for (const { path, title, heading } of [
  {
    path: "/devices/onboard",
    title: "nyxid - device onboard",
    heading: "Onboard device",
  },
  { path: "/triggers", title: "nyxid - triggers", heading: "Triggers" },
]) {
  test(`${path} has its title on direct load and refresh`, async ({ page }) => {
    await page.goto(path);
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
    await expect(page).toHaveTitle(title);

    await page.reload();
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
    await expect(page).toHaveTitle(title);
  });
}

test("titles follow sidebar navigation and browser history", async ({ page }) => {
  await page.goto("/nodes");
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
});

test("the legacy onboarding URL uses the destination title after redirect", async ({ page }) => {
  await page.goto("/settings/devices/onboard");
  await expect(page).toHaveURL(/\/devices\/onboard$/);
  await expect(page.getByRole("heading", { name: "Onboard device", exact: true })).toBeVisible();
  await expect(page).toHaveTitle("nyxid - device onboard");
});
