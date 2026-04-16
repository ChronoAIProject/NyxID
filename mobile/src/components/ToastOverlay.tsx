import { useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated, { FadeIn, FadeOut, LinearTransition } from "react-native-reanimated";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { radius } from "../theme/designTokens";

export type ToastKind = "success" | "error" | "info";

export type ToastState = {
  message: string;
  kind: ToastKind;
  action?: { label: string; onPress: () => void };
};

type ToastEntry = ToastState & { id: number };

type Props = {
  toast: ToastState | null;
  bottom?: number; // kept for backward compat, ignored
  duration?: number;
};

const MAX_VISIBLE = 3;

function getAccent(c: ThemeColors): Record<ToastKind, string> {
  return { success: c.success, error: c.danger, info: c.info };
}

function getAccentBorder(c: ThemeColors): Record<ToastKind, string> {
  return {
    success: c.successSoft,
    error: c.dangerSoftBg,
    info: c.infoSoft,
  };
}

let nextId = 0;

export function ToastOverlay({ toast, duration = 2400 }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const [queue, setQueue] = useState<ToastEntry[]>([]);
  const prevToastRef = useRef<ToastState | null>(null);

  useEffect(() => {
    if (toast && toast !== prevToastRef.current) {
      const id = ++nextId;
      setQueue((q) => [{ ...toast, id }, ...q].slice(0, MAX_VISIBLE));
      setTimeout(() => {
        setQueue((q) => q.filter((t) => t.id !== id));
      }, duration);
    }
    prevToastRef.current = toast;
  }, [toast, duration]);

  if (queue.length === 0) return null;

  return (
    <View style={styles.wrap} pointerEvents="box-none">
      {queue.map((entry) => (
        <Animated.View
          key={entry.id}
          entering={FadeIn.duration(200)}
          exiting={FadeOut.duration(200)}
          layout={LinearTransition.duration(200)}
          style={[styles.toast, { borderColor: getAccentBorder(colors)[entry.kind] }]}
        >
          <View style={[styles.dot, { backgroundColor: getAccent(colors)[entry.kind] }]} />
          <Text style={styles.text} numberOfLines={1}>
            {entry.message}
          </Text>
          {entry.action && (
            <>
              <View style={styles.divider} />
              <Pressable onPress={entry.action.onPress} hitSlop={8}>
                <Text style={styles.actionText}>{entry.action.label}</Text>
              </Pressable>
            </>
          )}
        </Animated.View>
      ))}
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    wrap: {
      position: "absolute",
      top: 0,
      left: 0,
      right: 0,
      alignItems: "center",
      zIndex: 10000,
      gap: 6,
    },
    toast: {
      flexDirection: "row",
      alignItems: "center",
      gap: 10,
      borderRadius: radius.pill,
      backgroundColor: c.card,
      borderWidth: 1,
      borderColor: "rgba(255,255,255,0.08)",
      paddingHorizontal: 16,
      paddingVertical: 12,
      maxWidth: "85%",
      shadowColor: c.shadowColor,
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: 0.3,
      shadowRadius: 12,
      elevation: 8,
    },
    dot: {
      width: 10,
      height: 10,
      borderRadius: 5,
    },
    text: {
      flex: 1,
      fontSize: 14,
      fontWeight: "500",
      lineHeight: 18,
      color: c.textPrimary,
    },
    divider: {
      width: 1,
      height: 16,
      backgroundColor: c.borderSoft,
    },
    actionText: {
      fontSize: 14,
      fontWeight: "500",
      color: c.textPrimary,
    },
  });
