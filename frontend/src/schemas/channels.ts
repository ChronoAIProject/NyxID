import { z } from "zod";
import type { ChannelPlatformDescriptor } from "@/types/channels";
import type { ChannelCredentialField } from "@/lib/channel-platforms";

const channelPlatformSchema = z.string().regex(/^[a-z][a-z0-9-]{0,63}$/, "Invalid platform");

/**
 * Platform values accepted when reading a conversation back from the API.
 * Device conversations (HTTP Event Gateway, NyxID#221) use `"device"` and
 * have no backing `channel_bot_id`.
 */
export const conversationPlatformSchema = z.enum([
  "telegram",
  "telegram-new",
  "discord",
  "lark",
  "feishu",
  "slack",
  "whatsapp",
  "x",
  "aurinko",
  "device",
]);

export type ConversationPlatform = z.infer<typeof conversationPlatformSchema>;

const conversationTypeSchema = z.enum([
  "private",
  "group",
  "channel",
  "device",
]);

export const createChannelBotSchema = z
  .object({
    platform: channelPlatformSchema,
    bot_token: z
      .string()
      .max(512, "Bot token is too long"),
    label: z
      .string()
      .min(1, "Label is required")
      .max(128, "Label must be at most 128 characters")
      .refine((v) => v.trim().length > 0, "Label must not be blank"),
    app_id: z.string().max(256).optional(),
    app_secret: z.string().max(512).optional(),
    verification_token: z.string().max(512).optional(),
    encrypt_key: z.string().max(512).optional(),
    public_key: z.string().max(256).optional(),
    phone_number_id: z.string().max(32).regex(/^[0-9]*$/, "Use a numeric identifier").optional(),
    waba_id: z.string().max(32).regex(/^[0-9]*$/, "Use a numeric identifier").optional(),
    /** When set, create this bot under the given org (caller must be admin). */
    target_org_id: z.string().optional(),
  }).catchall(z.string().optional());

export function buildCreateChannelBotSchema(platforms: readonly ChannelPlatformDescriptor[]) {
  return createChannelBotSchema.superRefine((data, ctx) => {
    const descriptor = platforms.find((p) => p.platform === data.platform && p.enabled);
    if (!descriptor) {
      ctx.addIssue({ code: "custom", message: "Choose an enabled platform", path: ["platform"] });
      return;
    }
    if (descriptor.managed_only) return;
    for (const field of descriptor.registration.fields) {
      const value = data[field.name as ChannelCredentialField]?.trim();
      if (field.required && !value) {
        ctx.addIssue({ code: "custom", message: `${field.label} is required`, path: [field.name] });
      }
      if (value && value.length > (field.secret ? 512 : 256)) {
        ctx.addIssue({ code: "custom", message: `${field.label} is too long`, path: [field.name] });
      }
    }
  });
}

export type CreateChannelBotFormData = z.infer<typeof createChannelBotSchema>;

export const updateChannelBotSchema = z.object({
  bot_token: z.string().max(512).optional(),
  label: z
    .string()
    .min(1, "Label is required")
    .max(128, "Label must be at most 128 characters")
    .refine((v) => v.trim().length > 0, "Label must not be blank")
    .optional(),
  verification_token: z.string().max(512).optional(),
  encrypt_key: z.string().max(512).optional(),
  app_id: z.string().max(256).optional(),
  app_secret: z.string().max(512).optional(),
}).catchall(z.string().optional());

export type UpdateChannelBotFormData = z.infer<typeof updateChannelBotSchema>;

export const createChannelConversationSchema = z.object({
  channel_bot_id: z.string().uuid("Invalid bot ID"),
  agent_api_key_id: z.string().uuid("Invalid API key ID"),
  platform_conversation_id: z.string().max(256).optional(),
  platform_conversation_type: conversationTypeSchema.optional(),
  platform_sender_id: z.string().max(256).optional(),
  default_agent: z.boolean().optional(),
  allow_agent_initiated: z.boolean().optional(),
  /** When set, create this conversation under the given org (caller must be admin). */
  target_org_id: z.string().optional(),
});

export type CreateChannelConversationFormData = z.infer<
  typeof createChannelConversationSchema
>;

/**
 * Device conversations (HTTP Event Gateway, NyxID#221) are not backed by a
 * bot. They require an explicit `platform_conversation_id` (the logical
 * device channel name, e.g. `household-camera`) and an agent API key.
 */
export const createDeviceConversationSchema = z.object({
  platform_conversation_id: z
    .string()
    .min(1, "Device channel ID is required")
    .max(256, "Device channel ID must be at most 256 characters"),
  agent_api_key_id: z.string().uuid("Invalid API key ID"),
  platform_conversation_type: z.string().max(64).optional(),
  target_org_id: z.string().optional(),
});

export type CreateDeviceConversationFormData = z.infer<
  typeof createDeviceConversationSchema
>;

export const updateChannelConversationSchema = z.object({
  agent_api_key_id: z.string().uuid("Invalid API key ID").optional(),
  default_agent: z.boolean().optional(),
  allow_agent_initiated: z.boolean().optional(),
  is_active: z.boolean().optional(),
});

export type UpdateChannelConversationFormData = z.infer<
  typeof updateChannelConversationSchema
>;

export const channelInitiatedSettingsSchema = z.object({
  allow_agent_initiated: z.boolean(),
});

export const channelTestMessageSchema = z.object({
  text: z.string().trim().min(1, "Enter a message"),
});

export type ChannelInitiatedSettingsFormData = z.infer<
  typeof channelInitiatedSettingsSchema
>;
export type ChannelTestMessageFormData = z.infer<
  typeof channelTestMessageSchema
>;
