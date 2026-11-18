import type { ReactNode } from "react";
import { View, useWindowDimensions } from "react-native";

/**
 * Holds a leading icon in a box exactly one text line tall, so in a row with
 * `alignItems: "flex-start"` the icon centres on the first line however many
 * lines the text wraps to, and keeps doing so at larger Dynamic Type sizes.
 */
export function FirstLineIcon({
  lineHeight,
  maxFontScale,
  children,
}: {
  /** The neighbouring text's line height at the default text size. */
  lineHeight: number;
  /** The text's `maxFontSizeMultiplier`, when it has one. */
  maxFontScale?: number;
  children: ReactNode;
}) {
  const { fontScale } = useWindowDimensions();
  const scale = maxFontScale ? Math.min(fontScale, maxFontScale) : fontScale;
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={{ height: Math.round(lineHeight * scale), justifyContent: "center" }}
    >
      {children}
    </View>
  );
}
