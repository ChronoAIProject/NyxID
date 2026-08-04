// NyxID design system tokens — mobile adaptation.
//
// Source of truth: DESIGN.md at repo root.
// Web (Tailwind/CSS) and mobile (StyleSheet) share the same hex values.
// Layout-only tokens that don't apply to mobile (sidebar widths, top-bar
// height, right-panel width) are intentionally not represented here.

/**
 * Tone triple for badges and inline banners.
 * Mirrors DESIGN.md §Badges recipe: `border-{color}/30 bg-{color}/15 text-{color}`
 * (success/warning/info use `/10` fill). Both light + dark carry their own values
 * so a pill never inherits the wrong-theme hex.
 */
export type ToneTriple = { bg: string; text: string; border: string };

export type ThemeColors = {
  bg: string;            // main background (spec: base)
  card: string;          // elevated content (spec: surface/card)
  cardSoft: string;      // darkest layer (spec: sidebar)
  border: string;        // default card borders + dividers
  borderSoft: string;    // subtle dividers within cards
  textPrimary: string;   // headings, important text (warm off-white)
  textSecondary: string; // body text, descriptions
  textMuted: string;     // supporting text, metadata
  textTertiary: string;  // timestamps, disabled text, section labels
  success: string;       // active services, healthy nodes, approved grants
  successSoft: string;
  info: string;          // informational badges, auth events
  infoSoft: string;
  warning: string;       // expiring tokens, pending approvals
  warningSoft: string;
  danger: string;        // expired, failed, denied
  dangerSoft: string;
  dangerSoftBg: string;
  primary: string;       // warm violet — identity + interaction only
  primaryDim: string;    // pressed states, hover deep
  primaryLight: string;  // logo wordmark tones, light text on accent
  primaryGlow: string;   // subtle ambient accent (AI setup card, etc.)
  /** `nyx-gradient-vivid` stops — used as fill for primary CTAs (DESIGN.md §Buttons). */
  gradientStart: string;
  gradientEnd: string;
  /** Pressed-state gradient (slightly darker). */
  gradientStartPressed: string;
  gradientEndPressed: string;
  onPrimary: string;
  /**
   * Color for purple-tinted pill TEXT on the current theme. On dark this is
   * the light wordmark tone (#c4b5fd) so it pops on a dark-purple-tinted bg;
   * on light it's the brand purple itself (#9775fa) so the text reads against
   * a pale-purple-tinted bg on a white card. Use this on `connectedPill` text,
   * `linkPill` text, `scopeBadge` text, segment count text — anything text that
   * sits on a primary-tinted (~15% alpha) surface.
   */
  primaryOnTint: string;
  ghostBg: string;
  ghostText: string;
  fabBg: string;
  fabBorder: string;
  overlayBg: string;
  handleBg: string;
  navActive: string;     // active nav row background — neutral white/black alpha, NEVER purple-tinted
  shadowColor: string;
  /**
   * Semantic tone triples for badges + banners. Use these instead of hardcoding
   * rgba literals (which only work for one theme). Each triple satisfies the
   * DESIGN.md §Badges recipe.
   */
  successTone: ToneTriple;
  warningTone: ToneTriple;
  dangerTone: ToneTriple;
  infoTone: ToneTriple;
  /** Purple-accent (identity) tone — used for `modeChip`, `connectedPill`, etc. */
  primaryTone: ToneTriple;
  riskHigh: ToneTriple;
  riskMedium: ToneTriple;
  riskLow: ToneTriple;
};

// ── Brand primaries (DESIGN.md §Color → Primary Accent) ──
const PRIMARY = "#9775fa";        // warm violet (NOT the AI-default Tailwind violet-500)
const PRIMARY_LIGHT = "#c4b5fd";  // logo wordmark, light text on accent
const PRIMARY_DEEP = "#7c5ce0";   // pressed states

