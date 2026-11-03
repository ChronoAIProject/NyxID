// NyxID design system tokens — mobile adaptation.
//
// Source of truth: DESIGN.md at repo root.
// Web and mobile share semantic roles; native text tones account for the
// native base/card backgrounds and accessibility contrast.
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
  sidebarBase: string;   // stationary sidebar behind the elevated content slab
  bg: string;            // main background (spec: base)
  card: string;          // elevated content (spec: surface/card)
  cardSoft: string;      // neutral inset surface
  border: string;        // default card borders + dividers
  controlBorder: string; // essential control boundaries, at least 3:1
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
  sheetScrim: string;
  /** Hairline on the drawer's rounded top edge, separating it from the dimmed chat. */
  sheetEdge: string;
  /** Filled chat composer surface and its faint outline. */
  composerBg: string;
  composerBorder: string;
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

// Native CTA gradient deepens the web light stop so white labels meet AA.
const GRADIENT_START = "#8752CC";
const GRADIENT_END = "#5E00F5";
const GRADIENT_START_PRESSED = "#7541BD";
const GRADIENT_END_PRESSED = "#4A00C2";

// ── Semantic status (DESIGN.md §Color → Semantic Status, dark theme hexes) ──
const SUCCESS_DARK = "#34d399";
const WARNING_DARK = "#f59e0b";
const DANGER_DARK = "#f87171";
const INFO_DARK = "#60a5fa";

// ── Semantic status — light theme equivalents (saturated for white surfaces) ──
const SUCCESS_LIGHT = "#08734d";
const WARNING_LIGHT = "#875300";
const DANGER_LIGHT = "#b91c1c";
const INFO_LIGHT = "#205ab4";

