import { useEffect, useMemo, useRef, useState } from "react";
import { Animated, LayoutChangeEvent, Pressable, StyleSheet, Text, View } from "react-native";
import Svg, { Path, Circle } from "react-native-svg";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";

export type BottomNavV2Tab = "activity" | "account";

type BottomNavV2Props = {
  active: BottomNavV2Tab;
  onTabPress: (tab: BottomNavV2Tab) => void;
  onFabPress?: () => void;
};

function ShieldCheckIcon({ color }: { color: string }) {
  return (
    <Svg width={18} height={18} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <Path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      <Path d="M9 12l2 2 4-4" />
    </Svg>
  );
}

function PersonIcon({ color }: { color: string }) {
  return (
    <Svg width={18} height={18} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <Path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
      <Circle cx={12} cy={7} r={4} />
    </Svg>
  );
}

const PADDING = spacing.xs + spacing.xxs; // 6
const GAP = spacing.xs + spacing.xxs; // 6
const FAB_WIDTH = 57;

export function BottomNavV2({ active, onTabPress, onFabPress }: BottomNavV2Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const translateX = useRef(new Animated.Value(0)).current;
  const [tabWidth, setTabWidth] = useState(0);
  const containerWidth = useRef(0);

  const activeIndex = active === "activity" ? 0 : 1;

  const showFab = Boolean(onFabPress);

  const computeOffset = (index: number) => {
    if (index === 0) return 0;
    if (showFab) return tabWidth + GAP + FAB_WIDTH + GAP;
    return tabWidth + GAP;
  };

  useEffect(() => {
    Animated.spring(translateX, {
      toValue: computeOffset(activeIndex),
      useNativeDriver: true,
      tension: 300,
      friction: 30,
    }).start();
  }, [activeIndex, tabWidth, translateX]);

  const onLayout = (e: LayoutChangeEvent) => {
    const w = e.nativeEvent.layout.width;
    containerWidth.current = w;
    const inner = w - 2 * PADDING;
    const newTabWidth = showFab
      ? (inner - FAB_WIDTH - 2 * GAP) / 2
      : (inner - GAP) / 2;
    setTabWidth(newTabWidth);
    // Use newTabWidth directly since setState is async
    const offset = activeIndex === 0 ? 0
      : showFab ? newTabWidth + GAP + FAB_WIDTH + GAP
      : newTabWidth + GAP;
    translateX.setValue(offset);
  };

  return (
    <View style={styles.wrap} onLayout={onLayout}>
      <Animated.View
        style={[
          styles.activeHighlight,
          {
            width: tabWidth || "35%",
            transform: [{ translateX }],
          },
        ]}
      />
      <Pressable
        style={styles.item}
        onPress={() => onTabPress("activity")}
      >
        <ShieldCheckIcon color={active === "activity" ? colors.textPrimary : colors.textMuted} />
        <Text style={[styles.text, active === "activity" && styles.textActive]}>Activity</Text>
      </Pressable>

      {showFab && (
        <Pressable style={styles.fabSpacer} onPress={onFabPress}>
          <Text style={styles.fabLabel}>Ask Nyx</Text>
        </Pressable>
      )}

      <Pressable
        style={styles.item}
        onPress={() => onTabPress("account")}
      >
        <PersonIcon color={active === "account" ? colors.textPrimary : colors.textMuted} />
        <Text style={[styles.text, active === "account" && styles.textActive]}>Account</Text>
      </Pressable>
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    wrap: {
      backgroundColor: c.card,
      borderRadius: radius.xl,
      borderWidth: 1,
      borderColor: c.border,
      padding: PADDING,
      flexDirection: "row",
      gap: GAP,
      position: "relative",
    },
    activeHighlight: {
      position: "absolute",
      top: PADDING,
      left: PADDING,
      bottom: PADDING,
      borderRadius: radius.md,
      backgroundColor: c.navActive,
    },
    item: {
      flex: 1,
      paddingVertical: spacing.sm,
      borderRadius: radius.md,
      alignItems: "center",
      justifyContent: "center",
      backgroundColor: "transparent",
      gap: 3,
      zIndex: 1,
    },
    fabSpacer: {
      width: FAB_WIDTH,
      alignItems: "center",
      justifyContent: "flex-end",
      paddingBottom: 2,
      zIndex: 1,
    },
    text: {
      ...typeScale.overline,
      color: c.textMuted,
      letterSpacing: 0.6,
    },
    textActive: {
      color: c.textPrimary,
    },
    fabLabel: {
      ...typeScale.overline,
      color: c.primary,
      letterSpacing: 0.6,
    },
  });
