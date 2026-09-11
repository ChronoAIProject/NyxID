import { z } from "zod";
import { ApiError, apiClient } from "@/lib/api-client";
import {
  AEVATAR_ORIGIN,
  AevatarAuthError,
  clearAevatarAuthorization,
  getAevatarAuthorization,
} from "@/lib/nyxbot-aevatar-auth";

export const AEVATAR_CHANNELS_PATH =
  "/proxy/s/aevatar/api/channels/registrations";
export const AEVATAR_WEBHOOK_BASE_URL = AEVATAR_ORIGIN;
export const NYXBOT_CHANNEL_SERVICE_SLUGS = [
  "api-google-workspace",
  "ornn-api",
  "chrono-llm-public",
] as const;

const telegramIdentitySchema = z.object({
  ok: z.literal(true),
  result: z.object({
    id: z.number().int().positive(),
    is_bot: z.literal(true),
    first_name: z.string().trim().min(1).max(128),
  }),
});

const registrationSchema = z.object({
  status: z.literal("accepted"),
  registration_id: z.string().min(1).max(128),
  platform: z.literal("telegram"),
  nyx_channel_bot_id: z.string().min(1).max(128),
});
export type NyxbotChannelRegistration = z.infer<typeof registrationSchema>;

const registrationStatusSchema = z.object({
  registration_id: z.string().min(1),
  nyx_channel_bot_id: z.string(),
  status: z.string(),
});

export class NyxbotChannelError extends Error {
  constructor(
    readonly code:
      | "tokenRejected"
      | "telegramUnavailable"
      | "channelAuthRequired"
      | "channelConsentRequired"
      | "channelRegistrationFailed",
  ) {
    super(code);
    this.name = "NyxbotChannelError";
  }
}

function botLabel(name: string): string {
  const suffix = "_nyxid_bot";
  const normalized = name.replace(/\s+/g, "_").replace(/(?:_nyxid_bot)+$/, "");
  const encoder = new TextEncoder();
  let prefix = "";
  let bytes = suffix.length;
  for (const character of normalized) {
    bytes += encoder.encode(character).length;
    if (bytes > 200) break; // NyxID validates the UTF-8 byte length.
    prefix += character;
  }
  return `${prefix}${suffix}`;
}

export async function getTelegramBotName(botToken: string): Promise<string> {
  if (!/^[0-9]+:[A-Za-z0-9_-]+$/.test(botToken))
    throw new NyxbotChannelError("tokenRejected");
  try {
    const response = await fetch(
      `https://api.telegram.org/bot${botToken}/getMe`,
      {
        method: "POST",
        credentials: "omit",
        cache: "no-store",
        referrerPolicy: "no-referrer",
        redirect: "error",
        signal: AbortSignal.timeout(15_000),
      },
    );
    if (response.status === 401 || response.status === 404)
      throw new NyxbotChannelError("tokenRejected");
    if (!response.ok) throw new NyxbotChannelError("telegramUnavailable");
    const payload: unknown = await response.json();
    const rejection = z
      .object({
        ok: z.literal(false),
        error_code: z.number(),
      })
      .safeParse(payload);
    if (rejection.success && [401, 404].includes(rejection.data.error_code))
      throw new NyxbotChannelError("tokenRejected");
    const result = telegramIdentitySchema.safeParse(payload);
    if (!result.success) throw new NyxbotChannelError("telegramUnavailable");
    return result.data.result.first_name;
  } catch (error) {
    // Fetch failures may include the credential-bearing Telegram URL. Never retain them.
    throw error instanceof NyxbotChannelError
      ? error
      : new NyxbotChannelError("telegramUnavailable");
  }
}

export async function registerNyxbotTelegram(
  botToken: string,
  serviceIds: readonly string[] = [],
): Promise<NyxbotChannelRegistration> {
  const token = botToken.trim();
  const botName = await getTelegramBotName(token);
  try {
    // Aevatar owns bot creation and relay provisioning; do not create a second NyxID bot.
    const authorization = await getAevatarAuthorization();
    const response = await apiClient<unknown>(AEVATAR_CHANNELS_PATH, {
      method: "POST",
      headers: { Authorization: authorization },
      body: {
        platform: "telegram",
        webhook_base_url: AEVATAR_WEBHOOK_BASE_URL,
        bot_token: token,
        label: botLabel(botName),
        service_ids: [...new Set(serviceIds)],
      },
      preserveSessionOn401: true,
      signal: AbortSignal.timeout(60_000),
    });
    return registrationSchema.parse(response);
  } catch (error) {
    if (error instanceof AevatarAuthError)
      throw new NyxbotChannelError(error.code);
    if (error instanceof ApiError && error.status === 401)
      clearAevatarAuthorization();
    throw new NyxbotChannelError(
      error instanceof ApiError &&
        (error.status === 401 || error.status === 403)
        ? "channelAuthRequired"
        : "channelRegistrationFailed",
    );
  }
}

export async function getNyxbotRegistrationStatus(registrationId: string) {
  try {
    const authorization = await getAevatarAuthorization();
    const response = await apiClient<unknown>(
      `${AEVATAR_CHANNELS_PATH}/${encodeURIComponent(registrationId)}/status`,
      { preserveSessionOn401: true, headers: { Authorization: authorization } },
    );
    return registrationStatusSchema.parse(response);
  } catch (error) {
    if (error instanceof ApiError && error.status === 401)
      clearAevatarAuthorization();
    throw error;
  }
}
