import * as SecureStore from "expo-secure-store";

const ACCESS_TOKEN_KEY = "nyxid.auth.access_token";
const REFRESH_TOKEN_KEY = "nyxid.auth.refresh_token";
const ACCESS_TOKEN_EXPIRES_AT_KEY = "nyxid.auth.access_token_expires_at";

export type StoredAuthSession = {
  accessToken: string;
  refreshToken?: string;
  accessTokenExpiresAt?: number;
};

export async function loadStoredAuthSession(): Promise<StoredAuthSession | null> {
  const [accessToken, refreshToken, accessTokenExpiresAtRaw] = await Promise.all([
    SecureStore.getItemAsync(ACCESS_TOKEN_KEY),
    SecureStore.getItemAsync(REFRESH_TOKEN_KEY),
    SecureStore.getItemAsync(ACCESS_TOKEN_EXPIRES_AT_KEY),
  ]);
  const parsedExpiresAt = accessTokenExpiresAtRaw ? Number(accessTokenExpiresAtRaw) : NaN;
  const accessTokenExpiresAt = Number.isFinite(parsedExpiresAt) ? parsedExpiresAt : undefined;

  if (accessToken) {
    return {
      accessToken,
      refreshToken: refreshToken ?? undefined,
      accessTokenExpiresAt,
    };
  }

  if (refreshToken) {
    await clearStoredAuthSession();
  }

  return null;
}

export async function persistAuthSession(session: StoredAuthSession): Promise<void> {
  await SecureStore.setItemAsync(ACCESS_TOKEN_KEY, session.accessToken);

  if (typeof session.accessTokenExpiresAt === "number" && Number.isFinite(session.accessTokenExpiresAt)) {
    await SecureStore.setItemAsync(
      ACCESS_TOKEN_EXPIRES_AT_KEY,
      String(Math.floor(session.accessTokenExpiresAt))
    );
  } else {
    await SecureStore.deleteItemAsync(ACCESS_TOKEN_EXPIRES_AT_KEY);
  }

  if (session.refreshToken) {
    await SecureStore.setItemAsync(REFRESH_TOKEN_KEY, session.refreshToken);
    return;
  }

  await SecureStore.deleteItemAsync(REFRESH_TOKEN_KEY);
}

export async function clearStoredAuthSession(): Promise<void> {
  await Promise.all([
    SecureStore.deleteItemAsync(ACCESS_TOKEN_KEY),
    SecureStore.deleteItemAsync(REFRESH_TOKEN_KEY),
    SecureStore.deleteItemAsync(ACCESS_TOKEN_EXPIRES_AT_KEY),
  ]);
}
