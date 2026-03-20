import type {
  ProviderTokenMetadataValue,
  TelegramLoginCallbackRequest,
} from "@/types/api";

export interface TelegramCallbackPayload {
  readonly providerId: string;
  readonly data: TelegramLoginCallbackRequest;
}

export interface TelegramCallbackParseResult {
  readonly isTelegramCallback: boolean;
  readonly payload: TelegramCallbackPayload | null;
  readonly error: string | null;
}

export interface TelegramIdentity {
  readonly userId: string | null;
  readonly username: string | null;
  readonly firstName: string | null;
  readonly lastName: string | null;
  readonly displayName: string;
  readonly subtitle: string | null;
  readonly photoUrl: string | null;
}

function readOptional(search: URLSearchParams, key: string): string | undefined {
  const value = search.get(key);
  return value && value.length > 0 ? value : undefined;
}

function readMetadataString(
  metadata:
    | Readonly<Record<string, ProviderTokenMetadataValue>>
    | null
    | undefined,
  ...keys: readonly string[]
): string | null {
  for (const key of keys) {
    const value = metadata?.[key];
    if (typeof value === "string" && value.length > 0) {
      return value;
    }
    if (typeof value === "number" && Number.isFinite(value)) {
      return String(value);
    }
  }

  return null;
}

export function normalizeTelegramBotUsername(botUsername: string): string {
  return botUsername.replace(/^@+/, "");
}

export function parseTelegramCallbackSearch(
  search: URLSearchParams,
): TelegramCallbackParseResult {
  const hasTelegramParams =
    search.has("id") || search.has("hash") || search.has("auth_date");

  if (!hasTelegramParams) {
    return {
      isTelegramCallback: false,
      payload: null,
      error: null,
    };
  }

  const providerId = readOptional(search, "provider_id");
  const id = readOptional(search, "id");
  const firstName = readOptional(search, "first_name");
  const authDateRaw = readOptional(search, "auth_date");
  const hash = readOptional(search, "hash");

  if (!providerId) {
    return {
      isTelegramCallback: true,
      payload: null,
      error: "Missing provider ID in Telegram callback.",
    };
  }

  if (!id || !firstName || !authDateRaw || !hash) {
    return {
      isTelegramCallback: true,
      payload: null,
      error: "Incomplete Telegram login payload.",
    };
  }

  const authDate = Number(authDateRaw);
  if (!Number.isInteger(authDate) || authDate <= 0) {
    return {
      isTelegramCallback: true,
      payload: null,
      error: "Invalid Telegram auth timestamp.",
    };
  }

  return {
    isTelegramCallback: true,
    payload: {
      providerId,
      data: {
        id,
        first_name: firstName,
        last_name: readOptional(search, "last_name"),
        username: readOptional(search, "username"),
        photo_url: readOptional(search, "photo_url"),
        auth_date: authDate,
        hash,
      },
    },
    error: null,
  };
}

export function getTelegramIdentity(
  metadata:
    | Readonly<Record<string, ProviderTokenMetadataValue>>
    | null
    | undefined,
): TelegramIdentity | null {
  const userId = readMetadataString(
    metadata,
    "telegram_user_id",
    "telegram_id",
    "user_id",
    "id",
  );
  const username = readMetadataString(metadata, "username");
  const firstName = readMetadataString(metadata, "first_name");
  const lastName = readMetadataString(metadata, "last_name");
  const photoUrl = readMetadataString(metadata, "photo_url");

  if (
    userId === null &&
    username === null &&
    firstName === null &&
    lastName === null &&
    photoUrl === null
  ) {
    return null;
  }

  const fullName = [firstName, lastName].filter(Boolean).join(" ").trim();
  const normalizedUsername =
    username !== null ? username.replace(/^@+/, "") : null;
  const displayName =
    fullName ||
    (normalizedUsername !== null
      ? `@${normalizedUsername}`
      : userId !== null
        ? `ID ${userId}`
        : "Telegram");
  const subtitle =
    fullName && normalizedUsername !== null
      ? `@${normalizedUsername}`
      : userId !== null && displayName !== `ID ${userId}`
        ? `ID ${userId}`
        : null;

  return {
    userId,
    username: normalizedUsername,
    firstName,
    lastName,
    displayName,
    subtitle,
    photoUrl,
  };
}
