import { useMemo } from "react";
import { Pressable, StyleSheet, Text } from "react-native";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { TOUCH_TARGET, radius, spacing, typeScale } from "../theme/designTokens";

type PrimaryButtonProps = {
  label: string;
  onPress: () => void;
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

  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      style={({ pressed }) => [
        styles.base,
        kind === "ghost" && styles.ghost,
        kind === "danger" && styles.danger,
        pressed && !disabled && (kind === "primary" ? styles.basePressed : styles.softPressed),
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
      backgroundColor: c.primary,
      borderRadius: radius.md,
      paddingHorizontal: spacing.xxl,
      minHeight: TOUCH_TARGET,
      alignItems: "center",
      justifyContent: "center",
      borderWidth: 1,
      borderColor: "transparent",
    },
    basePressed: {
      backgroundColor: c.primaryDim,
    },
    softPressed: {
      opacity: 0.7,
    },
    ghost: {
      backgroundColor: c.ghostBg,
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
