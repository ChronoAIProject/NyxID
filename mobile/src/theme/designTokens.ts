// NyxID type system — mobile adaptation of DESIGN.md §Typography.
//
// Web spec uses:
//   - Space Grotesk 500 (display/hero — page titles, stats, card headings)
//   - Manrope 400 (body) / 500 (UI labels)
//   - JetBrains Mono 400 (data — timestamps, log entries, code, API paths)
//   - Playfair Display 400 (logo wordmark only — mobile uses the icon, not the wordmark)

export const fonts = {
  // Display — page titles, stat values, card headings (Space Grotesk per spec)
  display: "SpaceGrotesk_500Medium",
  displaySemi: "SpaceGrotesk_600SemiBold",
  displayBold: "SpaceGrotesk_700Bold",
  // Body — Manrope 400 per spec (descriptions, row content)
  bodyRegular: "Manrope_400Regular",
  // UI / labels — Manrope 500 per spec (nav items, button labels, form labels)
  body: "Manrope_500Medium",
  bodySemi: "Manrope_600SemiBold",
  bodyBold: "Manrope_700Bold",
  // Data — timestamps, log entries, API paths, code (JetBrains Mono per spec)
  mono: "JetBrainsMono_400Regular",
  // ── deprecated aliases (kept so legacy refs don't break the build) ──
  /** @deprecated use `displayBold` */
  headingBold: "SpaceGrotesk_700Bold",
  /** @deprecated use `displaySemi` */
  headingSemi: "SpaceGrotesk_600SemiBold",
} as const;

export const spacing = {
  xxs: 2,
  xs: 4,
  sm: 8,
  md: 10,
  lg: 12,
  xl: 14,
  xxl: 16,
  xxxl: 20,
  huge: 24,
} as const;

// Spec: 10px cards/panels, 8px buttons/inputs/nav, 100px badges/pills (full).
// Mobile expands with `xl` (kept for back-compat) and `pill` (== `full` here).
export const radius = {
  sm: 6,    // dropdown items, select items, tooltips
  md: 8,    // buttons, inputs, nav items
  lg: 10,   // cards, panels, dialogs, popovers (DESIGN.md §Layout)
  xl: 14,   // legacy slot — prefer `lg`
  pill: 100, // badges, pills — fully rounded
  full: 999,
} as const;

export const typeScale = {
  // 28px — stat values, page titles (Space Grotesk per DESIGN.md §Typography)
  h1: {
    fontFamily: fonts.displayBold,
    fontSize: 28,
    lineHeight: 34,
    fontWeight: "700" as const,
    letterSpacing: -0.3,
  },
  // 20px — top-level page title (mobile header) — Space Grotesk
  pageHeader: {
    fontFamily: fonts.displaySemi,
    fontSize: 20,
    lineHeight: 26,
    fontWeight: "600" as const,
  },
  // 18px — section titles in detail views, dialog titles
  h2: {
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
    fontWeight: "600" as const,
  },
  // 15px — card titles (Space Grotesk per spec)
  title: {
    fontFamily: fonts.displaySemi,
    fontSize: 15,
    lineHeight: 20,
    fontWeight: "600" as const,
  },
  // 14px — descriptions, secondary body (Manrope 400 per spec)
  description: {
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    lineHeight: 19,
    fontWeight: "400" as const,
  },
  // 13px — nav items, row content, body text (Manrope 400 per spec)
  body: {
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "400" as const,
  },
  // 13px — body bold for emphasis (form labels, button labels)
  bodyStrong: {
    fontFamily: fonts.bodyBold,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "700" as const,
  },
  // 13px — UI labels, button text (Manrope 500 per spec)
  label: {
    fontFamily: fonts.body,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "500" as const,
  },
  // 12px — small body, secondary button text
  caption: {
    fontFamily: fonts.bodyRegular,
    fontSize: 12,
    lineHeight: 16,
    fontWeight: "400" as const,
  },
  // 11px — badges, timestamps, stat descriptions, tertiary text
  small: {
    fontFamily: fonts.body,
    fontSize: 11,
    lineHeight: 15,
    fontWeight: "500" as const,
  },
  // 10px — section labels (DESIGN.md: uppercase, tracking 1.2px)
  overline: {
    fontFamily: fonts.bodySemi,
    fontSize: 10,
    lineHeight: 14,
    fontWeight: "600" as const,
    letterSpacing: 1.2,
    textTransform: "uppercase" as const,
  },
  // 12px mono — timestamps, log entries, API paths, code snippets
  mono: {
    fontFamily: fonts.mono,
    fontSize: 12,
    lineHeight: 16,
    fontWeight: "400" as const,
  },
  // 11px mono — compact mono for dense metadata
  monoSmall: {
    fontFamily: fonts.mono,
    fontSize: 11,
    lineHeight: 15,
    fontWeight: "400" as const,
  },
} as const;

/**
 * Apple HIG minimum interactive touch target — applied to buttons,
 * input rows, list-row chevrons, etc. Spec's web `h-8` (32px) does
 * not pass mobile a11y review.
 */
export const TOUCH_TARGET = 44;

/** Extra bottom padding so content clears the absolutely-positioned bottom nav bar. */
export const BOTTOM_NAV_CLEARANCE = 120;
