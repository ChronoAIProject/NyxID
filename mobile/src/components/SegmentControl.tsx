import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  Animated,
  LayoutChangeEvent,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import type { StyleProp, ViewStyle } from "react-native";
import { radius, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { createSegmentControlStyles } from "./segmentControlStyles";
import { DynamicTypeText as Text } from "./DynamicTypeText";

type Segment = {
  /** Stable identity when the visible label includes a changing count. */
  id?: string;
  label: string;
  count?: number;
  accessibilityLabel?: string;
  icon?: ReactNode;
};

export type SegmentControlProps = {
  segments: Segment[];
  activeIndex: number;
  disabled?: boolean;
  accessibilityLabel?: string;
  /** External spacing only; grouped-choice visuals belong to this component. */
  style?: StyleProp<ViewStyle>;
  stacked?: boolean;
  onPress: (index: number) => void;
};

export function SegmentControl({
  segments,
  activeIndex,
  disabled = false,
  accessibilityLabel,
  style,
  stacked = false,
  onPress,
}: SegmentControlProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const translateX = useRef(new Animated.Value(0)).current;
  const [containerWidth, setContainerWidth] = useState(0);
  const trackInset = styles.container.padding + styles.container.borderWidth;
  // The highlight shares each tab's measured content width, excluding track
  // borders, padding and the gaps between tabs.
  const segmentWidth =
    !stacked && segments.length > 0
      ? Math.max(
          0,
          (containerWidth -
            2 * trackInset -
            (segments.length - 1) * styles.container.gap) /
            segments.length,
        )
      : 0;

  useEffect(() => {
    if (stacked) return;
    const animation = Animated.spring(translateX, {
      toValue: activeIndex * (segmentWidth + styles.container.gap),
      useNativeDriver: true,
      tension: 300,
      friction: 30,
      overshootClamping: true,
    });
    animation.start();
    return () => animation.stop();
  }, [activeIndex, segmentWidth, stacked, styles.container.gap, translateX]);

  const onContainerLayout = (e: LayoutChangeEvent) => {
    setContainerWidth(e.nativeEvent.layout.width);
  };

  return (
    <View
      accessibilityRole="tablist"
      accessibilityLabel={accessibilityLabel}
      style={[styles.container, stacked && styles.containerStacked, style]}
      onLayout={onContainerLayout}
    >
      {segmentWidth > 0 ? (
        <Animated.View
          pointerEvents="none"
          style={[
            styles.highlight,
            styles.selected,
            {
              width: segmentWidth,
              transform: [{ translateX }],
            },
          ]}
        />
      ) : null}
      {segments.map((seg, i) => {
        const isActive = i === activeIndex;
        return (
          <Pressable
            key={seg.id ?? seg.label}
            accessibilityRole="tab"
            accessibilityLabel={seg.accessibilityLabel ?? seg.label}
            accessibilityState={{ selected: isActive, disabled }}
            disabled={disabled}
            style={({ pressed }) => [
              styles.item,
              stacked && styles.itemStacked,
              stacked && isActive && styles.selected,
              pressed && !disabled && styles.pressed,
            ]}
            onPress={() => onPress(i)}
          >
            {seg.icon}
            <Text style={[styles.label, isActive && styles.labelActive]}>
              {seg.label}
            </Text>
            {seg.count !== undefined && seg.count > 0 ? (
              <View
                style={[styles.countBadge, isActive && styles.countBadgeActive]}
              >
                <Text
                  style={[styles.countText, isActive && styles.countTextActive]}
                >
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

const createStyles = (c: ThemeColors) => {
  const shared = createSegmentControlStyles(c);
  return StyleSheet.create({
    ...shared,
    container: {
      ...shared.container,
      marginBottom: spacing.xxl,
      position: "relative",
    },
    containerStacked: {
      flexDirection: "column",
    },
    highlight: {
      position: "absolute",
      top: shared.container.padding,
      left: shared.container.padding,
      bottom: shared.container.padding,
      borderRadius: shared.item.borderRadius,
      borderWidth: shared.item.borderWidth,
    },
    item: {
      ...shared.item,
      flexDirection: "row",
      gap: spacing.xs,
      zIndex: 1,
    },
    itemStacked: {
      flex: undefined,
    },
    label: {
      ...typeScale.label,
      color: c.textMuted,
      minWidth: 0,
      flexShrink: 1,
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
};
