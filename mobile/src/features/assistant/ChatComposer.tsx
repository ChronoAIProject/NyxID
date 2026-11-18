import { useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Animated,
  Pressable,
  StyleSheet,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { ArrowUp, AudioLines, Plus, Square } from "lucide-react-native";
import { useTheme } from "../../theme/ThemeContext";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { FLOATING_SURFACE_SHADOW } from "../../components/workspace/workspaceFloatingSurface";
import { chatComposerFocus } from "./chatComposerFocus";

/**
 * One filled pill shared by personal, contextual and group chats. Touching it gives a small ChatGPT-like bounce: a
 * quick dip, then a springy settle. Reduce Motion keeps it still, and the
 * voice composer opts out so holding to talk does not bounce.
 */
export function ChatComposerFrame({
  children,
  style,
  bounce = true,
}: {
  children: ReactNode;
  style?: StyleProp<ViewStyle>;
  bounce?: boolean;
}) {
  const { colors, mode } = useTheme();
  const reducedMotion = useReducedMotion();
  const [scale] = useState(() => new Animated.Value(1));
  const press = () => {
    // Touches on the composer (input, toolbar, mention list) never blur it.
    chatComposerFocus.touchStartInComposer();
    if (!bounce || reducedMotion) return;
    scale.stopAnimation();
    Animated.sequence([
      Animated.timing(scale, {
        toValue: 0.985,
        duration: 70,
        useNativeDriver: true,
      }),
      Animated.spring(scale, {
        toValue: 1,
        speed: 22,
        bounciness: 12,
        useNativeDriver: true,
      }),
    ]).start();
  };
  return (
    <Animated.View
      onTouchStart={press}
      style={[
        { transform: [{ scale }] },
        styles.frame,
        mode === "light" && styles.lightLift,
        {
          backgroundColor: colors.composerBg,
          borderColor: colors.composerBorder,
          shadowColor: colors.shadowColor,
        },
        style,
      ]}
    >
      {children}
    </Animated.View>
  );
}

/**
 * Controls sit below the full-width text area, with Send/Stop at the trailing
 * edge. `overlay` lays the same row over an idle input's single line instead;
 * its empty middle passes touches through to the input beneath.
 */
export function ChatComposerRow({
  children,
  overlay = false,
}: {
  children: ReactNode;
  overlay?: boolean;
}) {
  return (
    <View
      testID="chat-composer-toolbar"
      pointerEvents={overlay ? "box-none" : "auto"}
      style={[styles.row, overlay && styles.overlayRow]}
    >
      {children}
    </View>
  );
}

export function ComposerAttachButton({
  label,
  disabled,
  busy,
  onPress,
}: {
  label: string;
  disabled?: boolean;
  busy?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled, busy: !!busy }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.target,
        { opacity: disabled ? 0.4 : pressed ? 0.65 : 1 },
      ]}
    >
      {busy ? (
        <ActivityIndicator size="small" color={colors.textSecondary} />
      ) : (
        <Plus size={22} color={colors.textPrimary} />
      )}
    </Pressable>
  );
}

/** Voice, Send and Stop share one place so the composer never shifts. */
export function ComposerSubmitButton({
  kind,
  label,
  disabled,
  busy,
  onPress,
}: {
  kind: "send" | "stop" | "voice";
  label: string;
  disabled?: boolean;
  busy?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled, busy: !!busy }}
      disabled={disabled}
      onPress={onPress}
      style={styles.target}
    >
      {({ pressed }) => (
        <View
          style={[
            styles.circle,
            {
              backgroundColor: colors.textPrimary,
              opacity: disabled ? 0.28 : pressed ? 0.7 : 1,
            },
          ]}
        >
          {busy ? (
            <ActivityIndicator size="small" color={colors.bg} />
          ) : kind === "stop" ? (
            <Square size={12} color={colors.bg} fill={colors.bg} />
          ) : kind === "voice" ? (
            <AudioLines size={18} color={colors.bg} strokeWidth={2.5} />
          ) : (
            <ArrowUp size={18} color={colors.bg} strokeWidth={2.5} />
          )}
        </View>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  frame: {
    borderRadius: 26,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 4,
    paddingVertical: 4,
    gap: 6,
  },
  // Shared with the floating header controls and the quick rail.
  lightLift: FLOATING_SURFACE_SHADOW,
  row: { flexDirection: "row", alignItems: "flex-end", minHeight: 44 },
  overlayRow: {
    position: "absolute",
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
    alignItems: "center",
  },
  target: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  circle: {
    width: 34,
    height: 34,
    borderRadius: 17,
    alignItems: "center",
    justifyContent: "center",
  },
});
