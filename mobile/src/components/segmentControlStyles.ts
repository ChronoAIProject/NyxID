import type { ViewStyle } from "react-native";
import type { ThemeColors } from "../theme/mobileTheme";

/** Native grouped choices need visible edges even when page and card are white. */
export function createSegmentControlStyles(colors: ThemeColors) {
  return {
    container: {
      flexDirection: "row",
      padding: 3,
      gap: 3,
      borderWidth: 1,
      borderRadius: 10,
      backgroundColor: colors.ghostBg,
      borderColor: colors.border,
    },
    item: {
      flex: 1,
      minWidth: 0,
      minHeight: 44,
      paddingHorizontal: 8,
      paddingVertical: 8,
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 7,
      borderWidth: 1,
      borderColor: "transparent",
    },
    selected: {
      backgroundColor: colors.card,
      borderColor: colors.controlBorder,
    },
    pressed: {
      backgroundColor: colors.navActive,
      borderColor: colors.controlBorder,
    },
  } satisfies Record<string, ViewStyle>;
}
