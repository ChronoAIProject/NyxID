import type { ChannelPlatformDescriptor, ChannelRegistrationField } from "@/types/channels";
function field(name: string, secret = false, required = true, patchable = false, hint: string | null = null): ChannelRegistrationField {
  return { name, hint, label: name === "bot_token" ? "Bot token" : name, secret, required, patchable, clearable: false, storage: name,
    webhook_secret: false, platform_fallback: null };
}
export function platformFixture(platform: ChannelPlatformDescriptor["platform"], fields: readonly ChannelRegistrationField[] = [field("bot_token", true)]): ChannelPlatformDescriptor {
  return { platform, display_name: platform === "telegram" ? "Telegram bot token" : platform === "telegram-new" ? "Telegram" : platform,
    enabled: true, managed_only: false, managed_only_message: "Managed onboarding required", ingestion: { mode: "webhook" },
    registration: { fields, token_fields: ["bot_token"], extra_fields: [], required_suffix: "", automatic_webhook: true,
      webhook_ingestion: true, webhook_secret_label: null, create_response_status: "active", setup_instructions: [] },
    managed_onboarding: null, platform_credentials: null,
    capabilities: { initiated_send: true, reply_to: true, thread: true, edit: true, media: { inbound: ["image", "file"], outbound: ["image", "file"] } },
    webhook_path: `/api/v1/webhooks/channel/${platform}/{bot_id}` };
}
export const platformFixtures: ChannelPlatformDescriptor[] = [
  platformFixture("telegram"), platformFixture("telegram-new", []),
  platformFixture("discord", [field("bot_token", true), field("public_key")]),
  ...(["lark", "feishu"] as const).map((p) => platformFixture(p, [field("bot_token", true), field("app_id", false, true, true), field("app_secret", true, true, true), field("verification_token", true, true, true), field("encrypt_key", true, false, true)])),
  platformFixture("slack", [field("bot_token", true), field("app_secret", true, true, true)]),
  { ...platformFixture("whatsapp", [field("bot_token", true, true, true), field("phone_number_id"), field("app_secret", true, true, true), field("waba_id", false, false)]),
    managed_onboarding: { flow: "meta_embedded_signup", provider: "meta", bootstrap_fields: [], completion_fields: [] } },
  { ...platformFixture("x", []), managed_only: true, ingestion: { mode: "poll", min_interval_secs: 60 },
    managed_onboarding: { flow: "oauth_connection", provider: "twitter", bootstrap_fields: [], completion_fields: [] } },
  { ...platformFixture("aurinko", [field("bot_token", true, true, true), field("app_secret", true, true, true)]),
    display_name: "Aurinko Email",
    capabilities: { initiated_send: false, reply_to: true, thread: false, edit: false, media: { inbound: [], outbound: [] } },
    registration: {
      ...platformFixture("aurinko").registration,
      fields: [field("bot_token", true, true, true), field("app_secret", true, true, true)],
      setup_instructions: ["AI Service and channel bot tokens are stored separately."],
    },
  },
];
