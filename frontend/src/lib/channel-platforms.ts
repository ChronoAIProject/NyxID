import type { ChannelBotDetail, ChannelPlatform, CreateChannelBotRequest } from "@/types/channels";

export type ChannelCredentialField =
  | "bot_token" | "app_id" | "app_secret" | "verification_token"
  | "encrypt_key" | "public_key" | "phone_number_id" | "waba_id";

export interface ChannelFieldDescriptor {
  readonly name: ChannelCredentialField;
  readonly label: string;
  readonly secret?: boolean;
  readonly required?: boolean;
  readonly patchable?: boolean;
  readonly numeric?: boolean;
  readonly configuredKey?: keyof Pick<ChannelBotDetail, "app_secret_configured" | "lark_verification_token_configured" | "lark_encrypt_key_configured">;
  readonly hint?: string;
}

const tokenField: ChannelFieldDescriptor = { name: "bot_token", label: "Bot Token", secret: true, required: true };
const larkFields: readonly ChannelFieldDescriptor[] = [
  tokenField,
  { name: "app_id", label: "App ID", required: true, patchable: true },
  { name: "app_secret", label: "App Secret", secret: true, required: true, patchable: true, configuredKey: "app_secret_configured" },
  { name: "verification_token", label: "Verification Token", secret: true, required: true, patchable: true, configuredKey: "lark_verification_token_configured", hint: "Event Subscriptions > Security in the Lark/Feishu console." },
  { name: "encrypt_key", label: "Encrypt Key", secret: true, patchable: true, configuredKey: "lark_encrypt_key_configured", hint: "Optional. Required only when encrypted callbacks are enabled in the platform console." },
];

const larkSetupNote = {
  title: "Lark webhook verification",
  text: "In Lark/Feishu Event Subscriptions, copy the Verification Token from Security settings. Encrypt Key is optional and should match the Encrypt Key field from the same panel if you enabled encrypted callbacks.",
};

// Mirrors each backend adapter's registration descriptor. Rendering, validation,
// and payload selection use the same fields so hidden credentials never cross platforms.
export const CHANNEL_PLATFORMS: Record<ChannelPlatform, { readonly label: string; readonly fields: readonly ChannelFieldDescriptor[]; readonly webhookDocs: string; readonly managedFlow?: "meta_embedded_signup"; readonly advancedLabel?: string; readonly setupNote?: { readonly title: string; readonly text: string } }> = {
  telegram: { label: "Telegram", fields: [tokenField], webhookDocs: "https://core.telegram.org/bots/api#setwebhook" },
  discord: { label: "Discord", fields: [tokenField, { name: "public_key", label: "Public Key", required: true }], webhookDocs: "https://discord.com/developers/docs/interactions/receiving-and-responding" },
  lark: { label: "Lark", fields: larkFields, setupNote: larkSetupNote, webhookDocs: "https://open.larksuite.com/document/server-docs/event-subscription-guide/event-subscription-configure-/request-url-configuration-case" },
  feishu: { label: "Feishu", fields: larkFields, setupNote: larkSetupNote, webhookDocs: "https://open.feishu.cn/document/server-docs/event-subscription-guide/event-subscription-configure-/request-url-configuration-case" },
  slack: { label: "Slack", fields: [tokenField, { name: "app_secret", label: "Signing Secret", secret: true, required: true, patchable: true, configuredKey: "app_secret_configured", hint: "Basic Information > App Credentials in Slack app settings." }], webhookDocs: "https://api.slack.com/apis/events-api" },
  whatsapp: {
    managedFlow: "meta_embedded_signup",
    advancedLabel: "Advanced: use your own Meta app",
    label: "WhatsApp",
    setupNote: { title: "Meta Cloud API credentials", text: "Find the Phone Number ID in WhatsApp > API Setup and the App Secret in Meta App Dashboard > Basic settings." },
    webhookDocs: "https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/create-webhook-endpoint",
    fields: [
      { ...tokenField, label: "Access Token", patchable: true, hint: "Permanent System User token for the WhatsApp Business Platform (Meta Cloud API)." },
      { name: "phone_number_id", label: "Phone Number ID", required: true, numeric: true, hint: "Meta phone number identifier, not the display phone number or App ID." },
      { name: "app_secret", label: "Meta App Secret", secret: true, required: true, patchable: true, configuredKey: "app_secret_configured" },
      { name: "waba_id", label: "WhatsApp Business Account ID", numeric: true, hint: "Optional WABA ID." },
    ],
  },
};

export function channelBotRegistrationPayload(data: CreateChannelBotRequest): CreateChannelBotRequest {
  const fields = Object.fromEntries(CHANNEL_PLATFORMS[data.platform].fields.flatMap(({ name }) => {
    const value = data[name]?.trim();
    return value ? [[name, value]] : [];
  }));
  return { platform: data.platform, label: data.label.trim(), bot_token: data.bot_token.trim(), target_org_id: data.target_org_id || undefined, ...fields };
}

export type ChannelMutableField = Exclude<ChannelCredentialField, "phone_number_id" | "waba_id" | "public_key">;

export function editableChannelFields(platform: ChannelPlatform) {
  return CHANNEL_PLATFORMS[platform].fields.filter(
    (field): field is ChannelFieldDescriptor & { readonly name: ChannelMutableField } => field.patchable === true,
  );
}
