import { z } from "zod";
import { createChannelBotSchema } from "@/schemas/channels";
import type { KeyInfo } from "@/types/keys";

export const GOOGLE_WORKSPACE_SLUG = "api-google";
export const GOOGLE_WORKSPACE_SCOPES = [
  "https://www.googleapis.com/auth/drive.file",
  "https://www.googleapis.com/auth/calendar",
] as const;

export const nyxbotSearchSchema = z.object({
  step: z
    .enum(["account", "source", "channel", "link"])
    .optional()
    .catch(undefined),
  channel: z.enum(["telegram", "whatsapp"]).optional().catch(undefined),
  provider_status: z.enum(["success", "error"]).optional().catch(undefined),
  status: z.enum(["success", "error"]).optional().catch(undefined),
});
export type NyxbotStep = NonNullable<
  z.infer<typeof nyxbotSearchSchema>["step"]
>;
export type NyxbotChannel = "telegram" | "whatsapp";

export const nyxbotProgressSchema = z.object({
  channel: z.enum(["telegram", "whatsapp"]).nullable().default(null),
  channelUrl: z.string().url().nullable().default(null),
  googleKeyId: z.string().max(128).nullable().default(null),
  botId: z.string().max(128).nullable().default(null),
  registrationId: z.string().max(128).nullable().default(null),
});
export type NyxbotProgress = z.infer<typeof nyxbotProgressSchema>;

// Syntax only; the submit flow obtains the bot identity from Telegram getMe.
export function createNyxbotTelegramSchema(messages: {
  required: string;
  invalid: string;
}) {
  return z.object({
    bot_token: z
      .string()
      .trim()
      .min(1, messages.required)
      .refine(
        (value) =>
          createChannelBotSchema.shape.bot_token.safeParse(value).success &&
          /^[0-9]+:[A-Za-z0-9_-]+$/.test(value),
        messages.invalid,
      ),
  });
}
export type NyxbotTelegramForm = z.infer<
  ReturnType<typeof createNyxbotTelegramSchema>
>;

export function isPersonalGoogleKey(key: KeyInfo): boolean {
  return (
    key.catalog_service_slug === GOOGLE_WORKSPACE_SLUG &&
    key.credential_source?.type !== "org"
  );
}

export function hasWorkspaceAuthorization(key: KeyInfo): boolean {
  if (
    !isPersonalGoogleKey(key) ||
    !key.is_active ||
    key.status !== "active" ||
    key.connection_status === "expired" ||
    key.credential_missing ||
    key.connected === false
  )
    return false;
  const scopes = new Set(key.granted_scopes ?? []);
  return (
    (scopes.has(GOOGLE_WORKSPACE_SCOPES[0]) ||
      scopes.has("https://www.googleapis.com/auth/drive")) &&
    scopes.has(GOOGLE_WORKSPACE_SCOPES[1])
  );
}

export function readNyxbotProgress(userId: string): NyxbotProgress {
  try {
    return nyxbotProgressSchema.parse(
      JSON.parse(sessionStorage.getItem(`nyxbot-onboarding:${userId}`) ?? "{}"),
    );
  } catch {
    return nyxbotProgressSchema.parse({});
  }
}

export function saveNyxbotProgress(
  userId: string,
  progress: NyxbotProgress,
): void {
  try {
    sessionStorage.setItem(
      `nyxbot-onboarding:${userId}`,
      JSON.stringify(nyxbotProgressSchema.parse(progress)),
    );
  } catch {
    // Storage can be disabled in an embedded browser. Server state remains authoritative.
  }
}
