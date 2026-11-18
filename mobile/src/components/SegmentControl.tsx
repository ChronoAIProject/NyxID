import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  Animated,
  LayoutChangeEvent,
  Pressable,
  ScrollView,
  StyleSheet,
  View,
} from "react-native";
import type { StyleProp, ViewStyle } from "react-native";
import { fonts, radius, spacing, typeScale } from "../theme/designTokens";
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

/** Segment widths that never wrap or shrink a label. Equal widths while every
 * label fits (the iOS look); otherwise each segment keeps its single-line
 * width plus an equal share of any spare space, and a row wider than the
 * track scrolls. Exported for tests. */
export function segmentWidths(
  inner: number,
  gap: number,
  natural: readonly (number | undefined)[],
  count: number,
) {
  const equal = Math.max(0, (inner - (count - 1) * gap) / count);
  const measured = natural.slice(0, count);
  if (
    measured.length < count ||
    measured.some((width) => !width || width <= 0) ||
    Math.max(...(measured as number[])) <= equal
  )
    return Array.from({ length: count }, () => equal);
  const total = (measured as number[]).reduce((sum, width) => sum + width, 0);
  const slack = Math.max(0, (inner - (count - 1) * gap - total) / count);
  return (measured as number[]).map((width) => width + slack);
}

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
  const scroll = useRef<ScrollView>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const [natural, setNatural] = useState<(number | undefined)[]>([]);
  const trackInset = styles.container.padding + styles.container.borderWidth;
  const gap = styles.container.gap;
  const inner = Math.max(0, containerWidth - 2 * trackInset);
  const widths =
    !stacked && segments.length > 0 && containerWidth > 0
      ? segmentWidths(inner, gap, natural, segments.length)
      : [];
  const offsets = widths.map((_, i) =>
    widths.slice(0, i).reduce((sum, width) => sum + width + gap, 0),
  );
  const contentWidth = widths.length
    ? offsets[widths.length - 1]! + widths[widths.length - 1]!
    : 0;
  const equal = Math.max(
    0,
    (inner - (segments.length - 1) * gap) / segments.length,
  );
  const adaptive =
    !stacked &&
    natural.length >= segments.length &&
    natural.slice(0, segments.length).every((width) => !!width) &&
    natural.slice(0, segments.length).some((width) => width! > equal);
  const scrollable = adaptive && contentWidth > inner + 0.5;
  const activeOffset = offsets[activeIndex] ?? 0;
  const activeWidth = widths[activeIndex] ?? 0;

  useEffect(() => {
    if (stacked) return;
    const animation = Animated.spring(translateX, {
      toValue: activeOffset,
      useNativeDriver: true,
      tension: 300,
      friction: 30,
      overshootClamping: true,
    });
    animation.start();
    return () => animation.stop();
  }, [activeOffset, stacked, translateX]);

  useEffect(() => {
    // Keep the selected segment in view when the row scrolls.
    if (!scrollable) return;
    const x = Math.max(
      0,
      Math.min(
        contentWidth - inner,
        activeOffset - (inner - activeWidth) / 2
      ),
    );
    scroll.current?.scrollTo({ x, animated: true });
  }, [scrollable, activeOffset, activeWidth, contentWidth, inner]);

  const onContainerLayout = (e: LayoutChangeEvent) => {
    setContainerWidth(e.nativeEvent.layout.width);
  };
  const measure = (index: number, width: number) =>
    setNatural((previous) => {
      if (previous[index] === width) return previous;
      const next = [...previous];
      next[index] = width;
      return next;
    });

  const items = segments.map((seg, i) => {
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
          adaptive && widths.length > 0 && { flex: 0, width: widths[i] },
          stacked && isActive && styles.selected,
          pressed && !disabled && styles.pressed,
        ]}
        onPress={() => onPress(i)}
      >
        {segmentContent(seg, isActive, styles, adaptive)}
      </Pressable>
    );
  });

  return (
    <View
      accessibilityRole="tablist"
      accessibilityLabel={accessibilityLabel}
      style={[
        styles.container,
        stacked && styles.containerStacked,
        style
      ]}
      onLayout={onContainerLayout}
    >
      {stacked ? (
        items
      ) : (
        <>
          {adaptive ? (
          <ScrollView
            ref={scroll}
            horizontal
            scrollEnabled={scrollable}
            showsHorizontalScrollIndicator={false}
            keyboardShouldPersistTaps="handled"
            style={styles.scroll}
            contentContainerStyle={styles.scrollContent}
          >
            {widths.length > 0 ? (
              <Animated.View
                pointerEvents="none"
                style={[
                  styles.highlight,
                    { top: 0, left: 0, bottom: 0 },
                  styles.selected,
                  { width: activeWidth, transform: [{ translateX }] },
                ]}
              />
            ) : null}
            {items}
          </ScrollView>
          ) : (
            <>
              {widths.length > 0 ? (
                <Animated.View
                  pointerEvents="none"
                  style={[
                    styles.highlight,
                    styles.selected,
                    { width: activeWidth, transform: [{ translateX }] },
                  ]}
                />
              ) : null}
              {items}
            </>
          )}
          {/* Off-screen single-line copies give each label's natural width. */}
          <View
            pointerEvents="none"
            accessibilityElementsHidden
            importantForAccessibility="no-hide-descendants"
            style={styles.measureClip}
          >
            <View style={styles.measureRow}>
              {segments.map((seg, i) => (
                <View
                  key={seg.id ?? seg.label}
                  testID={`segment-measure-${i}`}
                  onLayout={(e) => measure(i, e.nativeEvent.layout.width)}
                  style={[styles.item, styles.itemNatural]}
                >
                  {segmentContent(seg, i === activeIndex, styles, false, true)}
                </View>
              ))}
            </View>
          </View>
        </>
      )}
    </View>
  );
}

