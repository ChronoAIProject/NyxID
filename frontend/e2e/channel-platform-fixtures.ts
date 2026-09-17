import { platformFixtures } from "../src/test/fixtures/channel-platforms";

const names: Record<string, string> = {
  discord: "Discord",
  lark: "Lark",
  feishu: "Feishu",
  slack: "Slack",
  whatsapp: "WhatsApp",
  x: "X (Twitter)",
};

const labels: Record<string, string> = {
  bot_token: "Bot token",
  phone_number_id: "Phone Number ID",
  waba_id: "WhatsApp Business Account ID",
  app_id: "App ID",
  app_secret: "App Secret",
  verification_token: "Verification Token",
  encrypt_key: "Encrypt Key",
};

export const larkSetupInstructions =
  "In Lark/Feishu Event Subscriptions, copy the Verification Token from Security settings. " +
  "Encrypt Key is optional and should match the Encrypt Key field from the same panel " +
  "if you enabled encrypted callbacks.";

export const channelPlatforms = platformFixtures.map((descriptor) => ({
  ...descriptor,
  display_name: names[descriptor.platform] ?? descriptor.display_name,
  registration: {
    ...descriptor.registration,
    fields: descriptor.registration.fields.map((field) => ({
      ...field,
      label:
        descriptor.platform === "whatsapp" && field.name === "bot_token"
          ? "Access token"
          : field.name === "app_secret" && descriptor.platform === "whatsapp"
            ? "Meta App Secret"
            : field.name === "app_secret" && descriptor.platform === "slack"
              ? "Signing Secret"
              : (labels[field.name] ?? field.label),
      storage:
        field.name === "phone_number_id"
          ? "platform_bot_id"
          : field.name === "app_secret"
            ? "app_secret_encrypted"
            : field.storage,
    })),
    setup_instructions:
      descriptor.platform === "lark" || descriptor.platform === "feishu"
        ? [larkSetupInstructions]
        : descriptor.platform === "whatsapp"
          ? [
              "Find the Phone Number ID in WhatsApp > API Setup and the App Secret " +
                "in Meta App Dashboard > Basic settings.",
            ]
          : descriptor.registration.setup_instructions,
  },
}));
