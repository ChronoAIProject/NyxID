import { z } from "zod";
import { ApiError, apiClient } from "@/lib/api-client";

export const AEVATAR_CHANNELS_PATH =
  "/proxy/s/aevatar/api/channels/registrations";
export const AEVATAR_WEBHOOK_BASE_URL =
  "https://aevatar-console-backend-api.aevatar.ai";

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
      | "channelRegistrationFailed",
  ) {
    super(code);
    this.name = "NyxbotChannelError";
  }
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
): Promise<NyxbotChannelRegistration> {
  const token = botToken.trim();
  const botName = await getTelegramBotName(token);
  try {
    // Reuse NyxID's session-authenticated proxy. Aevatar must support the proxy's
    // authentication contract; never extract browser cookies or mint CLI credentials.
    // Aevatar owns bot creation and relay provisioning; do not create a second NyxID bot.
    const response = await apiClient<unknown>(AEVATAR_CHANNELS_PATH, {
      method: "POST",
      body: {
        platform: "telegram",
        webhook_base_url: AEVATAR_WEBHOOK_BASE_URL,
        bot_token: token,
        label: botName.replace(/\s+/g, "_"),
      },
      preserveSessionOn401: true,
      signal: AbortSignal.timeout(60_000),
    });
    return registrationSchema.parse(response);
  } catch (error) {
    throw new NyxbotChannelError(
      error instanceof ApiError &&
        (error.status === 401 || error.status === 403)
        ? "channelAuthRequired"
        : "channelRegistrationFailed",
    );
  }
}

export async function getNyxbotRegistrationStatus(registrationId: string) {
  const response = await apiClient<unknown>(
    `${AEVATAR_CHANNELS_PATH}/${encodeURIComponent(registrationId)}/status`,
    { preserveSessionOn401: true },
  );
  return registrationStatusSchema.parse(response);
}
