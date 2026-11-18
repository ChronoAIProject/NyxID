import type { ViewStyle } from "react-native";
import type { ThemeColors } from "../theme/mobileTheme";

/**
 * The iOS segmented control: a borderless translucent track with the selected
 * segment raised on it. The track tint reads on white pages and grey sheets.
 */
export function createSegmentControlStyles(colors: ThemeColors) {
  return {
    container: {
      flexDirection: "row",
      padding: 2,
      gap: 2,
      borderWidth: 0,
      borderRadius: 10,
      backgroundColor: colors.segmentTrack,
    },
    item: {
      flex: 1,
      minWidth: 0,
      minHeight: 40,
      paddingHorizontal: 8,
      paddingVertical: 6,
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 8,
      borderWidth: 0,
    },
    selected: {
      backgroundColor: colors.segmentSelected,
      shadowColor: "#000",
      shadowOpacity: 0.12,
      shadowRadius: 4,
      shadowOffset: { width: 0, height: 2 },
      elevation: 2,
    },
    pressed: {
      opacity: 0.6,
    },
  } satisfies Record<string, ViewStyle>;
}