/** One line at the Dynamic Type size: never wrapped, split or scaled down.
 * A plain render helper (no hooks), so the label stays in the tab's tree. */
function segmentContent(
  seg: Segment,
  active: boolean,
  styles: ReturnType<typeof createStyles>,
  adaptive: boolean,
  measuring = false,
) {
  return (
    <>
      {seg.icon}
      <Text
        numberOfLines={adaptive || measuring ? 1 : undefined}
        style={[styles.label, active && styles.labelActive]}
      >
        {seg.label}
      </Text>
      {seg.count !== undefined && seg.count > 0 ? (
        <View
          style={[
            styles.countBadge,
            adaptive && { height: undefined, minHeight: 18 },
            active && styles.countBadgeActive,
          ]}
        >
          <Text
            numberOfLines={adaptive || measuring ? 1 : undefined}
            style={[styles.countText, active && styles.countTextActive]}
          >
            {seg.count}
          </Text>
        </View>
      ) : null}
    </>
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
    // Inside the scroll row: the track's padding is outside it.
    highlight: {
      position: "absolute",
      top: shared.container.padding,
      left: shared.container.padding,
      bottom: shared.container.padding,
      borderRadius: shared.item.borderRadius,
      borderWidth: shared.item.borderWidth,
    },
    scroll: { flex: 1, flexGrow: 1 },
    scrollContent: { flexGrow: 1, gap: shared.container.gap },
    measureClip: {
      position: "absolute",
      width: 0,
      height: 0,
      overflow: "hidden",
    },
    measureRow: {
      position: "absolute",
      flexDirection: "row",
      alignItems: "flex-start",
      width: 10000,
    },
    itemNatural: { flex: 0, flexGrow: 0, flexShrink: 0 },
    item: {
      ...shared.item,
      flexDirection: "row",
      gap: spacing.xs,
      zIndex: 1,
    },
    itemStacked: {
      flex: undefined,
    },
    // As on iOS both labels are primary text; the raised segment and a
    // heavier weight mark the selection.
    label: {
      ...typeScale.label,
      fontSize: 15,
      lineHeight: 20,
      color: c.textPrimary,
      minWidth: 0,
      flexShrink: 1,
    },
    labelActive: {
      fontFamily: fonts.bodySemi,
      fontWeight: "600",
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
