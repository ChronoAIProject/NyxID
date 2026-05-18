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

// DESIGN.md §Border Radius (latest):
//   - rounded-xl (12px) — cards, panels, dialogs, banners, code blocks (large)
//   - rounded-lg (8px)  — buttons, inputs, nav items, code blocks (small/inline)
//   - rounded-md (6px)  — dropdown items, select items, badges
//   - rounded-[6px]     — tooltips
//   - rounded-[4px]     — ButtonIcon inset
export const radius = {
  xs: 4,    // ButtonIcon inset
  sm: 6,    // dropdown items, select items, badges, tooltips
  md: 8,    // buttons, inputs, nav items
  lg: 12,   // cards, panels, dialogs, banners
  xl: 16,   // sheets, large modals (mobile-specific override)
  pill: 999, // legacy alias — prefer explicit `sm` for badges
  full: 999,
} as const;

export const typeScale = {
  // 28px — `sm+` page titles, hero stat values (DESIGN.md §Typography)
  h1: {
    fontFamily: fonts.displayBold,
    fontSize: 28,
    lineHeight: 30,
    fontWeight: "700" as const,
    // DESIGN.md: PageHeader letterSpacing: -0.03em — at 28px that's ~-0.84
    letterSpacing: -0.84,
  },
  // 22px — mobile page title (DESIGN.md: "The mobile downshift to 22px is intentional — never override.")
  pageHeader: {
    fontFamily: fonts.displayBold,
    fontSize: 22,
    lineHeight: 24,
    fontWeight: "700" as const,
    letterSpacing: -0.66,
  },
  // 18px — section titles in detail views (off-scale but useful for mobile detail-sheet titles)
  h2: {
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
    fontWeight: "600" as const,
  },
  // 15px — dialog titles, card headings ("Shortcuts", wizard step titles)
  title: {
    fontFamily: fonts.displaySemi,
    fontSize: 15,
    lineHeight: 20,
    fontWeight: "600" as const,
  },
  // 14px — long-form welcome / marketing copy (e.g. onboarding takeover body)
  description: {
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    lineHeight: 19,
    fontWeight: "400" as const,
  },
  // 13px — sidebar nav items, card body text, mobile-card primary text
  body: {
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "400" as const,
  },
  // 13px — body bold for emphasis (form labels, mobile-card primary text per spec)
  bodyStrong: {
    fontFamily: fonts.bodySemi,
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "600" as const,
  },
  // 12px — body text, button text, table cells, dropdown items, detail row values
  label: {
    fontFamily: fonts.body,
    fontSize: 12,
    lineHeight: 16,
    fontWeight: "500" as const,
  },
  // 12px regular — DetailRow value (web spec: text-[12px] font-medium)
  caption: {
    fontFamily: fonts.bodyRegular,
    fontSize: 12,
    lineHeight: 16,
    fontWeight: "400" as const,
  },
  // 11px — timestamps, mobile-card metadata rows, stat descriptions, pagination counters
  small: {
    fontFamily: fonts.body,
    fontSize: 11,
    lineHeight: 15,
    fontWeight: "500" as const,
  },
  // 10px — section labels, badge text (uppercase, tracking 1.5px)
  overline: {
    fontFamily: fonts.bodySemi,
    fontSize: 10,
    lineHeight: 14,
    fontWeight: "600" as const,
    letterSpacing: 1.5,
    textTransform: "uppercase" as const,
  },
  // 10px Title Case — DESIGN.md §Badges: `text-[10px] font-medium`, Title Case
  badge: {
    fontFamily: fonts.body,
    fontSize: 10,
    lineHeight: 14,
    fontWeight: "500" as const,
  },
  // 9px — sidebar group labels (uppercase, tracking 1.5px). Rarely used on mobile.
  microLabel: {
    fontFamily: fonts.body,
    fontSize: 9,
    lineHeight: 12,
    fontWeight: "500" as const,
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

/**
 * Apple HIG minimum interactive touch target — applied to buttons,
 * input rows, list-row chevrons, etc. Web spec's `h-8` (32px) does
 * not pass mobile a11y review.
 */
export const TOUCH_TARGET = 44;

/** Extra bottom padding so content clears the absolutely-positioned bottom nav bar. */
export const BOTTOM_NAV_CLEARANCE = 120;
