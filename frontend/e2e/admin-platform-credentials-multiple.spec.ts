import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  test(`shared provider and field clears require impact confirmation at ${String(viewport.width)}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await mockDashboard(page);
    const provider = {
      provider: "x", label: "X (Twitter)", platform: "x", available: true,
      backing: { type: "provider_oauth", provider_slug: "twitter" },
      updated_at: "2026-09-08T00:00:00Z", setup_checklist: [],
      webhook_verify_token: null, callback_url: null,
      fields: ["client_id", "client_secret"].map((name) => ({ name,
        label: name === "client_id" ? "Client ID" : "Client Secret",
        secret: true, required: true, numeric: false, configured: true, help: "" })),
    };
    await page.route("**/api/v1/admin/platform-credentials", (route) => route.fulfill({ json: [provider] }));
    const writes: { method: string; body: unknown }[] = [];
    await page.route("**/api/v1/admin/platform-credentials/x", async (route) => {
      const method = route.request().method();
      writes.push({ method, body: method === "PATCH" ? route.request().postDataJSON() : null });
      await route.fulfill(method === "DELETE" ? { status: 204 } : { json: provider });
    });
    await page.goto("/admin/platform-credentials");
    const warning = "These credentials are shared with the twitter provider. Clearing them stops all of its OAuth connections and logins until credentials are restored.";
    for (const name of ["Clear Client ID", "Clear Client Secret", "Clear provider"]) {
      await page.getByRole("button", { name, exact: true }).click();
      await expect(page.getByRole("dialog").getByText(warning, { exact: true })).toBeVisible();
      await page.getByRole("dialog").getByRole("button", { name: "Cancel", exact: true }).click();
      expect(writes).toEqual([]);
      await expect(page.getByRole("button", { name: "Save credentials" })).toBeDisabled();
    }
    await page.getByRole("button", { name: "Clear Client Secret", exact: true }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Confirm", exact: true }).click();
    await expect.poll(() => writes.length).toBe(1);
    expect(writes[0]).toEqual({ method: "PATCH", body: { fields: { client_secret: null } } });
    await page.getByRole("button", { name: "Clear provider", exact: true }).click();
    await expect(page.getByRole("dialog").getByText(warning, { exact: true })).toBeVisible();
    await page.getByRole("dialog").getByRole("button", { name: "Confirm", exact: true }).click();
    await expect.poll(() => writes.length).toBe(2);
    expect(writes[1]).toEqual({ method: "DELETE", body: null });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  });

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
