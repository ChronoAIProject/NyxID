import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  getAuthAccountId,
  subscribeAuthAccount,
} from "../../lib/auth/authAccount";
import {
  SERVICE_OAUTH_TTL_MS,
  pendingOAuthSchema,
  type PendingServiceOAuth,
  type OAuthReturn,
} from "./serviceOAuthModel";

export const SERVICE_OAUTH_STORAGE_KEY = "nyxid.pending-service-oauth.v1";
let writes = Promise.resolve();
let revision = 0;
let account = getAuthAccountId();
let hint: OAuthReturn | undefined;
const listeners = new Set<() => void>();
const notify = () => {
  for (const listener of listeners) listener();
};
const serialize = (write: () => Promise<void>) => {
  writes = writes.catch(() => {}).then(write);
  return writes;
};

subscribeAuthAccount(() => {
  const next = getAuthAccountId();
  if (account !== null && next !== account) {
    revision++;
    hint = undefined;
    void serialize(() =>
      AsyncStorage.removeItem(SERVICE_OAUTH_STORAGE_KEY),
    ).catch(() => {});
    notify();
  }
  account = next;
});

export function subscribeServiceOAuth(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function recordServiceOAuthReturn(value: OAuthReturn) {
  hint = value;
  notify();
}
export function serviceOAuthReturnHint() {
  return hint;
}
export function serviceOAuthRevision() {
  return revision;
}

export async function loadServiceOAuth(): Promise<PendingServiceOAuth | null> {
  const expectedRevision = revision;
  const expectedAccount = getAuthAccountId();
  await writes.catch(() => {});
  if (!expectedAccount) return null;
  const raw = await AsyncStorage.getItem(SERVICE_OAUTH_STORAGE_KEY);
  if (revision !== expectedRevision || expectedAccount !== getAuthAccountId())
    return null;
  let parsed;
  try {
    parsed = pendingOAuthSchema.safeParse(JSON.parse(raw ?? "null"));
  } catch {
    parsed = pendingOAuthSchema.safeParse(null);
  }
  if (
    !parsed.success ||
    parsed.data.accountId !== expectedAccount ||
    parsed.data.expiresAt <= Date.now() ||
    parsed.data.expiresAt - parsed.data.startedAt !== SERVICE_OAUTH_TTL_MS ||
    parsed.data.startedAt > Date.now()
  ) {
    if (revision === expectedRevision) await clearServiceOAuth();
    return null;
  }
  return parsed.data;
}

export async function saveServiceOAuth(attempt: PendingServiceOAuth) {
  if (getAuthAccountId() !== attempt.accountId) return;
  revision++;
  hint = undefined;
  await serialize(async () => {
    if (getAuthAccountId() === attempt.accountId)
      await AsyncStorage.setItem(
        SERVICE_OAUTH_STORAGE_KEY,
        JSON.stringify(attempt),
      );
  });
}

export async function clearServiceOAuth(attempt?: PendingServiceOAuth) {
  await serialize(async () => {
    if (attempt) {
      if (getAuthAccountId() !== attempt.accountId) return;
      const raw = await AsyncStorage.getItem(SERVICE_OAUTH_STORAGE_KEY);
      // A late browser result or expiry timer must not clear a newer attempt.
      if (raw !== JSON.stringify(attempt)) return;
    }
    revision++;
    hint = undefined;
    await AsyncStorage.removeItem(SERVICE_OAUTH_STORAGE_KEY);
  });
}
