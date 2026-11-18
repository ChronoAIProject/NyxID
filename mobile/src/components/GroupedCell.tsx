import type { ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import { radius, spacing } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";

// Rows lead with a 30pt service icon, so separators start past it as in Settings.
const ICON_INSET = spacing.xxl + 30 + spacing.lg;

/**
 * One row of an iOS inset grouped list rendered by a virtualized list: the
 * first and last rows round the group's corners and later rows draw the
 * hairline above themselves, so cells from `renderItem` read as one card.
 */
export function GroupedCell({
  first = false,
  last = false,
  inset = true,
  children,
}: {
  first?: boolean;
  last?: boolean;
  /** Separators start after the leading icon. */
  inset?: boolean;
  children: ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <View
      style={[
        styles.cell,
        { backgroundColor: colors.groupedFill },
        first && styles.first,
        last && styles.last,
      ]}
    >
      {first ? null : (
        <View
          style={[
            styles.separator,
            {
              backgroundColor: colors.borderSoft,
              marginLeft: inset ? ICON_INSET : spacing.xxl,
            },
          ]}
        />
      )}
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  cell: { overflow: "hidden" },
  first: { borderTopLeftRadius: radius.lg, borderTopRightRadius: radius.lg },
  last: {
    borderBottomLeftRadius: radius.lg,
    borderBottomRightRadius: radius.lg,
  },
  separator: { height: StyleSheet.hairlineWidth },
});