// ── nyx-gradient-vivid (frontend/src/app.css: linear-gradient(to right, #A672FB 0%, #5E00F5 100%)) ──
const GRADIENT_START = "#A672FB";
const GRADIENT_END = "#5E00F5";
const GRADIENT_START_PRESSED = "#8C5AE0";
const GRADIENT_END_PRESSED = "#4A00C2";

// ── Semantic status (DESIGN.md §Color → Semantic Status, dark theme hexes) ──
const SUCCESS_DARK = "#34d399";
const WARNING_DARK = "#f59e0b";
const DANGER_DARK = "#f87171";
const INFO_DARK = "#60a5fa";

// ── Semantic status — light theme equivalents (saturated for white surfaces) ──
const SUCCESS_LIGHT = "#10B981";
const WARNING_LIGHT = "#D97706";
const DANGER_LIGHT = "#DC2626";
const INFO_LIGHT = "#3B82F6";

export const darkColors: ThemeColors = {
  // 3-layer depth — sidebar / base / surface
  bg: "#07060e",
  card: "#0c0b14",
  cardSoft: "#06060b",
  // Borders
  border: "#1c1828",
  borderSoft: "rgba(255, 255, 255, 0.05)",
  // 4-level text hierarchy
  textPrimary: "#e8e4f0",
  textSecondary: "#9e96b0",
  textMuted: "#7a7490",
  textTertiary: "#4a4460",
  // Semantic
  success: SUCCESS_DARK,
  successSoft: "rgba(52, 211, 153, 0.18)",
  info: INFO_DARK,
  infoSoft: "rgba(96, 165, 250, 0.18)",
  warning: WARNING_DARK,
  warningSoft: "rgba(245, 158, 11, 0.18)",
  danger: DANGER_DARK,
  dangerSoft: "#fca5a5",
  dangerSoftBg: "rgba(248, 113, 113, 0.10)",
  // Brand
  primary: PRIMARY,
  primaryDim: PRIMARY_DEEP,
  primaryLight: PRIMARY_LIGHT,
  primaryGlow: "rgba(151, 117, 250, 0.12)",
  // On dark, the pale wordmark hex reads correctly on a purple tint.
  primaryOnTint: PRIMARY_LIGHT,
  gradientStart: GRADIENT_START,
  gradientEnd: GRADIENT_END,
  gradientStartPressed: GRADIENT_START_PRESSED,
  gradientEndPressed: GRADIENT_END_PRESSED,
  onPrimary: "#FFFFFF",
  // Ghost / interactive chrome — DESIGN.md: neutral white-alpha, NOT purple-tinted.
  ghostBg: "rgba(255, 255, 255, 0.03)",
  ghostText: "#e8e4f0",
  fabBg: "rgba(7, 6, 14, 1)",
  fabBorder: "rgba(151, 117, 250, 0.35)",
  overlayBg: "#000",
  handleBg: "rgba(255, 255, 255, 0.15)",
  // DESIGN.md §Usage Rules: "Hover stays neutral (bg-white/[0.03] / bg-white/[0.06])".
  navActive: "rgba(255, 255, 255, 0.06)",
  shadowColor: "#000",
  // Semantic tone triples (10% fill / 30% border / brand text).
  successTone: { bg: "rgba(52, 211, 153, 0.10)", text: SUCCESS_DARK, border: "rgba(52, 211, 153, 0.30)" },
  warningTone: { bg: "rgba(245, 158, 11, 0.10)", text: WARNING_DARK, border: "rgba(245, 158, 11, 0.30)" },
  dangerTone:  { bg: "rgba(248, 113, 113, 0.10)", text: DANGER_DARK,  border: "rgba(248, 113, 113, 0.30)" },
  infoTone:    { bg: "rgba(96, 165, 250, 0.10)", text: INFO_DARK,    border: "rgba(96, 165, 250, 0.30)" },
  primaryTone: { bg: "rgba(151, 117, 250, 0.15)", text: PRIMARY_LIGHT, border: "rgba(151, 117, 250, 0.30)" },
  riskHigh:   { bg: "rgba(248, 113, 113, 0.12)", text: "#fca5a5", border: "rgba(248, 113, 113, 0.30)" },
  riskMedium: { bg: "rgba(245, 158, 11, 0.12)", text: "#fcd34d", border: "rgba(245, 158, 11, 0.30)" },
  riskLow:    { bg: "rgba(52, 211, 153, 0.12)", text: "#6ee7b7", border: "rgba(52, 211, 153, 0.30)" },
};

