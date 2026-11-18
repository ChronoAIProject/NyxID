import { getAssistantIdentityUserId } from "./assistant/identity";
import { safePersistStorage } from "./safe-persist-storage";

const OWNER_KEY = "nyxid:resource-preferences-owner";
let boundOwner: string | null = null;
const prefix = (owner: string) => `nyxid:resource:${encodeURIComponent(owner)}:`;
export function accountResourcePreferenceKey(name: string): string {
  const owner = getAssistantIdentityUserId();
  return owner ? prefix(owner) + name : `nyxid:resource:anonymous:${name}`;
}

/** Migrate legacy preferences once; departing cleanup touches only its own prefix. */
export function bindAccountResourcePreferences(userId: string | null): void {
  try {
    const legacyOwner = localStorage.getItem(OWNER_KEY);
    for (let index = localStorage.length - 1; index >= 0; index--) {
      const key = localStorage.key(index);
      if (!key) continue;
      if (boundOwner && boundOwner !== userId && key.startsWith(prefix(boundOwner))) {
        safePersistStorage.removeItem(key);
      } else if (key === "nyxid:last-picked-agent-key" || key.startsWith("nyxid:desktop-collapsed:")) {
        if (userId && legacyOwner === userId) {
          const destination = prefix(userId) + key;
          // A newer scoped choice always wins over a legacy snapshot.
          if (localStorage.getItem(destination) === null) safePersistStorage.setItem(destination, localStorage.getItem(key)!);
        }
        safePersistStorage.removeItem(key);
      }
    }
    if (userId) safePersistStorage.setItem(OWNER_KEY, userId);
    else safePersistStorage.removeItem(OWNER_KEY);
  } catch { /* Browser preferences can be unavailable. */ }
  boundOwner = userId;
}
