// NyxID design system tokens — mobile adaptation.
//
// Source of truth: DESIGN.md at repo root.
// Web (Tailwind/CSS) and mobile (StyleSheet) share the same hex values.
// Layout-only tokens that don't apply to mobile (sidebar widths, top-bar
// height, right-panel width) are intentionally not represented here.

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
  onPrimary: string;
  ghostBg: string;
  ghostText: string;
  fabBg: string;
  fabBorder: string;
  overlayBg: string;
  handleBg: string;
  navActive: string;     // active nav row background
  shadowColor: string;
  riskHigh: { bg: string; text: string; border: string };
  riskMedium: { bg: string; text: string; border: string };
  riskLow: { bg: string; text: string; border: string };
};

// ── Brand primaries (DESIGN.md §Color → Primary Accent) ──
const PRIMARY = "#9775fa";        // warm violet (NOT the AI-default Tailwind violet-500)
const PRIMARY_LIGHT = "#c4b5fd";  // logo wordmark, light text on accent
const PRIMARY_DEEP = "#7c5ce0";   // pressed states

// ── Semantic status (DESIGN.md §Color → Semantic Status) ──
const SUCCESS = "#34d399";
const WARNING = "#f59e0b";
const DANGER = "#f87171";
const INFO = "#60a5fa";

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
  success: SUCCESS,
  successSoft: "rgba(52, 211, 153, 0.18)",
  info: INFO,
  infoSoft: "rgba(96, 165, 250, 0.18)",
  warning: WARNING,
  warningSoft: "rgba(245, 158, 11, 0.18)",
  danger: DANGER,
  dangerSoft: "#fca5a5",
  dangerSoftBg: "rgba(248, 113, 113, 0.10)",
  // Brand
  primary: PRIMARY,
  primaryDim: PRIMARY_DEEP,
  primaryLight: PRIMARY_LIGHT,
  primaryGlow: "rgba(151, 117, 250, 0.12)",
  onPrimary: "#FFFFFF",
  // Ghost / interactive chrome
  ghostBg: "rgba(151, 117, 250, 0.06)",
  ghostText: "#e8e4f0",
  fabBg: "rgba(7, 6, 14, 1)",
  fabBorder: "rgba(151, 117, 250, 0.35)",
  overlayBg: "#000",
  handleBg: "rgba(255, 255, 255, 0.15)",
  navActive: "rgba(255, 255, 255, 0.06)",
  shadowColor: "#000",
  riskHigh: { bg: "rgba(248, 113, 113, 0.12)", text: "#fca5a5", border: "rgba(248, 113, 113, 0.30)" },
  riskMedium: { bg: "rgba(245, 158, 11, 0.12)", text: "#fcd34d", border: "rgba(245, 158, 11, 0.30)" },
  riskLow: { bg: "rgba(52, 211, 153, 0.12)", text: "#6ee7b7", border: "rgba(52, 211, 153, 0.30)" },
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
  success: "#10B981",
  successSoft: "rgba(16, 185, 129, 0.18)",
  info: "#3B82F6",
  infoSoft: "rgba(59, 130, 246, 0.18)",
  warning: "#D97706",
  warningSoft: "rgba(217, 119, 6, 0.18)",
  danger: "#DC2626",
  dangerSoft: "#DC2626",
  dangerSoftBg: "rgba(220, 38, 38, 0.06)",
  primary: PRIMARY,
  primaryDim: PRIMARY_DEEP,
  primaryLight: PRIMARY_LIGHT,
  primaryGlow: "rgba(151, 117, 250, 0.08)",
  onPrimary: "#FFFFFF",
  ghostBg: "rgba(151, 117, 250, 0.05)",
  ghostText: PRIMARY,
  fabBg: "rgba(80, 78, 93, 1)",
  fabBorder: "rgba(151, 117, 250, 0.35)",
  overlayBg: "rgba(0, 0, 0, 0.3)",
  handleBg: "rgba(0, 0, 0, 0.12)",
  navActive: "#EDEBF7",
  shadowColor: "rgba(30, 20, 60, 0.15)",
  riskHigh: { bg: "rgba(220, 38, 38, 0.08)", text: "#DC2626", border: "rgba(220, 38, 38, 0.20)" },
  riskMedium: { bg: "rgba(217, 119, 6, 0.08)", text: "#D97706", border: "rgba(217, 119, 6, 0.20)" },
  riskLow: { bg: "rgba(16, 185, 129, 0.08)", text: "#10B981", border: "rgba(16, 185, 129, 0.20)" },
};

/** @deprecated Use `useTheme()` from ThemeContext instead. */
export const mobileTheme = darkColors;
