import { z } from "zod";

export const telegramNewRequestSchema = z.object({
  id: z.string().uuid(),
  status: z.enum([
    "waiting_telegram",
    "waiting_bot",
    "waiting_consent",
    "ready",
    "provisioning",
    "connected",
    "cancelled",
    "expired",
    "suspended",
  ]),
  revision: z.number().int(),
  label: z.string(),
  owner_user_id: z.string(),
  expires_at: z.string(),
  telegram_bot_id: z.string().nullable(),
  bot_username: z.string().nullable(),
  channel_bot_id: z.string().nullable(),
  auto_connect: z.boolean().optional(),
  connection_error: z.string().nullable().optional(),
});

export const telegramNewConfigSchema = z.object({
  available: z.boolean(),
  manager_username: z.string().nullable(),
  request: telegramNewRequestSchema.nullable(),
});

export const telegramNewLaunchSchema = z.object({
  request: telegramNewRequestSchema,
  launch_url: z
    .string()
    .url()
    .refine((value) => {
      const url = new URL(value);
      return (
        url.origin === "https://t.me" && /^\/[A-Za-z0-9_]+$/.test(url.pathname)
      );
    }, "Invalid Telegram launch URL"),
});

export const telegramNewBeginSchema = z.object({
  label: z
    .string()
    .trim()
    .min(1, "Enter a label before creating your bot")
    .max(128),
  target_org_id: z.string().uuid().optional(),
  auto_connect: z.boolean().optional(),
});

export type TelegramNewRequest = z.infer<typeof telegramNewRequestSchema>;

export const telegramClaimCodeSchema = z.string().max(64)
  .transform((value) => value.replace(/[-\s]/g, "").toUpperCase())
  .pipe(z.string().regex(/^[A-HJ-NP-Z2-9]{20}$/, "Enter the claim code from your private Telegram setup chat"));

export const telegramClaimPreviewSchema = z.object({
  bot_username: z.string(),
  expires_at: z.string().datetime({ offset: true }),
});
