import { expect, test } from "@playwright/test";

for (const viewport of [{ width: 1440, height: 1000 }, { width: 390, height: 844 }]) {
  test(`WhatsApp registration and one-time Verify Token at ${String(viewport.width)}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const secret = "e2e-verification-".padEnd(64, "0");
    const bot = {
      id: "whatsapp-e2e", platform: "whatsapp", label: "Customer Support",
      platform_bot_id: "123456", phone_number_id: "123456", waba_id: "654321",
      platform_bot_username: "+1 555 123 4567", status: "pending_webhook",
      is_active: true, webhook_registered: false, app_secret_configured: true,
      lark_verification_token_configured: false, lark_encrypt_key_configured: false,
      conversations_count: 0, created_at: "2026-09-08T00:00:00Z", updated_at: "2026-09-08T00:00:00Z",
      user_id: "test-user", webhook_url: "https://nyxid.example/api/v1/webhooks/channel/whatsapp/whatsapp-e2e",
      webhook_secret_label: "Verify Token",
      setup_instructions: ["Enter the Callback URL and Verify Token in Meta App Dashboard.", "Subscribe to the messages webhook field."],
    };
    let registration: unknown;
    let rotation: unknown;
    await page.route("**/api/v1/**", async (route) => {
      const path = new URL(route.request().url()).pathname.replace("/api/v1", "");
      let body: unknown = {};
      if (path === "/users/me") body = { id: "test-user", email: "test@example.com", display_name: "Test User", is_admin: true, is_active: true, email_verified: true, created_at: bot.created_at };
      else if (path === "/channel-bots" && route.request().method() === "POST") {
        registration = route.request().postDataJSON();
        body = { ...bot, webhook_secret: secret };
      } else if (path === "/channel-bots/whatsapp-e2e" && route.request().method() === "PATCH") {
        rotation = route.request().postDataJSON();
        body = bot;
      } else if (path === "/channel-bots") body = { bots: [], total: 0 };
      else if (path === "/channel-bots/whatsapp-e2e") body = bot;
      else if (path.includes("conversations")) body = { conversations: [], total: 0 };
      else if (path === "/orgs") body = { organizations: [], orgs: [] };
      else if (path === "/api-keys") body = { keys: [] };
      else if (path === "/nodes") body = { nodes: [] };
      await route.fulfill({ json: body });
    });
    await page.goto("/channel-bots");
    await page.getByRole("button", { name: "Add Bot", exact: true }).first().click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("combobox").last().click();
    await page.getByRole("option", { name: "WhatsApp", exact: true }).click();
    await dialog.getByLabel("Label", { exact: true }).fill(bot.label);
    await dialog.getByLabel("Access Token", { exact: true }).fill("e2e-access-token");
    await dialog.getByLabel("Phone Number ID", { exact: true }).fill(bot.phone_number_id);
    await dialog.getByLabel("Meta App Secret", { exact: true }).fill("e2e-app-secret");
    await dialog.getByLabel("WhatsApp Business Account ID (optional)", { exact: true }).fill(bot.waba_id);
    await expect(dialog.getByRole("button", { name: "Add Bot", exact: true })).toBeEnabled();
    await dialog.screenshot({ path: `/tmp/nyxid-whatsapp-form-${String(viewport.width)}.png` });
    await dialog.getByRole("button", { name: "Add Bot", exact: true }).click();
    await expect(dialog.getByText(secret, { exact: true })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Copy Verify Token", exact: true })).toBeVisible();
    expect(registration).toEqual({ platform: "whatsapp", label: bot.label, bot_token: "e2e-access-token", phone_number_id: "123456", waba_id: "654321", app_secret: "e2e-app-secret" });
    await dialog.screenshot({ path: `/tmp/nyxid-whatsapp-created-${String(viewport.width)}.png` });
    expect(await dialog.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    await dialog.getByRole("button", { name: "Done", exact: true }).click();
    await expect(page).toHaveURL(/channel-bots\/whatsapp-e2e$/);
    await expect(page.getByText(secret, { exact: true })).toHaveCount(0);
    await expect(page.getByText("Phone Number ID", { exact: true })).toBeVisible();
    await page.getByLabel("Access Token", { exact: true }).fill("replacement-access-token");
    await page.getByLabel("Meta App Secret", { exact: true }).fill("replacement-app-secret");
    await page.getByRole("button", { name: "Save Credentials", exact: true }).click();
    await expect(page.getByLabel("Access Token", { exact: true })).toHaveValue("");
    await expect(page.getByLabel("Meta App Secret", { exact: true })).toHaveValue("");
    expect(rotation).toEqual({ bot_token: "replacement-access-token", app_secret: "replacement-app-secret" });
    await page.screenshot({ path: `/tmp/nyxid-whatsapp-detail-${String(viewport.width)}.png`, fullPage: true });
    expect(errors).toEqual([]);
  });
}
