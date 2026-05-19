import { useEffect, useMemo, useRef } from "react";
import { Animated, LayoutChangeEvent, Pressable, StyleSheet, Text, View } from "react-native";
import { TOUCH_TARGET, radius, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";

type Segment = {
  label: string;
  count?: number;
};

type SegmentControlProps = {
  segments: Segment[];
  activeIndex: number;
  onPress: (index: number) => void;
};

export function SegmentControl({ segments, activeIndex, onPress }: SegmentControlProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const translateX = useRef(new Animated.Value(0)).current;
  const segmentWidth = useRef(0);

  useEffect(() => {
    Animated.spring(translateX, {
      toValue: activeIndex * segmentWidth.current,
      useNativeDriver: true,
      tension: 300,
      friction: 30,
    }).start();
  }, [activeIndex, translateX]);

  const onContainerLayout = (e: LayoutChangeEvent) => {
    const totalWidth = e.nativeEvent.layout.width - 4; // subtract container padding (2+2)
    segmentWidth.current = totalWidth / segments.length;
    translateX.setValue(activeIndex * segmentWidth.current);
  };

  return (
    <View style={styles.container} onLayout={onContainerLayout}>
      <Animated.View
        style={[
          styles.highlight,
          {
            width: `${100 / segments.length}%` as unknown as number,
            transform: [{ translateX }],
          },
        ]}
      />
      {segments.map((seg, i) => {
        const isActive = i === activeIndex;
        return (
          <Pressable
            key={seg.label}
            style={styles.item}
            onPress={() => onPress(i)}
          >
            <Text style={[styles.label, isActive && styles.labelActive]}>{seg.label}</Text>
            {seg.count !== undefined && seg.count > 0 ? (
              <View style={[styles.countBadge, isActive && styles.countBadgeActive]}>
                <Text style={[styles.countText, isActive && styles.countTextActive]}>
                  {seg.count}
                </Text>
              </View>
            ) : null}
          </Pressable>
        );
      })}
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    container: {
      flexDirection: "row",
      // DESIGN.md: chrome uses 50%-opacity borders.
      backgroundColor: c.cardSoft,
      borderWidth: 1,
      borderColor: c.borderSoft,
      borderRadius: radius.md,
      padding: 2,
      marginBottom: spacing.xxl,
      position: "relative",
    },
    // DESIGN.md §Tabs: active state is subtle, not a filled pill. Mobile uses
    // the same `navActive` token as BottomNavV2 so segment controls and bottom
    // nav share one visual language for "selected".
    highlight: {
      position: "absolute",
      top: 2,
      left: 2,
      bottom: 2,
      borderRadius: radius.xs + 2, // 6 — visually nested inside the 8px container
      backgroundColor: c.navActive,
    },
    item: {
      flex: 1,
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "center",
      gap: spacing.xs,
      minHeight: TOUCH_TARGET,
      paddingHorizontal: spacing.xs,
      borderRadius: radius.sm,
      zIndex: 1,
    },
    label: {
      ...typeScale.label,
      color: c.textMuted,
    },
    // Active label color matches BottomNavV2 — the textPrimary swap signals selection.
    labelActive: {
      color: c.textPrimary,
    },
    countBadge: {
      minWidth: 18,
      height: 18,
      // DESIGN.md §Badges: rounded-md (6px), purple accent variant. Theme-aware
      // via `primaryTone`, so both light + dark render correctly.
      borderRadius: radius.sm,
      backgroundColor: c.primaryTone.bg,
      borderWidth: 1,
      borderColor: c.primaryTone.border,
      alignItems: "center",
      justifyContent: "center",
      paddingHorizontal: spacing.xs,
    },
    countBadgeActive: {
      // Same chip shape; same color family. The active-segment surface change
      // already signals selection — no need to escalate the count badge fill.
      backgroundColor: c.primaryTone.bg,
      borderColor: c.primaryTone.border,
    },
    countText: {
      ...typeScale.badge,
      color: c.primaryOnTint,
    },
    countTextActive: {
      color: c.primaryOnTint,
    },
  });
