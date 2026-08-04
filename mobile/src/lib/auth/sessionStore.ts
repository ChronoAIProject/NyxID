import * as SecureStore from "expo-secure-store";
import { decodeJwtSub } from "./jwt";

const ACCESS_TOKEN_KEY = "nyxid.auth.access_token";
const REFRESH_TOKEN_KEY = "nyxid.auth.refresh_token";
const ACCESS_TOKEN_EXPIRES_AT_KEY = "nyxid.auth.access_token_expires_at";
const USER_ID_KEY = "nyxid.auth.user_id";

export type StoredAuthSession = {
  accessToken: string;
  refreshToken?: string;
  accessTokenExpiresAt?: number;
  userId?: string;
};

export async function loadStoredAuthSession(): Promise<StoredAuthSession | null> {
  const [accessToken, refreshToken, accessTokenExpiresAtRaw, storedUserId] = await Promise.all([
    SecureStore.getItemAsync(ACCESS_TOKEN_KEY),
    SecureStore.getItemAsync(REFRESH_TOKEN_KEY),
    SecureStore.getItemAsync(ACCESS_TOKEN_EXPIRES_AT_KEY),
    SecureStore.getItemAsync(USER_ID_KEY),
  ]);
  const parsedExpiresAt = accessTokenExpiresAtRaw ? Number(accessTokenExpiresAtRaw) : NaN;
  const accessTokenExpiresAt = Number.isFinite(parsedExpiresAt) ? parsedExpiresAt : undefined;

  if (accessToken) {
    // Read-only: derive from the JWT for pre-feature sessions, but never
    // write back from here. `persistAuthSession` is the single writer, so
    // we can't race with `clearStoredAuthSession` during sign-out.
    // Existing users get the dedicated key populated on their next login
    // or token refresh; until then, the derive path covers them.
    const userId = storedUserId ?? decodeJwtSub(accessToken);
    return {
      accessToken,
      refreshToken: refreshToken ?? undefined,
      accessTokenExpiresAt,
      userId,
    };
  }

  // Access token is missing: clean up any orphan keys from partial writes
  // or interrupted sign-out races. The previous version only cleared when
  // `refreshToken` was the orphan and left `user_id` / `expires_at` behind.
  if (refreshToken || storedUserId || accessTokenExpiresAtRaw) {
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

  const userId = session.userId ?? decodeJwtSub(session.accessToken);
  if (userId) {
    await SecureStore.setItemAsync(USER_ID_KEY, userId);
  } else {
    await SecureStore.deleteItemAsync(USER_ID_KEY);
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
    SecureStore.deleteItemAsync(USER_ID_KEY),
  ]);
}
