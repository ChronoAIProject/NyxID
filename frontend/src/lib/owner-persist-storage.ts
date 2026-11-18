import { getAssistantIdentityUserId } from "./assistant/identity";
import { safePersistStorage } from "./safe-persist-storage";

export const accountStorageKey = (name: string, owner: string) =>
  `${name}:${encodeURIComponent(owner)}`;

/** Never write a departed tab's snapshot over another owner's record. */
export function ownerPersistStorage(
  ownerField = "ownerUserId",
  storage = safePersistStorage,
) {
  return {
    getItem(name: string): string | null {
      const owner = getAssistantIdentityUserId();
      const scoped = owner
        ? storage.getItem(accountStorageKey(name, owner))
        : null;
      if (scoped) return scoped;
      const legacy = storage.getItem(name);
      if (!legacy || !owner) return legacy;
      try {
        const stored = JSON.parse(legacy) as {
          state?: Record<string, unknown>;
        };
        if (stored.state?.[ownerField] === owner) {
          storage.setItem(accountStorageKey(name, owner), legacy);
          storage.removeItem(name);
          return legacy;
        }
      } catch {
        /* Invalid legacy data is discarded at confirmation. */
      }
      storage.removeItem(name);
      return null;
    },
    setItem(name: string, value: string): void {
      try {
        const stored = JSON.parse(value) as { state?: Record<string, unknown> };
        const owner = stored.state?.[ownerField];
        if (typeof owner === "string")
          storage.setItem(accountStorageKey(name, owner), value);
      } catch {
        /* Invalid or unavailable persistence cannot interrupt memory. */
      }
    },
    removeItem(name: string): void {
      const owner = getAssistantIdentityUserId();
      if (owner) storage.removeItem(accountStorageKey(name, owner));
    },
  };
}

export function removeOwnerPersistedState(
  name: string,
  owner: string | null,
): void {
  if (owner) safePersistStorage.removeItem(accountStorageKey(name, owner));
}
