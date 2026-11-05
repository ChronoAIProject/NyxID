import type { ThemeColors } from "../../theme/mobileTheme";

/** Auth-only fills: workspace cards and the global light theme stay unchanged. */
export const authLightColors = {
  fill: "#F1F1F4",
  pressed: "#E5E5EB",
  border: "#82828D",
  focus: "#79649F",
  muted: "#60606A",
} as const;

export function authSurfaceColors(
  colors: ThemeColors,
  mode: string,
): ThemeColors {
  if (mode !== "light") return colors;
  return {
    ...colors,
    card: authLightColors.fill,
    cardSoft: authLightColors.pressed,
    ghostBg: authLightColors.fill,
    navActive: authLightColors.pressed,
    border: authLightColors.border,
    borderSoft: authLightColors.border,
    controlBorder: authLightColors.border,
    textMuted: authLightColors.muted,
    textTertiary: authLightColors.muted,
  };
}

export function authInputChrome(
  mode: string,
  focused: boolean,
  invalid: boolean,
) {
  return mode === "light"
    ? {
        borderWidth: 1,
        borderRadius: 14,
        ...(!invalid && focused ? { borderColor: authLightColors.focus } : {}),
      }
    : undefined;
}
