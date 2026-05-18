import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { TOUCH_TARGET, radius, spacing, typeScale } from "../theme/designTokens";

type PrimaryButtonProps = {
  label: string;
  onPress: () => void;
  /**
   * - `primary`: `nyx-gradient-vivid` gradient — the dominant CTA per DESIGN.md.
   * - `ghost`: bordered ghost button for secondary actions.
   * - `danger`: destructive action (soft red fill + red border).
   */
  kind?: "primary" | "ghost" | "danger";
  disabled?: boolean;
};

export function PrimaryButton({
  label,
  onPress,
  kind = "primary",
  disabled = false,
}: PrimaryButtonProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  if (kind === "primary") {
    return (
      <Pressable
        onPress={onPress}
        disabled={disabled}
        style={({ pressed }) => [
          styles.gradientWrap,
          disabled && styles.disabled,
          pressed && !disabled && styles.primaryPressed,
        ]}
      >
        {({ pressed }) => (
          <LinearGradient
            colors={
              pressed && !disabled
                ? [colors.gradientStartPressed, colors.gradientEndPressed]
                : [colors.gradientStart, colors.gradientEnd]
            }
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 0 }}
            style={styles.gradientInner}
          >
            <Text style={[styles.label, disabled && styles.labelDisabled]}>
              {label}
            </Text>
          </LinearGradient>
        )}
      </Pressable>
    );
  }

  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      style={({ pressed }) => [
        styles.base,
        kind === "ghost" && styles.ghost,
        kind === "danger" && styles.danger,
        pressed && !disabled && styles.softPressed,
        disabled && styles.disabled,
      ]}
    >
      <Text
        style={[
          styles.label,
          kind === "ghost" && styles.ghostLabel,
          kind === "danger" && styles.dangerLabel,
          disabled && styles.labelDisabled,
        ]}
      >
        {label}
      </Text>
    </Pressable>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    base: {
      borderRadius: radius.md,
      paddingHorizontal: spacing.xxl,
      minHeight: TOUCH_TARGET,
      alignItems: "center",
      justifyContent: "center",
      borderWidth: 1,
      borderColor: "transparent",
    },
    gradientWrap: {
      borderRadius: radius.md,
      overflow: "hidden",
      // DESIGN.md §Buttons: primary CTA carries a soft purple glow.
      // Matches the web `shadow-[0_0_12px_rgba(90,42,241,0.25)]` recipe.
      shadowColor: c.primary,
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: 0.3,
      shadowRadius: 12,
      elevation: 6,
    },
    gradientInner: {
      minHeight: TOUCH_TARGET,
      paddingHorizontal: spacing.xxl,
      alignItems: "center",
      justifyContent: "center",
    },
    primaryPressed: {
      shadowOpacity: 0.45,
    },
    softPressed: {
      opacity: 0.7,
    },
    ghost: {
      backgroundColor: "rgba(255,255,255,0.03)",
      borderColor: c.border,
    },
    danger: {
      backgroundColor: c.dangerSoftBg,
      borderColor: c.danger,
    },
    disabled: {
      opacity: 0.5,
    },
    label: {
      color: c.onPrimary,
      ...typeScale.label,
    },
    ghostLabel: {
      color: c.textPrimary,
    },
    dangerLabel: {
      color: c.danger,
    },
    labelDisabled: {
      color: c.textMuted,
    },
  });
