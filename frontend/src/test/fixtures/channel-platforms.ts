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
  ...(["lark", "feishu"] as const).map((p) => platformFixture(p, [field("app_id", false, true, true), field("app_secret", true, true, true), field("verification_token", true, true, true), field("encrypt_key", true, false, true)])),
  platformFixture("slack", [field("bot_token", true), field("app_secret", true, true, true)]),
  { ...platformFixture("whatsapp", [field("bot_token", true, true, true), field("phone_number_id"), field("app_secret", true, true, true), field("waba_id", false, false)]),
    managed_onboarding: { flow: "meta_embedded_signup", provider: "meta", bootstrap_fields: [], completion_fields: [] } },
  { ...platformFixture("x", []), activities: [
    {kind: "dm", label: "Direct message", subscription: "dm", subscription_label: "Direct messages", description: "Receive unencrypted direct messages and reply privately.", content_availability: "available", reply_supported: true},
    {kind: "encrypted_chat", label: "Encrypted chat", subscription: "chat", subscription_label: "Encrypted chat notifications", description: "Count encrypted messages and notify compatible agents. Message text and replies are unavailable.", content_availability: "encrypted", reply_supported: false},
    {kind: "mention", label: "Mention", subscription: "mentions", subscription_label: "Mentions", description: "Receive posts mentioning your account. Agent replies are public.", content_availability: "available", reply_supported: true},
    {kind: "reply", label: "Reply to my post", subscription: "replies", subscription_label: "Replies to my posts", description: "Receive direct replies to your posts. Agent replies are public.", content_availability: "available", reply_supported: true},
    {kind: "post", label: "My post", subscription: "posts", subscription_label: "My posts", description: "Count posts authored by your account and notify compatible agents without automatic replies.", content_availability: "metadata_only", reply_supported: false},
  ], managed_only: true, ingestion: { mode: "poll", min_interval_secs: 60 },
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
