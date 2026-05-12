// NyxID type system — mobile adaptation of DESIGN.md §Typography.
//
// Web spec uses:
//   - Space Grotesk 500 (display/hero — page titles, stats, card headings)
//   - Manrope 400 (body) / 500 (UI labels)
//   - JetBrains Mono 400 (data — timestamps, log entries, code, API paths)
//   - Playfair Display 400 (logo wordmark only — mobile uses the icon, not the wordmark)
//
// Mobile adapts the spec's type scale (10/11/12/13/15/28 px) to React Native's
// StyleSheet system. Body text floors at 12px for readability; smallest text
// (overline / status labels) is 10px.

export const fonts = {
  // Display — page titles, stat values, card headings (Space Grotesk per spec)
  display: "SpaceGrotesk_500Medium",
  displaySemi: "SpaceGrotesk_600SemiBold",
  displayBold: "SpaceGrotesk_700Bold",
  // Body / UI — body text + labels (Manrope per spec)
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

export const radius = {
  sm: 6,    // dropdown items, select items, tooltips, badges
  md: 8,    // buttons, inputs, nav items
  lg: 12,   // cards, panels, dialogs, popovers
  xl: 14,   // legacy slot, prefer lg
  pill: 28,
} as const;

export const typeScale = {
  // 28px — page titles (DESIGN.md §Typography)
  h1: {
    fontFamily: fonts.displayBold,
    fontSize: 28,
    lineHeight: 34,
    fontWeight: "700" as const,
    letterSpacing: -0.3,
  },
  // 18-20px — section titles in detail views, dialog titles upscaled for touch
  h2: {
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
    fontWeight: "600" as const,
  },
  // 15px — card headings, dialog titles
  title: {
    fontFamily: fonts.displaySemi,
    fontSize: 15,
    lineHeight: 20,
    fontWeight: "600" as const,
  },
  // 13px — sidebar nav items, section titles in detail views
  bodyStrong: {
    fontFamily: fonts.bodyBold,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "700" as const,
  },
  // 13px — body text, button text default
  body: {
    fontFamily: fonts.body,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "500" as const,
  },
  // 12px — table cells, detail row values, input text, secondary body
  caption: {
    fontFamily: fonts.body,
    fontSize: 12,
    lineHeight: 16,
    fontWeight: "500" as const,
  },
  // 11px — timestamps, stat descriptions, tertiary text
  small: {
    fontFamily: fonts.body,
    fontSize: 11,
    lineHeight: 15,
    fontWeight: "500" as const,
  },
  // 10px — section labels (uppercase + letterspaced), badges, overlines
  overline: {
    fontFamily: fonts.bodySemi,
    fontSize: 10,
    lineHeight: 14,
    fontWeight: "600" as const,
    letterSpacing: 1.5,
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

/** Extra bottom padding so content clears the absolutely-positioned bottom nav bar. */
export const BOTTOM_NAV_CLEARANCE = 120;
