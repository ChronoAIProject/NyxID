import { create } from "zustand";
import { persist } from "zustand/middleware";
import { parseBuildVersion, type BuildVersion } from "@/lib/build-version";

export const UPDATE_REMINDER_DELAY = 2 * 60 * 60_000;

interface PendingUpdate {
  readonly build: BuildVersion;
  readonly detectedAt: number;
}

interface BuildUpdateState {
  readonly pending: PendingUpdate | null;
  readonly record: (build: BuildVersion) => void;
  readonly clear: () => void;
}

export const useBuildUpdateStore = create<BuildUpdateState>()(
  persist(
    (set) => ({
      pending: null,
      record: (build) => set((state) => ({
        pending: { build, detectedAt: state.pending?.detectedAt ?? Date.now() },
      })),
      clear: () => set({ pending: null }),
    }),
    {
      name: "nyxid.build-update",
      partialize: (state) => ({ pending: state.pending }),
      merge: (persisted, current) => {
        const saved = (persisted as { pending?: Partial<PendingUpdate> } | null)?.pending;
        const build = parseBuildVersion(saved?.build);
        const detectedAt = saved?.detectedAt;
        return {
          ...current,
          pending: build && typeof detectedAt === "number" && Number.isFinite(detectedAt) && detectedAt >= 0
            ? { build, detectedAt: Math.min(detectedAt, Date.now()) } : null,
        };
      },
    },
  ),
);
