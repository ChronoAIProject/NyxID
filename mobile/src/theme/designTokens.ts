export const fonts = {
  headingBold: "SpaceGrotesk_700Bold",
  headingSemi: "SpaceGrotesk_600SemiBold",
  body: "Manrope_500Medium",
  bodySemi: "Manrope_600SemiBold",
  bodyBold: "Manrope_700Bold",
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
  sm: 8,
  md: 10,
  lg: 12,
  xl: 14,
  pill: 28,
} as const;

export const typeScale = {
  h1: {
    fontFamily: fonts.headingBold,
    fontSize: 30,
    lineHeight: 36,
    fontWeight: "700" as const,
  },
  h2: {
    fontFamily: fonts.headingSemi,
    fontSize: 18,
    lineHeight: 24,
    fontWeight: "600" as const,
  },
  title: {
    fontFamily: fonts.headingBold,
    fontSize: 16,
    lineHeight: 22,
    fontWeight: "700" as const,
  },
  body: {
    fontFamily: fonts.body,
    fontSize: 14,
    lineHeight: 20,
    fontWeight: "500" as const,
  },
  bodyStrong: {
    fontFamily: fonts.bodyBold,
    fontSize: 14,
    lineHeight: 20,
    fontWeight: "700" as const,
  },
  caption: {
    fontFamily: fonts.body,
    fontSize: 12,
    lineHeight: 16,
    fontWeight: "500" as const,
  },
  overline: {
    fontFamily: fonts.bodySemi,
    fontSize: 10,
    lineHeight: 14,
    fontWeight: "600" as const,
    letterSpacing: 0.4,
  },
} as const;
