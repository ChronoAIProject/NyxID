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
  readonly pinned: Record<string, boolean>;
  readonly setPinned: (ownerId: string, key: string, pinned: boolean) => void;
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
      pinned: {},
      setPinned: (ownerId, key, pinned) => {
        if (
          getAssistantIdentityUserId() !== ownerId ||
          !cmaSessionKeySchema.safeParse(key).success
        )
          return;
        set((state) => ({
          ownerId,
          pinned: Object.fromEntries([
            ...Object.entries(state.ownerId === ownerId ? state.pinned : {})
              .filter(([id, value]) => id !== key && value)
              .slice(-49),
            ...(pinned ? [[key, true] as const] : []),
          ]),
          minimised: state.ownerId === ownerId ? state.minimised : {},
        }));
      },
      setMinimised: (ownerId, key, minimised) => {
        if (
          getAssistantIdentityUserId() !== ownerId ||
          !cmaSessionKeySchema.safeParse(key).success
        )
          return;
        set((state) => ({
          ownerId,
          pinned: state.ownerId === ownerId ? state.pinned : {},
          minimised: Object.fromEntries([
            ...Object.entries(state.ownerId === ownerId ? state.minimised : {})
              .filter(([id]) => id !== key)
              .slice(-49),
            [key, minimised],
          ]),
        }));
      },
      clear: () => set({ ownerId: null, minimised: {}, pinned: {} }),
    }),
    {
      name: "nyxid.cma-session",
      version: 3,
      partialize: ({ ownerId, minimised, pinned }) => ({
        ownerId,
        minimised,
        pinned,
      }),
      // Version-1 pins contain unverified inventory metadata and are discarded.
      migrate: (value, version) => {
        const old = value as { ownerId?: unknown; minimised?: unknown } | null;
        const minimised = cmaSessionPreferencesSchema.safeParse(old?.minimised);
        return version === 2
          ? {
              ownerId: typeof old?.ownerId === "string" ? old.ownerId : null,
              minimised: minimised.success ? minimised.data : {},
              pinned: {},
            }
          : { ownerId: null, minimised: {}, pinned: {} };
      },
      merge: (persisted, current) => {
        if (!persisted || typeof persisted !== "object") return current;
        const value = persisted as Record<string, unknown>;
        const pinned = cmaSessionPreferencesSchema.safeParse(value.pinned);
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
          pinned:
            ownerId && pinned.success
              ? Object.fromEntries(
                  Object.entries(pinned.data)
                    .filter(([, value]) => value)
                    .slice(-50),
                )
              : {},
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
