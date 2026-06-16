/**
 * Dashboard theme — light / dark / follow-system.
 *
 * Persisted to localStorage so the choice survives reloads (mirrors
 * `consent-store`). Only the user's `mode` is stored; the live OS preference
 * is re-derived on each boot and kept current via a `matchMedia` listener, so
 * a stale persisted value can never override the real system setting.
 *
 * The theme is applied (and confined) to the dashboard by
 * `useApplyDashboardTheme` in `hooks/use-theme.ts`; public surfaces
 * (landing/blog) never carry a theme class and stay on the dark defaults.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

export type ThemeMode = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

/**
 * Collapse (mode, OS preference) into the concrete theme to render.
 * Pure + exported so it can be unit-tested without a DOM.
 */
export function resolveTheme(
  mode: ThemeMode,
  systemPrefersDark: boolean,
): ResolvedTheme {
  if (mode === "system") return systemPrefersDark ? "dark" : "light";
  return mode;
}

function getSystemPrefersDark(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    // Default to the product's native canvas (dark) when the OS can't be read.
    return true;
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

interface ThemeState {
  /** The user's chosen mode. `system` follows the OS. Persisted. */
  readonly mode: ThemeMode;
  /** Live OS preference. Not persisted — re-derived each boot and on change. */
  readonly systemPrefersDark: boolean;
  /** Set an explicit mode (or `system`). */
  readonly setMode: (mode: ThemeMode) => void;
  /** Flip between explicit light/dark based on what's currently showing. */
  readonly toggle: () => void;
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set, get) => ({
      mode: "system",
      systemPrefersDark: getSystemPrefersDark(),
      setMode: (mode) => set({ mode }),
      toggle: () => {
        const { mode, systemPrefersDark } = get();
        const current = resolveTheme(mode, systemPrefersDark);
        set({ mode: current === "dark" ? "light" : "dark" });
      },
    }),
    {
      name: "nyxid.theme",
      version: 1,
      // Persist only the user's intent; the OS preference is environmental.
      partialize: (s) => ({ mode: s.mode }),
    },
  ),
);

// Keep `systemPrefersDark` in sync with the OS while the app is open, so
// `mode: "system"` reacts live to the user flipping their system theme.
if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  mq.addEventListener?.("change", (e) =>
    useThemeStore.setState({ systemPrefersDark: e.matches }),
  );
}
