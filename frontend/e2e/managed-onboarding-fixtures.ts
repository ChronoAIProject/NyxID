import type { Page } from "@playwright/test";

export const managedBot = {
  id: "managed-whatsapp",
  platform: "whatsapp",
  label: "Managed Support",
  platform_bot_id: "333",
  phone_number_id: "333",
  waba_id: "444",
  platform_bot_username: "+1 555 123 4567",
  status: "pending_webhook",
  is_active: true,
  webhook_registered: false,
  app_secret_configured: false,
  lark_verification_token_configured: false,
  lark_encrypt_key_configured: false,
  conversations_count: 0,
  created_at: "2026-09-08T00:00:00Z",
  updated_at: "2026-09-08T00:00:00Z",
  user_id: "test-user",
  credential_source: "platform",
  managed_setup: {
    subscription: "subscribed",
    webhook_override: "failed",
    registration: "registered",
    coexistence: false,
  },
  webhook_url:
    "https://nyxid.example/api/v1/webhooks/channel/whatsapp/managed-whatsapp",
};

export const managedBootstrap = {
  available: true,
  app_id: "111",
  embedded_signup_config_id: "222",
  graph_version: "v25.0",
  signup_version: "v4",
  feature_types: ["", "whatsapp_business_app_onboarding"],
  signup_extras: {
    "": {},
    whatsapp_business_app_onboarding: {
      featureType: "whatsapp_business_app_onboarding",
    },
  },
};

export async function mockDashboard(page: Page) {
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname.replace("/api/v1", "");
    let body: unknown = {};
    if (path === "/users/me")
      body = {
        id: "test-user",
        email: "test@example.com",
        display_name: "Test User",
        is_admin: true,
        is_active: true,
        email_verified: true,
        created_at: managedBot.created_at,
      };
    else if (path === "/channel-bots") body = { bots: [], total: 0 };
    else if (path === "/channel-bots/managed-whatsapp") body = managedBot;
    else if (path.includes("conversations"))
      body = { conversations: [], total: 0 };
    else if (path === "/orgs") body = { organizations: [], orgs: [] };
    else if (path === "/api-keys") body = { keys: [] };
    else if (path === "/nodes") body = { nodes: [] };
    else if (path === "/runtime-config")
      body = {
        api_base_url: "https://api.nyxid.example",
        release_integrity: {
          enabled: false,
          manifest_url: null,
          verification_ttl_secs: 300,
        },
      };
    await route.fulfill({ json: body });
  });
}
