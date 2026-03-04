import * as SecureStore from "expo-secure-store";

const ACCESS_TOKEN_KEY = "nyxid.auth.access_token";
const REFRESH_TOKEN_KEY = "nyxid.auth.refresh_token";

export type StoredAuthSession = {
  accessToken: string;
  refreshToken?: string;
};

export async function loadStoredAuthSession(): Promise<StoredAuthSession | null> {
  const [accessToken, refreshToken] = await Promise.all([
    SecureStore.getItemAsync(ACCESS_TOKEN_KEY),
    SecureStore.getItemAsync(REFRESH_TOKEN_KEY),
  ]);

  if (accessToken) {
    return {
      accessToken,
      refreshToken: refreshToken ?? undefined,
    };
  }

  if (refreshToken) {
    await clearStoredAuthSession();
  }

  return null;
}

export async function persistAuthSession(session: StoredAuthSession): Promise<void> {
  await SecureStore.setItemAsync(ACCESS_TOKEN_KEY, session.accessToken);

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
  ]);
}
