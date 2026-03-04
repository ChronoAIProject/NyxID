import { Pressable, StyleSheet, Text } from "react-native";
import { mobileTheme } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";

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
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      style={[
        styles.base,
        kind === "ghost" && styles.ghost,
        kind === "danger" && styles.danger,
        disabled && styles.disabled,
      ]}
    >
      <Text style={[styles.label, disabled && styles.labelDisabled]}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  base: {
    backgroundColor: mobileTheme.primary,
    borderRadius: radius.md,
    paddingVertical: spacing.lg,
    paddingHorizontal: spacing.xxl,
    alignItems: "center",
    borderWidth: 1,
    borderColor: "transparent",
  },
  ghost: {
    backgroundColor: "#8B5CF610",
    borderColor: mobileTheme.borderSoft,
  },
  danger: {
    backgroundColor: "#2A1217",
    borderColor: "#7F1D1D",
  },
  disabled: {
    opacity: 0.6,
  },
  label: {
    color: "#F8F7FF",
    ...typeScale.bodyStrong,
  },
  labelDisabled: {
    color: "#CFCBE0",
  },
});
