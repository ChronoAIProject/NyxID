import { create } from "zustand";
import { persist } from "zustand/middleware";
import {
  cmaSessionKeySchema,
  cmaSessionPreferencesSchema,
} from "@/schemas/cma-session";
import {
  getAssistantIdentityUserId,
  subscribeAssistantIdentity,
} from "@/lib/assistant/identity";

interface CmaSessionStore {
  readonly ownerId: string | null;
  readonly minimised: Record<string, boolean>;
  readonly setMinimised: (
    ownerId: string,
    key: string,
    minimised: boolean,
  ) => void;
  readonly clear: () => void;
}

export const useCmaSessionStore = create<CmaSessionStore>()(
  persist(
    (set) => ({
      ownerId: null,
      minimised: {},
      setMinimised: (ownerId, key, minimised) => {
        if (
          getAssistantIdentityUserId() !== ownerId ||
          !cmaSessionKeySchema.safeParse(key).success
        )
          return;
        set((state) => ({
          ownerId,
          minimised: Object.fromEntries([
            ...Object.entries(state.ownerId === ownerId ? state.minimised : {})
              .filter(([id]) => id !== key)
              .slice(-49),
            [key, minimised],
          ]),
        }));
      },
      clear: () => set({ ownerId: null, minimised: {} }),
    }),
    {
      name: "nyxid.cma-session",
      version: 2,
      partialize: ({ ownerId, minimised }) => ({ ownerId, minimised }),
      // The old manual pin no longer determines which sessions are shown.
      migrate: () => ({ ownerId: null, minimised: {} }),
      merge: (persisted, current) => {
        if (!persisted || typeof persisted !== "object") return current;
        const value = persisted as Record<string, unknown>;
        const minimised = cmaSessionPreferencesSchema.safeParse(
          value.minimised,
        );
        const ownerId =
          typeof value.ownerId === "string" ? value.ownerId : null;
        const currentOwner = getAssistantIdentityUserId();
        if (currentOwner && currentOwner !== ownerId) return current;
        return {
          ...current,
          ownerId,
          minimised:
            ownerId && minimised.success
              ? Object.fromEntries(Object.entries(minimised.data).slice(-50))
              : {},
        };
      },
    },
  ),
);

subscribeAssistantIdentity((next, previous) => {
  const { ownerId, clear } = useCmaSessionStore.getState();
  if (previous !== null || (ownerId !== null && ownerId !== next)) clear();
});