export const lightColors: ThemeColors = {
  bg: "#F8F7FC",
  card: "#FFFFFF",
  cardSoft: "#F3F2F8",
  border: "#DDD9E8",
  borderSoft: "#ECEAF3",
  textPrimary: "#1A1730",
  textSecondary: "#5E5875",
  textMuted: "#8F88AB",
  textTertiary: "#A8A0BC",
  success: SUCCESS_LIGHT,
  successSoft: "rgba(16, 185, 129, 0.18)",
  info: INFO_LIGHT,
  infoSoft: "rgba(59, 130, 246, 0.18)",
  warning: WARNING_LIGHT,
  warningSoft: "rgba(217, 119, 6, 0.18)",
  danger: DANGER_LIGHT,
  dangerSoft: DANGER_LIGHT,
  dangerSoftBg: "rgba(220, 38, 38, 0.06)",
  primary: PRIMARY,
  primaryDim: PRIMARY_DEEP,
  primaryLight: PRIMARY_LIGHT,
  primaryGlow: "rgba(151, 117, 250, 0.08)",
  // On light, use the saturated brand purple as text — the pale wordmark hex
  // disappears on a white card behind a 15% purple tint.
  primaryOnTint: PRIMARY,
  gradientStart: GRADIENT_START,
  gradientEnd: GRADIENT_END,
  gradientStartPressed: GRADIENT_START_PRESSED,
  gradientEndPressed: GRADIENT_END_PRESSED,
  onPrimary: "#FFFFFF",
  ghostBg: "rgba(0, 0, 0, 0.03)",
  ghostText: PRIMARY,
  fabBg: "rgba(80, 78, 93, 1)",
  fabBorder: "rgba(151, 117, 250, 0.35)",
  overlayBg: "rgba(0, 0, 0, 0.3)",
  handleBg: "rgba(0, 0, 0, 0.12)",
  // DESIGN.md §Usage Rules: idle/hover stays neutral. Light-mode = black-alpha,
  // matching the web pattern of `bg-black/[0.06]` on hover surfaces.
  navActive: "rgba(0, 0, 0, 0.06)",
  shadowColor: "rgba(30, 20, 60, 0.15)",
  successTone: { bg: "rgba(16, 185, 129, 0.10)", text: SUCCESS_LIGHT, border: "rgba(16, 185, 129, 0.30)" },
  warningTone: { bg: "rgba(217, 119, 6, 0.10)",  text: WARNING_LIGHT, border: "rgba(217, 119, 6, 0.30)" },
  dangerTone:  { bg: "rgba(220, 38, 38, 0.08)",  text: DANGER_LIGHT,  border: "rgba(220, 38, 38, 0.25)" },
  infoTone:    { bg: "rgba(59, 130, 246, 0.10)", text: INFO_LIGHT,    border: "rgba(59, 130, 246, 0.30)" },
  primaryTone: { bg: "rgba(151, 117, 250, 0.12)", text: PRIMARY, border: "rgba(151, 117, 250, 0.30)" },
  riskHigh:   { bg: "rgba(220, 38, 38, 0.08)",  text: DANGER_LIGHT,  border: "rgba(220, 38, 38, 0.20)" },
  riskMedium: { bg: "rgba(217, 119, 6, 0.08)",  text: WARNING_LIGHT, border: "rgba(217, 119, 6, 0.20)" },
  riskLow:    { bg: "rgba(16, 185, 129, 0.08)", text: SUCCESS_LIGHT, border: "rgba(16, 185, 129, 0.20)" },
};

/** @deprecated Use `useTheme()` from ThemeContext instead. */
export const mobileTheme = darkColors;
