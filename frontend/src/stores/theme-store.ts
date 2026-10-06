/**
 * Display preferences — theme mode, text size, density, motion and custom
 * colors (Settings → Display).
 *
 * Persisted to localStorage, so preferences are per browser (mirrors
 * `consent-store`). The live OS color preference is re-derived on each boot
 * and kept current via a `matchMedia` listener, so a stale persisted value can
 * never override the real system setting. Every persisted field is validated
 * on load; anything unexpected falls back to the default.
 *
 * Applied by `useApplyTheme` in `hooks/use-theme.ts`, mounted by every themed
 * layout; landing/blog never carry a theme class and stay on the defaults.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";
import {
  EMPTY_CUSTOM_COLORS,
  isHexColor,
  sanitizeCustomColors,
  type ColorKey,
  type ColorMode,
  type CustomColors,
} from "@/lib/theme-colors";

export type ThemeMode = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

/**
 * Text size as a multiple of the browser's default font size. Applied as the
 * root font size, so it scales every `rem` text token (`text-12`, `text-xs`, …)
 * while spacing stays fixed (`--spacing` is px in app.css).
 */
export const TEXT_SCALES = [0.875, 1, 1.125, 1.25] as const;
export type TextScale = (typeof TEXT_SCALES)[number];

/** Multiplier on the 4px spacing step: padding, gaps and control heights. */
export const DENSITIES = [0.875, 1, 1.125] as const;
export type Density = (typeof DENSITIES)[number];

/** `system` follows the OS reduced-motion setting; `reduce` always reduces. */
export type MotionPreference = "system" | "reduce";

/** Main dashboard sidebar behaviour (the assistant sidebar is always expanded). */
export type SidebarMode = "expanded" | "collapsed" | "hover";
export type SidebarId = "dashboard" | "assistant";

export const SIDEBAR_WIDTH = { min: 180, max: 360, default: 200 } as const;

export function clampSidebarWidth(width: number): number {
  return Math.round(Math.min(SIDEBAR_WIDTH.max, Math.max(SIDEBAR_WIDTH.min, width)));
}

function validSidebarWidth(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value)
    ? clampSidebarWidth(value)
    : SIDEBAR_WIDTH.default;
}

/** Where the dashboard sidebar mode lived before it joined display settings. */
const LEGACY_SIDEBAR_MODE_KEY = "nyxid:sidebar-mode";
const SIDEBAR_MODES: readonly SidebarMode[] = ["expanded", "collapsed", "hover"];

function legacySidebarMode(): SidebarMode {
  try {
    const legacy = localStorage.getItem(LEGACY_SIDEBAR_MODE_KEY);
    return SIDEBAR_MODES.includes(legacy as SidebarMode) ? (legacy as SidebarMode) : "expanded";
  } catch {
    return "expanded";
  }
}

const THEME_MODES: readonly ThemeMode[] = ["system", "light", "dark"];
const MOTION_PREFERENCES: readonly MotionPreference[] = ["system", "reduce"];

function oneOf<T>(allowed: readonly T[], value: unknown, fallback: T): T {
  return allowed.includes(value as T) ? (value as T) : fallback;
}

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
  readonly textScale: TextScale;
  readonly density: Density;
  readonly motion: MotionPreference;
  /** Per-theme color overrides (`#RRGGBB` only). Empty = shipped colors. */
  readonly customColors: CustomColors;
  readonly sidebarMode: SidebarMode;
  /** Expanded width in px of each desktop sidebar. */
  readonly sidebarWidths: Record<SidebarId, number>;
  /** Live OS preference. Not persisted — re-derived each boot and on change. */
  readonly systemPrefersDark: boolean;
  /** Set an explicit mode (or `system`). */
  readonly setMode: (mode: ThemeMode) => void;
  /** Flip between explicit light/dark based on what's currently showing. */
  readonly toggle: () => void;
  readonly setTextScale: (textScale: TextScale) => void;
  readonly setDensity: (density: Density) => void;
  readonly setMotion: (motion: MotionPreference) => void;
  /** Override one color for one theme; non-hex values are ignored. */
  readonly setCustomColor: (mode: ColorMode, key: ColorKey, color: string) => void;
  /** Revert one color, or every color of a theme when `key` is omitted. */
  readonly resetCustomColors: (mode: ColorMode, key?: ColorKey) => void;
  readonly setSidebarMode: (mode: SidebarMode) => void;
  /** Clamped to `SIDEBAR_WIDTH`. */
  readonly setSidebarWidth: (sidebar: SidebarId, width: number) => void;
  /** Restore every display preference except the theme mode. */
  readonly resetDisplay: () => void;
}

const DISPLAY_DEFAULTS = {
  textScale: 1,
  density: 1,
  motion: "system",
  customColors: EMPTY_CUSTOM_COLORS,
  sidebarMode: "expanded",
  sidebarWidths: { dashboard: SIDEBAR_WIDTH.default, assistant: SIDEBAR_WIDTH.default },
} as const satisfies Partial<ThemeState>;

export const useThemeStore = create<ThemeState>()(
  persist(
    (set, get) => ({
      mode: "system",
      ...DISPLAY_DEFAULTS,
      // Users without a persisted display state keep their pre-existing choice.
      sidebarMode: legacySidebarMode(),
      systemPrefersDark: getSystemPrefersDark(),
      setMode: (mode) => set({ mode }),
      setTextScale: (textScale) => set({ textScale }),
      setDensity: (density) => set({ density }),
      setMotion: (motion) => set({ motion }),
      setSidebarMode: (sidebarMode) => set({ sidebarMode }),
      setSidebarWidth: (sidebar, width) =>
        set({ sidebarWidths: { ...get().sidebarWidths, [sidebar]: clampSidebarWidth(width) } }),
      setCustomColor: (mode, key, color) => {
        if (!isHexColor(color)) return;
        const { customColors } = get();
        set({
          customColors: {
            ...customColors,
            [mode]: { ...customColors[mode], [key]: color.toUpperCase() },
          },
        });
      },
      resetCustomColors: (mode, key) => {
        const { customColors } = get();
        const next = { ...customColors[mode] };
        if (key) delete next[key];
        set({ customColors: { ...customColors, [mode]: key ? next : {} } });
      },
      resetDisplay: () => set({ ...DISPLAY_DEFAULTS }),
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
      partialize: (s) => ({
        mode: s.mode,
        textScale: s.textScale,
        density: s.density,
        motion: s.motion,
        customColors: s.customColors,
        sidebarMode: s.sidebarMode,
        sidebarWidths: s.sidebarWidths,
      }),
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Record<string, unknown>;
        return {
          ...current,
          mode: oneOf(THEME_MODES, p.mode, current.mode),
          textScale: oneOf(TEXT_SCALES, p.textScale, current.textScale),
          density: oneOf(DENSITIES, p.density, current.density),
          motion: oneOf(MOTION_PREFERENCES, p.motion, current.motion),
          customColors: sanitizeCustomColors(p.customColors),
          sidebarMode: oneOf(SIDEBAR_MODES, p.sidebarMode, current.sidebarMode),
          sidebarWidths: {
            dashboard: validSidebarWidth((p.sidebarWidths as Record<string, unknown> | undefined)?.dashboard),
            assistant: validSidebarWidth((p.sidebarWidths as Record<string, unknown> | undefined)?.assistant),
          },
        };
      },
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
