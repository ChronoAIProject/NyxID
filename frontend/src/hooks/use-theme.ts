import { useLayoutEffect, useMemo } from "react";
import { buildCustomColorCss } from "@/lib/theme-colors";
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
 * Applies the resolved theme to `<html>` for the lifetime of the host surface.
 *
 * Mounted by every app layout/standalone page (dashboard, docs, auth, OAuth
 * consent/error, legal docs, CLI wizard, onboarding takeover) so the whole
 * product respects the theme choice. Landing/blog still skip it and keep
 * the dark token defaults. Applying at `<html>` (rather than a nested
 * wrapper) is deliberate: Radix portals (dropdowns, dialogs, the command
 * palette) render into `document.body`, so they only inherit the light tokens
 * when the class lives on a common ancestor. A layout effect lands the class
 * before first paint (no flash); the cleanup strips it on unmount so leaving
 * the themed surface reverts to dark.
 */
export function useApplyTheme(): void {
  const resolved = useResolvedTheme();
  const textScale = useThemeStore((s) => s.textScale);
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("theme-light", resolved === "light");
    root.classList.toggle("theme-dark", resolved === "dark");
    return () => {
      root.classList.remove("theme-light", "theme-dark");
    };
  }, [resolved]);
  const density = useThemeStore((s) => s.density);
  const motion = useThemeStore((s) => s.motion);
  const customColors = useThemeStore((s) => s.customColors);
  const customCss = useMemo(() => buildCustomColorCss(customColors), [customColors]);

  // Text size: a percentage root font size keeps the user's browser default
  // as the 100% baseline. Only rem text follows it; spacing is px.
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.style.fontSize = textScale === 1 ? "" : `${textScale * 100}%`;
    return () => {
      root.style.fontSize = "";
    };
  }, [textScale]);

  // Density scales Tailwind's spacing step (4px by default in app.css).
  useLayoutEffect(() => {
    const root = document.documentElement;
    if (density === 1) root.style.removeProperty("--spacing");
    else root.style.setProperty("--spacing", `${4 * density}px`);
    return () => {
      root.style.removeProperty("--spacing");
    };
  }, [density]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("motion-reduce", motion === "reduce");
    return () => {
      root.classList.remove("motion-reduce");
    };
  }, [motion]);

  // Custom colors: one stylesheet scoped to `html.theme-*`, removed on unmount.
  useLayoutEffect(() => {
    if (!customCss) return;
    const style = document.createElement("style");
    style.id = "nyxid-custom-colors";
    style.textContent = customCss;
    document.head.appendChild(style);
    return () => {
      style.remove();
    };
  }, [customCss]);
}
