import { useEffect, useRef } from "react";
import { Animated, StyleSheet, View } from "react-native";
import type { DimensionValue, StyleProp, ViewStyle } from "react-native";
import { useReducedMotionPreference } from "../lib/accessibility/useReducedMotion";
import { useTheme } from "../theme/ThemeContext";
import { radius, spacing } from "../theme/designTokens";

/**
 * Skeleton loaders stand in for content on its first load, in the shape it
 * will take, so pages never show a bare spinner. Spinners are reserved for a
 * user's own pull to refresh and for in-progress actions (Send, Save).
 * One soft pulse; static under Reduce Motion. VoiceOver hears one "Loading".
 */
function usePulse() {
  const reduced = useReducedMotionPreference();
  const opacity = useRef(new Animated.Value(1)).current;
  useEffect(() => {
    if (reduced !== false) {
      opacity.setValue(0.75);
      return;
    }
    const loop = Animated.loop(
      Animated.sequence([
        Animated.timing(opacity, {
          toValue: 0.45,
          duration: 700,
          useNativeDriver: true,
        }),
        Animated.timing(opacity, {
          toValue: 1,
          duration: 700,
          useNativeDriver: true,
        }),
      ]),
    );
    loop.start();
    return () => loop.stop();
  }, [reduced, opacity]);
  return opacity;
}

function Bar({
  width,
  height = 12,
  round = false,
}: {
  width: DimensionValue;
  height?: number;
  round?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <View
      style={{
        width,
        height,
        borderRadius: round ? height / 2 : 6,
        backgroundColor: colors.skeleton,
      }}
    />
  );
}

// Varied widths so stacked rows read as text, not as a striped block.
const WIDTHS: DimensionValue[] = ["62%", "48%", "70%", "54%", "66%"];

/** A grouped list's rows on first load: leading glyph, title and detail. */
export function SkeletonRows({
  rows = 3,
  icon = true,
  detail = true,
  title = false,
  accessibilityLabel = "Loading",
  style,
}: {
  rows?: number;
  icon?: boolean;
  detail?: boolean;
  /** Reserve a section title above the group, as the loaded page will have. */
  title?: boolean;
  accessibilityLabel?: string;
  style?: StyleProp<ViewStyle>;
}) {
  const { colors } = useTheme();
  const opacity = usePulse();
  return (
    <Animated.View
      accessible
      accessibilityRole="progressbar"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ busy: true }}
      style={[styles.group, { opacity }, style]}
    >
      {title ? (
        <View style={styles.title}>
          <Bar width={96} height={10} />
        </View>
      ) : null}
      <View style={[styles.card, { backgroundColor: colors.groupedFill }]}>
        {Array.from({ length: rows }, (_, index) => (
          <View key={index}>
            {index > 0 ? (
              <View
                style={[
                  styles.separator,
                  {
                    backgroundColor: colors.borderSoft,
                    marginLeft: icon ? spacing.xxl + 30 + spacing.lg : spacing.xxl,
                  },
                ]}
              />
            ) : null}
            <View style={styles.row}>
              {icon ? <Bar width={30} height={30} round /> : null}
              <View style={styles.lines}>
                <Bar width={WIDTHS[index % WIDTHS.length]!} height={13} />
                {detail ? (
                  <Bar
                    width={WIDTHS[(index + 2) % WIDTHS.length]!}
                    height={10}
                  />
                ) : null}
              </View>
            </View>
          </View>
        ))}
      </View>
    </Animated.View>
  );
}

/** Card lists (Automations) on first load. */
export function SkeletonCards({
  count = 3,
  accessibilityLabel = "Loading",
}: {
  count?: number;
  accessibilityLabel?: string;
}) {
  const { colors } = useTheme();
  const opacity = usePulse();
  return (
    <Animated.View
      accessible
      accessibilityRole="progressbar"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ busy: true }}
      style={[styles.cards, { opacity }]}
    >
      {Array.from({ length: count }, (_, index) => (
        <View
          key={index}
          style={[styles.cardItem, { backgroundColor: colors.groupedFill }]}
        >
          <View style={styles.cardHead}>
            <Bar width={28} height={28} round />
            <View style={styles.lines}>
              <Bar width={WIDTHS[index % WIDTHS.length]!} height={13} />
              <Bar width="38%" height={10} />
            </View>
            <Bar width={58} height={20} round />
          </View>
          <View style={[styles.rule, { backgroundColor: colors.borderSoft }]} />
          <Bar width="56%" height={10} />
          <Bar width="44%" height={10} />
        </View>
      ))}
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  group: { gap: spacing.sm },
  title: { paddingHorizontal: spacing.xxl, paddingVertical: 2 },
  card: { borderRadius: radius.lg, overflow: "hidden" },
  separator: { height: StyleSheet.hairlineWidth },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.lg,
    minHeight: 56,
    paddingHorizontal: spacing.xxl,
    paddingVertical: spacing.md,
  },
  lines: { flex: 1, gap: spacing.sm },
  cards: { gap: spacing.md },
  cardItem: {
    borderRadius: radius.lg,
    padding: spacing.lg + 2,
    gap: spacing.md,
  },
  cardHead: { flexDirection: "row", alignItems: "center", gap: spacing.md },
  rule: { height: StyleSheet.hairlineWidth },
});
