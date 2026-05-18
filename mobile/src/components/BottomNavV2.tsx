import { useEffect, useMemo, useRef, useState } from "react";
import { Animated, LayoutChangeEvent, Pressable, StyleSheet, Text, View } from "react-native";
// `lucide-react-native` mirrors the lucide-react icon set used by the web app
// (frontend/src/components/dashboard/sidebar.tsx). One vocabulary across both
// platforms: `User` for the account icon, `ShieldCheck` for the approvals icon.
import { ShieldCheck, User } from "lucide-react-native";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";

export type BottomNavV2Tab = "activity" | "account";

type BottomNavV2Props = {
  active: BottomNavV2Tab;
  onTabPress: (tab: BottomNavV2Tab) => void;
  onFabPress?: () => void;
};

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
        <ShieldCheck size={18} color={active === "activity" ? colors.textPrimary : colors.textMuted} strokeWidth={2} />
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
        <User size={18} color={active === "account" ? colors.textPrimary : colors.textMuted} strokeWidth={2} />
        <Text style={[styles.text, active === "account" && styles.textActive]}>Account</Text>
      </Pressable>
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    wrap: {
      backgroundColor: c.card,
      // DESIGN.md §Border Radius: cards = rounded-xl (12px). 50%-opacity border on chrome.
      borderRadius: radius.lg,
      borderWidth: 1,
      borderColor: c.borderSoft,
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
      // Nav items = rounded-lg (8px) per DESIGN.md.
      borderRadius: radius.md,
      // DESIGN.md: active nav item bg-white/[0.06] — already encoded in navActive token.
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
    // DESIGN.md §Interaction Rules: status labels are Title Case, not UPPERCASE.
    // Mobile nav labels follow the same case discipline as the rest of the app.
    text: {
      ...typeScale.small,
      color: c.textMuted,
      letterSpacing: 0,
      textTransform: "none",
    },
    textActive: {
      color: c.textPrimary,
    },
    fabLabel: {
      ...typeScale.small,
      color: c.primary,
      letterSpacing: 0,
      textTransform: "none",
    },
  });
