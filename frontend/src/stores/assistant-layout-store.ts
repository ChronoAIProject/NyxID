/** Assistant layout choices kept per browser: whether the sidebar is hidden. */

import { create } from "zustand";
import { persist } from "zustand/middleware";

interface AssistantLayoutState {
  readonly sidebarHidden: boolean;
  readonly setSidebarHidden: (hidden: boolean) => void;
}

export const useAssistantLayoutStore = create<AssistantLayoutState>()(
  persist(
    (set) => ({
      sidebarHidden: false,
      setSidebarHidden: (sidebarHidden) => set({ sidebarHidden }),
    }),
    {
      name: "nyxid.assistant-layout",
      version: 1,
      partialize: (state) => ({ sidebarHidden: state.sidebarHidden }),
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Record<string, unknown>;
        return { ...current, sidebarHidden: p.sidebarHidden === true };
      },
    },
  ),
);
