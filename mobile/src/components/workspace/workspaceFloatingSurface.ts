import type { ViewStyle } from "react-native";
import type { ThemeColors } from "../../theme/mobileTheme";
import { FLOATING_CONTROL_SIZE } from "../floatingControlTokens";

// Pure styles: no runtime react-native import, so screens' tests can load
// this module beside their mocked react-native. Callers pass
// StyleSheet.hairlineWidth.

/** The composer's soft lift. Dark surfaces stand out by fill and border alone. */
export const FLOATING_SURFACE_SHADOW = {
  shadowOffset: { width: 0, height: 2 },
  shadowOpacity: 1,
  shadowRadius: 10,
  elevation: 3,
} as const;

/** A 44pt floating glass control (header buttons, the quick rail). */
export const FLOATING_BUTTON_SIZE = FLOATING_CONTROL_SIZE;

/** The composer-like floating surface (the composer's permission pill). */
export function floatingSurface(
  colors: Pick<ThemeColors, "composerBg" | "composerBorder" | "shadowColor">,
  mode: "light" | "dark",
  hairlineWidth: number,
): ViewStyle {
  return {
    backgroundColor: colors.composerBg,
    borderColor: colors.composerBorder,
    borderWidth: hairlineWidth,
    shadowColor: colors.shadowColor,
    ...(mode === "light" ? FLOATING_SURFACE_SHADOW : null),
  };
}

/**
 * The one surface of every floating control (header buttons, the identity
 * pill, the folded Duo's rail capsules): the theme's floating-control fill
 * and hairline edge, with the composer's soft lift in light. Opaque theme
 * colours, never a system-chosen glass tint, so the header and the rail
 * match exactly in both themes.
 */
export function floatingControlSurface(
  colors: Pick<
    ThemeColors,
    "floatingControlBg" | "floatingControlBorder" | "shadowColor"
  >,
  mode: "light" | "dark",
  hairlineWidth: number,
): ViewStyle {
  return {
    backgroundColor: colors.floatingControlBg,
    borderColor: colors.floatingControlBorder,
    borderWidth: hairlineWidth,
    shadowColor: colors.shadowColor,
    ...(mode === "light" ? FLOATING_SURFACE_SHADOW : null),
  };
}

/**
 * Which glass a floating capsule uses: `standard` everywhere (style 'regular',
 * 60% tint); the Duo rail's capsules use `rail` (style 'clear', 25%) and, in
 * full screen over content, `railOpaque` (style 'regular', 90%).
 */
export type FloatingGlass = "standard" | "rail" | "railOpaque";

/** The glass tint's alpha (60%): the token's colour, still clearly glass. */
export const FLOATING_GLASS_TINT_ALPHA = 0.6;
/** The Duo rail's see-through glass (style 'clear'): a hint of the token's
 * colour; the native material, refraction and rim highlights carry it. */
export const FLOATING_RAIL_GLASS_TINT_ALPHA = 0.25;
/** The Duo rail in full screen: its capsules float over content, so their
 * glass is nearly solid (90%) and the page never shows through the icons. */
export const FLOATING_GLASS_OPAQUE_TINT_ALPHA = 0.9;

/**
 * The one tint every floating control's liquid glass carries: the header
 * buttons' surface token (#F9F9F9 light, #353539 dark) at
 * FLOATING_GLASS_TINT_ALPHA, so header and rail glass sample the same colour
 * over the page in both themes.
 */
export function floatingControlGlassTint(
  colors: Pick<ThemeColors, "floatingControlBg">,
  glass: FloatingGlass = "standard",
) {
  const alpha = Math.round(
    (glass === "railOpaque"
      ? FLOATING_GLASS_OPAQUE_TINT_ALPHA
      : glass === "rail"
        ? FLOATING_RAIL_GLASS_TINT_ALPHA
        : FLOATING_GLASS_TINT_ALPHA) * 255,
  )
    .toString(16)
    .padStart(2, "0")
    .toUpperCase();
  return `${colors.floatingControlBg}${alpha}`;
}