export const darkColors: ThemeColors = {
  // Stationary sidebar is darker than the elevated content slab.
  sidebarBase: "#0C0C0E",
  bg: "#17171A",
  card: "#222225",
  cardSoft: "#1C1C1F",
  // Borders
  border: "#414148",
  controlBorder: "#80808A",
  borderSoft: "rgba(255, 255, 255, 0.14)",
  // 4-level text hierarchy
  textPrimary: "#EFEFF1",
  textSecondary: "#BABAC1",
  textMuted: "#95959F",
  textTertiary: "#95959F",
  // Semantic
  success: SUCCESS_DARK,
  successSoft: "rgba(52, 211, 153, 0.18)",
  info: INFO_DARK,
  infoSoft: "rgba(96, 165, 250, 0.18)",
  warning: WARNING_DARK,
  warningSoft: "rgba(245, 158, 11, 0.18)",
  danger: "#FCA5A5",
  dangerSoft: "#fca5a5",
  dangerSoftBg: "rgba(248, 113, 113, 0.10)",
  // Brand
  primary: "#C6B3FC",
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
  ghostBg: "rgba(255, 255, 255, 0.05)",
  ghostText: "#EFEFF1",
  fabBg: "#17171A",
  fabBorder: "rgba(151, 117, 250, 0.35)",
  overlayBg: "#000",
  sheetScrim: "rgba(0,0,0,0.5)",
  sheetEdge: "rgba(255, 255, 255, 0.16)",
  composerBg: "#2C2C30",
  composerBorder: "rgba(255, 255, 255, 0.06)",
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
  sidebarBase: "#F4F4F5",
  bg: "#FFFFFF",
  card: "#FFFFFF",
  cardSoft: "#FFFFFF",
  border: "#C4C4CA",
  controlBorder: "#85858D",
  borderSoft: "rgba(0, 0, 0, 0.20)",
  textPrimary: "#18181B",
  textSecondary: "#3F3F46",
  textMuted: "#5B5B63",
  textTertiary: "#5B5B63",
  success: SUCCESS_LIGHT,
  successSoft: "rgba(16, 185, 129, 0.18)",
  info: INFO_LIGHT,
  infoSoft: "rgba(59, 130, 246, 0.18)",
  warning: WARNING_LIGHT,
  warningSoft: "rgba(217, 119, 6, 0.18)",
  danger: DANGER_LIGHT,
  dangerSoft: DANGER_LIGHT,
  dangerSoftBg: "rgba(220, 38, 38, 0.06)",
  primary: "#7049C6",
  primaryDim: "#6240B0",
  primaryLight: PRIMARY_LIGHT,
  primaryGlow: "rgba(151, 117, 250, 0.08)",
  // Native light text deepens the brand hue to meet AA on white and tints.
  primaryOnTint: "#6d45b5",
  gradientStart: GRADIENT_START,
  gradientEnd: GRADIENT_END,
  gradientStartPressed: GRADIENT_START_PRESSED,
  gradientEndPressed: GRADIENT_END_PRESSED,
  onPrimary: "#FFFFFF",
  ghostBg: "rgba(0, 0, 0, 0.035)",
  ghostText: "#6d45b5",
  fabBg: "#3F3F46",
  fabBorder: "rgba(151, 117, 250, 0.35)",
  overlayBg: "rgba(0, 0, 0, 0.3)",
  sheetScrim: "rgba(0,0,0,0.24)",
  sheetEdge: "rgba(0, 0, 0, 0.20)",
  composerBg: "#FFFFFF",
  composerBorder: "#C4C4CA",
  handleBg: "rgba(0, 0, 0, 0.12)",
  // DESIGN.md §Usage Rules: idle/hover stays neutral. Light-mode = black-alpha,
  // matching the web pattern of `bg-black/[0.06]` on hover surfaces.
  navActive: "rgba(0, 0, 0, 0.06)",
  shadowColor: "rgba(0, 0, 0, 0.15)",
  successTone: { bg: "rgba(16, 185, 129, 0.10)", text: SUCCESS_LIGHT, border: "rgba(16, 185, 129, 0.30)" },
  warningTone: { bg: "rgba(217, 119, 6, 0.10)",  text: WARNING_LIGHT, border: "rgba(217, 119, 6, 0.30)" },
  dangerTone:  { bg: "rgba(220, 38, 38, 0.08)",  text: DANGER_LIGHT,  border: "rgba(220, 38, 38, 0.25)" },
  infoTone:    { bg: "rgba(59, 130, 246, 0.10)", text: INFO_LIGHT,    border: "rgba(59, 130, 246, 0.30)" },
  primaryTone: { bg: "rgba(151, 117, 250, 0.12)", text: "#6d45b5", border: "rgba(151, 117, 250, 0.30)" },
  riskHigh:   { bg: "rgba(220, 38, 38, 0.08)",  text: DANGER_LIGHT,  border: "rgba(220, 38, 38, 0.20)" },
  riskMedium: { bg: "rgba(217, 119, 6, 0.08)",  text: WARNING_LIGHT, border: "rgba(217, 119, 6, 0.20)" },
  riskLow:    { bg: "rgba(16, 185, 129, 0.08)", text: SUCCESS_LIGHT, border: "rgba(16, 185, 129, 0.20)" },
};

/**
 * Drawer palette: one elevation above the chat it covers. Dark mode cannot rely
 * on the scrim because the chat is already near-black, so the drawer surface,
 * its cards and its quieter text tones all step up together.
 */
export const darkSheetColors: ThemeColors = {
  ...darkColors,
  bg: "#1C1C1F",
  card: "#222225",
  cardSoft: "#2C2C30",
  border: "#414148",
  borderSoft: "rgba(255, 255, 255, 0.07)",
  textMuted: "#A0A0A9",
  textTertiary: "#A0A0A9",
  navActive: "rgba(255, 255, 255, 0.08)",
};

export const lightSheetColors: ThemeColors = {
  ...lightColors,
  bg: "#FFFFFF",
  card: "#F4F4F5",
  cardSoft: "#FFFFFF",
};

/** @deprecated Use `useTheme()` from ThemeContext instead. */
export const mobileTheme = darkColors;
