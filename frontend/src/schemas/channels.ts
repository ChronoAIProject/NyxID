import { z } from "zod";

const channelPlatformSchema = z.enum([
  "telegram",
  "discord",
  "lark",
  "feishu",
  "slack",
]);

/**
 * Platform values accepted when reading a conversation back from the API.
 * Device conversations (HTTP Event Gateway, NyxID#221) use `"device"` and
 * have no backing `channel_bot_id`.
 */
export const conversationPlatformSchema = z.enum([
  "telegram",
  "discord",
  "lark",
  "feishu",
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
      .min(1, "Bot token is required")
      .max(512, "Bot token is too long"),
    label: z
      .string()
      .min(1, "Label is required")
      .max(128, "Label must be at most 128 characters"),
    app_id: z.string().max(256).optional(),
    app_secret: z.string().max(512).optional(),
    verification_token: z.string().max(512).optional(),
    encrypt_key: z.string().max(512).optional(),
    public_key: z.string().max(256).optional(),
    /** When set, create this bot under the given org (caller must be admin). */
    target_org_id: z.string().optional(),
  })
  .superRefine((data, ctx) => {
    const appId = data.app_id?.trim();
    const appSecret = data.app_secret?.trim();
    const verificationToken = data.verification_token?.trim();
    const publicKey = data.public_key?.trim();

    if (
      (data.platform === "lark" || data.platform === "feishu") &&
      !appId
    ) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "App ID is required for Lark/Feishu",
        path: ["app_id"],
      });
    }
    if (
      (data.platform === "lark" || data.platform === "feishu") &&
      !appSecret
    ) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "App Secret is required for Lark/Feishu",
        path: ["app_secret"],
      });
    }
    if (
      (data.platform === "lark" || data.platform === "feishu") &&
      !verificationToken
    ) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "Verification Token is required for Lark/Feishu",
        path: ["verification_token"],
      });
    }
    if (data.platform === "discord" && !publicKey) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "Public Key is required for Discord",
        path: ["public_key"],
      });
    }
    if (data.platform === "slack" && !appSecret) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "Signing Secret is required for Slack",
        path: ["app_secret"],
      });
    }
  });

export type CreateChannelBotFormData = z.infer<typeof createChannelBotSchema>;

export const updateChannelBotSchema = z.object({
  label: z.string().max(128, "Label must be at most 128 characters").optional(),
  verification_token: z.string().max(512).optional(),
  encrypt_key: z.string().max(512).optional(),
  app_id: z.string().max(256).optional(),
  app_secret: z.string().max(512).optional(),
});

export type UpdateChannelBotFormData = z.infer<typeof updateChannelBotSchema>;

export const createChannelConversationSchema = z.object({
  channel_bot_id: z.string().uuid("Invalid bot ID"),
  agent_api_key_id: z.string().uuid("Invalid API key ID"),
  platform_conversation_id: z.string().max(256).optional(),
  platform_conversation_type: conversationTypeSchema.optional(),
  platform_sender_id: z.string().max(256).optional(),
  default_agent: z.boolean().optional(),
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
  is_active: z.boolean().optional(),
});

export type UpdateChannelConversationFormData = z.infer<
  typeof updateChannelConversationSchema
>;
