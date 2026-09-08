import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  test(`platform credentials isolate two provider forms at ${String(viewport.width)}px`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await mockDashboard(page);
    const shared = {
      available: false,
      updated_at: null,
      webhook_verify_token: null,
      setup_checklist: [],
    };
    const field = (name: string, label: string) => ({
      name,
      label,
      secret: true,
      required: true,
      numeric: false,
      configured: false,
      help: "",
    });
    const meta = {
      ...shared,
      provider: "meta",
      label: "Meta",
      platform: "whatsapp",
      callback_url:
        "https://nyxid.example/api/v1/webhooks/channel/whatsapp/platform",
      fields: [field("app_secret", "App Secret")],
    };
    const x = {
      ...shared,
      provider: "x",
      label: "X (Twitter)",
      platform: "x",
      backing: { type: "provider_oauth", provider_slug: "twitter" },
      callback_url: "https://nyxid.example/api/v1/providers/callback",
      fields: [
        field("client_id", "Client ID"),
        field("client_secret", "Client Secret"),
      ],
    };
    await page.route("**/api/v1/admin/platform-credentials", (route) =>
      route.fulfill({ json: [meta, x] }),
    );
    const updates: unknown[] = [];
    await page.route(
      "**/api/v1/admin/platform-credentials/x",
      async (route) => {
        updates.push(route.request().postDataJSON());
        x.fields.forEach((field) => {
          field.configured = true;
        });
        x.available = true;
        await route.fulfill({ json: x });
      },
    );
    await page.goto("/admin/platform-credentials");
    await expect(
      page.getByRole("heading", { name: "Meta", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "X (Twitter)", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Shared with the twitter provider."),
    ).toBeVisible();
    const section = page
      .locator("section")
      .filter({
        has: page.getByRole("heading", { name: "X (Twitter)", exact: true }),
      });
    await page.getByLabel("App Secret", { exact: true }).fill("unsaved-meta");
    await section.getByLabel("Client ID", { exact: true }).fill("x-client");
    await section
      .getByLabel("Client Secret", { exact: true })
      .fill("private-secret");
    await section.getByRole("button", { name: "Save credentials" }).click();
    await expect(
      section.getByLabel("Client Secret", { exact: true }),
    ).toHaveValue("");
    await expect(section.getByLabel("Client ID", { exact: true })).toHaveValue(
      "",
    );
    expect(updates).toEqual([
      { fields: { client_id: "x-client", client_secret: "private-secret" } },
    ]);
    await expect(page.getByLabel("App Secret", { exact: true })).toHaveValue(
      "unsaved-meta",
    );
    await expect(
      section.getByRole("button", { name: "Regenerate verify token" }),
    ).toHaveCount(0);
    await page.screenshot({
      path: `/tmp/nyx-admin-multiple-${String(viewport.width)}.png`,
      fullPage: true,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  });
}
