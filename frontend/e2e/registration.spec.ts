import { expect, test, type Page } from "@playwright/test";

async function mockPublicAuth(page: Page, emailEnabled: boolean) {
  await page.route("**/api/v1/**", (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/api/v1/public/config") {
      return route.fulfill({
        json: {
          backend_url: "http://localhost:3001",
          frontend_url: "http://localhost:4611",
          mcp_url: "http://localhost:3001/mcp",
          node_ws_url: "ws://localhost:3001/api/v1/nodes/ws",
          version: "test",
          social_providers: ["google", "github", "apple"],
          email_auth_enabled: emailEnabled,
        },
      });
    }
    return route.fulfill({
      status: 401,
      json: { error: "unauthorized", error_code: 1001, message: "Sign in" },
    });
  });
}

for (const width of [390, 1440]) {
  test(`email signup needs no invitation code at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mockPublicAuth(page, true);
    let registration: unknown;
    await page.route("**/api/v1/auth/register", (route) => {
      registration = route.request().postDataJSON();
      return route.fulfill({
        json: { user_id: "new-user", message: "Check your email." },
      });
    });
    await page.goto("/register?return_to=%2Fteam&code=RETIRED");
    await expect(
      page.getByRole("heading", { name: "Create your account" }),
    ).toBeVisible();
    await expect(page.getByPlaceholder("NYX-XXXXXXXX")).toHaveCount(0);
    for (const provider of ["Google", "GitHub", "Apple"]) {
      await expect(
        page.getByRole("button", { name: `Continue with ${provider}` }),
      ).toBeVisible();
    }
    await page.screenshot({ path: `/tmp/nyxid-register-methods-${width}.png` });
    await page.getByRole("button", { name: "Continue with Email" }).click();
    await expect(
      page.getByRole("heading", { name: "Email registration" }),
    ).toBeVisible();
    await page.getByLabel("Full Name").fill("Ada Lovelace");
    await page
      .getByLabel("Email", { exact: true })
      .last()
      .fill("ada@example.com");
    await page
      .getByLabel("Password", { exact: true })
      .last()
      .fill("Password123");
    await page.getByLabel("Confirm Password").fill("Password123");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({ path: `/tmp/nyxid-register-email-${width}.png` });
    await page
      .getByRole("button", { name: "Create Account", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "Welcome back" }),
    ).toBeVisible();
    expect(registration).toEqual({
      display_name: "Ada Lovelace",
      email: "ada@example.com",
      password: "Password123",
    });
    expect(new URL(page.url()).searchParams.get("return_to")).toBe("/team");
    expect(new URL(page.url()).searchParams.has("code")).toBe(false);
  });
}

test("social-only registration keeps email signup hidden", async ({ page }) => {
  await mockPublicAuth(page, false);
  await page.goto("/register");
  await expect(
    page.getByRole("button", { name: "Continue with Apple" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Continue with Email" }),
  ).toHaveCount(0);
  await expect(page.getByPlaceholder("NYX-XXXXXXXX")).toHaveCount(0);
});
