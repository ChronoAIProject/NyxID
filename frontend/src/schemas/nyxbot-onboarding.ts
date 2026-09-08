import { z } from "zod";
import { createChannelBotSchema } from "@/schemas/channels";
import type { KeyInfo } from "@/types/keys";

export const GOOGLE_WORKSPACE_SLUG = "api-google";
export const GOOGLE_WORKSPACE_SCOPES = [
  "https://www.googleapis.com/auth/drive.file",
  "https://www.googleapis.com/auth/calendar",
] as const;

export const nyxbotSearchSchema = z.object({
  channel: z.enum(["telegram", "whatsapp"]).optional().catch(undefined),
  status: z.enum(["success", "error"]).optional().catch(undefined),
});
export type NyxbotChannel = "telegram" | "whatsapp";

export const nyxbotProgressSchema = z.object({
  channel: z.enum(["telegram", "whatsapp"]).nullable().default(null),
  googleKeyId: z.string().max(128).nullable().default(null),
  botId: z.string().max(128).nullable().default(null),
});
export type NyxbotProgress = z.infer<typeof nyxbotProgressSchema>;

// Use the same token validation as the channel management form.
export const nyxbotTelegramSchema = z.object({
  bot_token: createChannelBotSchema.shape.bot_token,
});
export type NyxbotTelegramForm = z.infer<typeof nyxbotTelegramSchema>;

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
