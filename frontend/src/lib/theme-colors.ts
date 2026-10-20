/**
 * User-editable theme colors (Settings → Display).
 *
 * Each editable color maps onto one or more semantic `--color-*` tokens from
 * app.css. Overrides are stored per theme in `theme-store` and injected by
 * `useApplyTheme` as a stylesheet scoped to `html.theme-{mode}`.
 *
 * Values are restricted to `#RRGGBB`: they are written into a <style> element,
 * so anything else (including `;` or `}`) must never reach the CSS builder.
 */

export type ColorMode = "light" | "dark";

export const EDITABLE_COLORS = [
  { key: "background", label: "Page background", vars: ["background"] },
  { key: "sidebar", label: "Sidebar", vars: ["sidebar"] },
  { key: "card", label: "Cards and menus", vars: ["card", "popover", "surface"] },
  { key: "muted", label: "Subtle fills", vars: ["muted"] },
  {
    key: "foreground",
    label: "Primary text",
    vars: ["foreground", "card-foreground", "popover-foreground", "accent-foreground"],
  },
  {
    key: "muted-foreground",
    label: "Secondary text",
    vars: ["muted-foreground", "secondary-foreground"],
  },
  { key: "text-tertiary", label: "Tertiary text", vars: ["text-tertiary"] },
  { key: "border", label: "Borders and dividers", vars: ["border"] },
  { key: "input", label: "Input borders", vars: ["input"] },
  { key: "input-focus", label: "Focused input border", vars: ["input-focus"] },
  { key: "primary", label: "Accent", vars: ["primary", "ring"] },
] as const;

export type ColorKey = (typeof EDITABLE_COLORS)[number]["key"];
export type ColorOverrides = Partial<Record<ColorKey, string>>;
export type CustomColors = Record<ColorMode, ColorOverrides>;

/** The values app.css ships (kept in sync by `theme-colors.test.ts`). */
export const DEFAULT_COLORS: Record<ColorMode, Record<ColorKey, string>> = {
  dark: {
    background: "#17171A",
    sidebar: "#141416",
    card: "#222225",
    muted: "#2C2C30",
    foreground: "#EFEFF1",
    "muted-foreground": "#BABAC1",
    "text-tertiary": "#95959F",
    border: "#414148",
    input: "#55555E",
    "input-focus": "#B0B0B8",
    primary: "#5A2AF1",
  },
  light: {
    background: "#F4F4F5",
    sidebar: "#F4F4F5",
    card: "#FFFFFF",
    muted: "#F4F4F5",
    foreground: "#18181B",
    "muted-foreground": "#3F3F46",
    "text-tertiary": "#5B5B63",
    border: "#C4C4CA",
    input: "#8E8E96",
    "input-focus": "#7C5CF0",
    primary: "#5A2AF1",
  },
};

export const EMPTY_CUSTOM_COLORS: CustomColors = { light: {}, dark: {} };

const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;
const COLOR_KEYS = new Set<string>(EDITABLE_COLORS.map((c) => c.key));

export function isHexColor(value: unknown): value is string {
  return typeof value === "string" && HEX_COLOR.test(value);
}

/** Drop unknown keys and non-hex values (e.g. tampered localStorage). */
export function sanitizeCustomColors(value: unknown): CustomColors {
  const result: CustomColors = { light: {}, dark: {} };
  if (typeof value !== "object" || value === null) return result;
  for (const mode of ["light", "dark"] as const) {
    const overrides = (value as Record<string, unknown>)[mode];
    if (typeof overrides !== "object" || overrides === null) continue;
    for (const [key, color] of Object.entries(overrides)) {
      if (COLOR_KEYS.has(key) && isHexColor(color)) {
        result[mode][key as ColorKey] = color.toUpperCase();
      }
    }
  }
  return result;
}

/** Effective color for a key: the user's override, else the shipped default. */
export function resolveColor(mode: ColorMode, overrides: ColorOverrides, key: ColorKey): string {
  return overrides[key] ?? DEFAULT_COLORS[mode][key];
}

/** Stylesheet text for the user's overrides; empty when there are none. */
export function buildCustomColorCss(custom: CustomColors): string {
  const safe = sanitizeCustomColors(custom);
  return (["dark", "light"] as const)
    .map((mode) => {
      const declarations = EDITABLE_COLORS.flatMap(({ key, vars }) => {
        const color = safe[mode][key];
        return color ? vars.map((v) => `  --color-${v}: ${color};`) : [];
      });
      // Doubled class outranks the `html.theme-*` blocks in app.css.
      return declarations.length
        ? `html.theme-${mode}.theme-${mode} {\n${declarations.join("\n")}\n}`
        : "";
    })
    .filter(Boolean)
    .join("\n");
}

function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  const channel = (v: number) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return (
    0.2126 * channel((n >> 16) & 255) +
    0.7152 * channel((n >> 8) & 255) +
    0.0722 * channel(n & 255)
  );
}

/** WCAG 2 contrast ratio between two `#RRGGBB` colors. */
export function contrastRatio(a: string, b: string): number {
  const la = luminance(a);
  const lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** Readability checks shown under the editor (WCAG AA targets). */
export const CONTRAST_CHECKS: readonly {
  readonly fg: ColorKey;
  readonly bg: ColorKey;
  readonly target: number;
  readonly label: string;
}[] = [
  { fg: "foreground", bg: "background", target: 4.5, label: "Primary text on page" },
  { fg: "muted-foreground", bg: "card", target: 4.5, label: "Secondary text on cards" },
  { fg: "text-tertiary", bg: "background", target: 4.5, label: "Tertiary text on page" },
  { fg: "text-tertiary", bg: "card", target: 4.5, label: "Tertiary text on cards" },
  { fg: "input", bg: "card", target: 3, label: "Input borders on cards" },
  { fg: "input-focus", bg: "card", target: 3, label: "Focused input border on cards" },
];
