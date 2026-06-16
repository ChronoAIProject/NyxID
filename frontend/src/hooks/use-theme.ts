import { useLayoutEffect } from "react";
import {
  useThemeStore,
  resolveTheme,
  type ResolvedTheme,
} from "@/stores/theme-store";

/** The concrete theme (light|dark) for the current store state. */
export function useResolvedTheme(): ResolvedTheme {
  const mode = useThemeStore((s) => s.mode);
  const systemPrefersDark = useThemeStore((s) => s.systemPrefersDark);
  return resolveTheme(mode, systemPrefersDark);
}

/**
 * Applies the resolved theme to `<html>` for the lifetime of the dashboard.
 *
 * Mounted only by `DashboardLayout`, so the theme class is confined to
 * logged-in surfaces — public pages (landing/blog) never get one and keep the
 * dark token defaults. Applying at `<html>` (rather than a nested wrapper) is
 * deliberate: Radix portals (dropdowns, dialogs, the command palette) render
 * into `document.body`, so they only inherit the light tokens when the class
 * lives on a common ancestor. A layout effect lands the class before first
 * paint (no flash); the cleanup strips it on unmount so leaving the dashboard
 * reverts to dark.
 */
export function useApplyDashboardTheme(): void {
  const resolved = useResolvedTheme();
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("theme-light", resolved === "light");
    root.classList.toggle("theme-dark", resolved === "dark");
    return () => {
      root.classList.remove("theme-light", "theme-dark");
    };
  }, [resolved]);
}
